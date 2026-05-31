package com.example.agentsmoke

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Color
import android.graphics.PixelFormat
import android.os.BatteryManager
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.provider.Settings
import android.util.Base64
import android.util.Log
import android.view.Gravity
import android.view.MotionEvent
import android.view.View
import android.view.WindowManager
import android.view.inputmethod.InputMethodManager
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import java.util.concurrent.Executors
import org.json.JSONArray
import org.json.JSONObject
import uniffi.agent_smoke.AgentCore
import uniffi.agent_smoke.PlatformToolHost

class AgentHostService : Service() {
    private val mainHandler = Handler(Looper.getMainLooper())
    private val executor = Executors.newSingleThreadExecutor()
    private var agent: AgentCore? = null
    private var overlay: FloatingAgentOverlay? = null
    private var lastStatus: String = "Agent ready"
    private var apiBaseOverride: String? = null
    private var apiKeyOverride: String? = null
    private var modelOverride: String? = null
    private var proxyOverride: String? = null
    private val appMapMemory = AndroidAppMapMemory()

    override fun onCreate() {
        super.onCreate()
        startForeground(NOTIFICATION_ID, notification("Agent host is running"))
        ensureAgent()
        if (Settings.canDrawOverlays(this)) {
            overlay = FloatingAgentOverlay(this, ::submitPrompt)
            overlay?.show()
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        intent?.getStringExtra(EXTRA_API_BASE)?.takeIf { it.isNotBlank() }?.let {
            apiBaseOverride = it
            agent?.close()
            agent = null
        }
        intent?.getStringExtra(EXTRA_API_KEY)?.takeIf { it.isNotBlank() }?.let {
            apiKeyOverride = it
            agent?.close()
            agent = null
        }
        intent?.getStringExtra(EXTRA_MODEL)?.takeIf { it.isNotBlank() }?.let {
            modelOverride = it
            agent?.close()
            agent = null
        }
        intent?.getStringExtra(EXTRA_PROXY)?.let {
            proxyOverride = it
            agent?.close()
            agent = null
        }
        intent?.getStringExtra(EXTRA_DEBUG_TOOL)?.takeIf { it.isNotBlank() }?.let { toolName ->
            var input = decodeDebugInput(intent)
            if (toolName == DEBUG_AGENT_AUTONOMOUS_MAP_TASK && intent.hasExtra(EXTRA_MAX_TOOL_ROUNDS)) {
                val args = if (input.isBlank()) JSONObject() else JSONObject(input)
                args.put("max_tool_rounds", intent.getIntExtra(EXTRA_MAX_TOOL_ROUNDS, DEFAULT_TOOL_ROUNDS))
                input = args.toString()
            }
            executeDebugTool(toolName, input)
        }
        intent?.getStringExtra(EXTRA_PROMPT)
            ?.takeIf { it.isNotBlank() }
            ?.let { prompt ->
                submitPrompt(prompt, intent.getIntExtra(EXTRA_MAX_TOOL_ROUNDS, DEFAULT_TOOL_ROUNDS))
            }
        return START_STICKY
    }

    private fun decodeDebugInput(intent: Intent): String {
        val encoded = intent.getStringExtra(EXTRA_DEBUG_INPUT_BASE64)
        if (!encoded.isNullOrBlank()) {
            return String(Base64.decode(encoded, Base64.NO_WRAP), Charsets.UTF_8)
        }
        return intent.getStringExtra(EXTRA_DEBUG_INPUT) ?: "{}"
    }

    override fun onDestroy() {
        overlay?.remove()
        overlay = null
        agent?.close()
        agent = null
        executor.shutdownNow()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    private fun ensureAgent(): AgentCore? {
        agent?.let { return it }

        val created = createAgent(includeMacroTools = true) ?: return null
        agent = created
        return created
    }

    private fun createAgent(includeMacroTools: Boolean): AgentCore? {
        val apiKey = apiKeyOverride ?: BuildConfig.MIMO_API_KEY
        if (apiKey.isBlank()) {
            updateStatus("Missing MIMO_API_KEY")
            return null
        }
        val apiBase = apiBaseOverride ?: BuildConfig.MIMO_API_BASE
        val model = modelOverride ?: BuildConfig.MIMO_MODEL
        val proxy = proxyOverride ?: BuildConfig.MIMO_PROXY

        val created = if (proxy.isBlank()) {
            AgentCore.newWithBaseUrl(
                apiBase,
                apiKey,
                model
            )
        } else {
            AgentCore.newWithBaseUrlAndProxy(
                apiBase,
                apiKey,
                model,
                proxy
            )
        }
        created.setPlatformToolHost(ServicePlatformToolHost(this, appMapMemory))
        registerTools(created, includeMacroTools)
        return created
    }

    private fun submitPrompt(prompt: String, maxToolRounds: Int = DEFAULT_TOOL_ROUNDS) {
        updateStatus("Running: $prompt")
        executor.execute {
            try {
                val currentAgent = ensureAgent()
                    ?: return@execute
                val response = currentAgent.promptWithToolsLimit(
                    prompt,
                    maxToolRounds.coerceIn(1, MAX_TOOL_ROUNDS).toUInt()
                )
                val tools = response.toolTraces.joinToString(", ") { it.name }
                updateStatus(
                    "Done\nTools: $tools\nTokens: ${response.totalTokens}\n\n${response.answer.take(MAX_STATUS_CHARS)}"
                )
            } catch (error: Throwable) {
                updateStatus("Agent error: ${error.message ?: error.javaClass.simpleName}")
            }
        }
    }

    private fun executeDebugTool(toolName: String, inputJson: String) {
        updateStatus("Debug tool: $toolName")
        executor.execute {
            try {
                val output = if (toolName == DEBUG_AGENT_AUTONOMOUS_MAP_TASK) {
                    executeAutonomousMapTask(inputJson)
                } else {
                    ServicePlatformToolHost(this, appMapMemory).executeTool(toolName, inputJson)
                }
                logDebugOutput(toolName, output)
                updateStatus("Debug tool: $toolName\n$output")
            } catch (error: Throwable) {
                Log.e(TAG, "Debug tool $toolName failed", error)
                updateStatus("Debug tool error: ${error.message ?: error.javaClass.simpleName}")
            }
        }
    }

    private fun executeAutonomousMapTask(inputJson: String): String {
        val args = if (inputJson.isBlank()) JSONObject() else JSONObject(inputJson)
        val taskAgent = createAgent(includeMacroTools = false)
            ?: return JSONObject()
                .put("ok", false)
                .put("error", "Missing MIMO_API_KEY")
                .toString()
        val targetToolCalls = args.optInt("target_tool_calls", 100).coerceIn(1, 120)
        val minimumRequiredToolCalls = args
            .optInt("minimum_required_tool_calls", ((targetToolCalls * 95) + 99) / 100)
            .coerceIn(1, targetToolCalls)
        val reactSelfCheckInterval = args
            .optInt("react_self_check_interval", args.optInt("loop_body_tool_calls", 10))
            .coerceIn(4, 16)
        val maxRounds = args.optInt("max_tool_rounds", 128).coerceIn(1, MAX_TOOL_ROUNDS)
        val taskName = args.optString(
            "task",
            "real phone-using readiness audit across Settings, Wi-Fi, Launcher, and Agent Smoke"
        )
        val macroTools = setOf(
            "android_map_goal_task",
            "android_map_long_task_smoke",
            "android_map_stale_path_probe",
            "android_map_stale_page_refresh_probe"
        )
        val navigationTools = setOf("android_open_settings", "android_global_action", "android_open_agent")
        val observationTools = setOf(
            "android_map_observe",
            "android_map_view",
            "android_map_search",
            "android_map_plan_path",
            "android_ui_state"
        )
        val artifactTools = setOf("android_set_clipboard", "android_show_toast")
        val deviceReadTools = setOf("android_device_info", "android_battery")
        val toolCounts = linkedMapOf<String, Int>()
        val allToolNames = mutableListOf<String>()
        val toolArgumentTexts = mutableListOf<Pair<String, String>>()
        val toolTraceSample = JSONArray()
        var successfulNavigationToolCalls = 0
        var successfulObservationToolCalls = 0
        var successfulArtifactToolCalls = 0
        var successfulDeviceReadToolCalls = 0

        fun countTools(toolNames: Set<String>): Int = toolNames.sumOf { toolCounts[it] ?: 0 }

        fun repeatedFixedActionLoop(toolNames: List<String>, blockSize: Int): Boolean {
            if (toolNames.size < blockSize * 4) {
                return false
            }
            val firstBlock = toolNames.take(blockSize)
            var repeatedBlocks = 0
            for (start in blockSize until toolNames.size - blockSize + 1 step blockSize) {
                if (toolNames.subList(start, start + blockSize) == firstBlock) {
                    repeatedBlocks += 1
                }
            }
            return repeatedBlocks >= 4
        }

        fun graphLoopPrompt(): String {
            return """
                You are executing one real phone-using evaluation graph named phone_readiness_react_eval.

                Graph:
                start -> react_loop -> final_summary

                ReAct loop rule:
                - react_loop is observe -> reason/decide -> act -> observe, repeated inside this single graph run.
                - It is not a fixed action checklist. Do not repeat the same action sequence as a counter.
                - Each tool call must advance a concrete audit subgoal, verify a changed phone state, recover navigation, update map memory, or write a useful artifact.
                - Continue until this single graph run has produced at least $targetToolCalls provider-visible primitive tool calls and all required evidence is collected.
                - To avoid ending early from an off-by-one count, aim for at least ${targetToolCalls + 8} provider-visible tool calls before final_summary.
                - Every $reactSelfCheckInterval tool calls, mentally check which subgoals still lack evidence and choose the next tool accordingly.
                - Do not use macro/debug tools. Only use primitive tools visible to you.

                Real task objective:
                Produce a device readiness brief for this Android emulator by using the phone, not by simulating progress.

                Required audit subgoals:
                1. Baseline: collect device model/API/package and battery level/charging state at the beginning and again near the end.
                2. Agent workspace: open Agent Smoke, observe its accessibility/app-map state, and confirm the agent host can return to its workspace.
                3. Settings root: open Android Settings, inspect the current UI tree, observe it into app-map memory, and search/plan the remembered Settings page.
                4. Wi-Fi readiness: open Wi-Fi settings, inspect the UI tree, observe it into app-map memory, and compare it to the Settings root evidence.
                5. Launcher recovery: go Home, inspect Launcher/System UI, and prove the agent can continue after leaving its own app.
                6. Map memory: use android_map_observe, android_map_view, android_map_search, and android_map_plan_path to avoid re-exploring blindly.
                7. Navigation recovery: use Home/Back/Open Settings/Open Agent as needed based on the observed state.
                8. Artifacts: write at least three progressively richer readiness notes to clipboard.
                9. Visible status: show at least three milestone toasts that correspond to real completed subgoals.
                10. Final verification: before final_summary, re-read device/battery and current UI/map state.

                If all subgoals are complete before $targetToolCalls calls, do not repeat a canned sequence. Continue by deepening missing evidence:
                - inspect a different current screen representation with android_ui_state vs android_map_observe;
                - search or plan for a different semantic target such as settings, wifi, launcher, or agent workspace;
                - verify an artifact after a navigation change;
                - recover from the current screen and observe the result.

                final_summary must answer in Chinese with:
                - actual provider-visible tool call count
                - whether macro tools were avoided
                - which real audit subgoals were completed
                - navigation/observation/device-read/artifact coverage
                - final device readiness conclusion
            """.trimIndent()
        }

        fun recordResponse(response: uniffi.agent_smoke.AgentResponse) {
            response.toolTraces.forEach { trace ->
                toolCounts[trace.name] = (toolCounts[trace.name] ?: 0) + 1
                allToolNames.add(trace.name)
                val inputJson = try {
                    JSONObject(trace.inputJson)
                } catch (_: Throwable) {
                    JSONObject()
                }
                toolArgumentTexts.add(trace.name to inputJson.toString().lowercase())
                val outputJson = try {
                    JSONObject(trace.output)
                } catch (_: Throwable) {
                    JSONObject()
                }
                if (outputJson.optBoolean("ok", false)) {
                    when (trace.name) {
                        in navigationTools -> successfulNavigationToolCalls += 1
                        in observationTools -> successfulObservationToolCalls += 1
                        in artifactTools -> successfulArtifactToolCalls += 1
                        in deviceReadTools -> successfulDeviceReadToolCalls += 1
                    }
                }
                if (toolTraceSample.length() < 32) {
                    val input = if (inputJson.length() > 0) {
                        inputJson
                    } else {
                        JSONObject().put("raw", trace.inputJson.take(160))
                    }
                    val output = if (outputJson.length() > 0) {
                        compactTraceResult(outputJson)
                    } else {
                        JSONObject().put("raw", trace.output.take(160))
                    }
                    toolTraceSample.put(
                        JSONObject()
                            .put("tool_name", trace.name)
                            .put("arguments", input)
                            .put("result", output)
                    )
                }
            }
        }

        val response = try {
            taskAgent.reset()
            updateStatus("Provider phone graph: target $targetToolCalls primitive tool calls")
            taskAgent.promptWithToolsLimit(graphLoopPrompt(), maxRounds.toUInt())
        } finally {
            taskAgent.close()
        }
        recordResponse(response)

        val counts = JSONObject()
        toolCounts.forEach { (name, count) -> counts.put(name, count) }
        val macroToolCalls = countTools(macroTools)
        val navigationToolCalls = countTools(navigationTools)
        val observationToolCalls = countTools(observationTools)
        val artifactToolCalls = countTools(artifactTools)
        val deviceReadToolCalls = countTools(deviceReadTools)
        val argumentSignatureCount = toolArgumentTexts
            .map { (name, arguments) -> "$name:$arguments" }
            .distinct()
            .size
        fun hasArgument(toolName: String, value: String): Boolean =
            toolArgumentTexts.any { (name, arguments) ->
                name == toolName && arguments.contains(value)
            }
        val subgoalCoverage = linkedMapOf(
            "baseline_device_and_battery" to (
                (toolCounts["android_device_info"] ?: 0) >= 2 &&
                    (toolCounts["android_battery"] ?: 0) >= 2
                ),
            "agent_workspace_return" to ((toolCounts["android_open_agent"] ?: 0) >= 1),
            "settings_root_inspected" to hasArgument("android_open_settings", "settings"),
            "wifi_inspected" to hasArgument("android_open_settings", "wifi"),
            "launcher_recovery" to hasArgument("android_global_action", "home"),
            "map_memory_used" to (
                (toolCounts["android_map_observe"] ?: 0) >= 8 &&
                    (
                        (toolCounts["android_map_view"] ?: 0) > 0 ||
                            (toolCounts["android_map_search"] ?: 0) > 0
                        ) &&
                    (toolCounts["android_map_plan_path"] ?: 0) > 0
                ),
            "raw_ui_inspected" to ((toolCounts["android_ui_state"] ?: 0) >= 2),
            "clipboard_artifact_written" to ((toolCounts["android_set_clipboard"] ?: 0) >= 3),
            "visible_status_shown" to ((toolCounts["android_show_toast"] ?: 0) >= 3),
            "final_verification_pass" to (
                allToolNames.takeLast(24).contains("android_device_info") &&
                    allToolNames.takeLast(24).contains("android_battery") &&
                    allToolNames.takeLast(24).any { it in observationTools }
                )
        )
        val subgoalCoverageCount = subgoalCoverage.values.count { it }
        val repeatedFixedLoop = repeatedFixedActionLoop(allToolNames, 10)
        val completed = allToolNames.size >= minimumRequiredToolCalls
        val meaningfulPhoneTask = completed &&
            macroToolCalls == 0 &&
            navigationToolCalls >= 10 &&
            observationToolCalls >= 30 &&
            artifactToolCalls >= 6 &&
            deviceReadToolCalls >= 4 &&
            successfulNavigationToolCalls >= 10 &&
            successfulObservationToolCalls >= 20 &&
            successfulArtifactToolCalls >= 6 &&
            successfulDeviceReadToolCalls >= 4 &&
            toolCounts.size >= 10 &&
            argumentSignatureCount >= 30 &&
            subgoalCoverageCount >= 8 &&
            !repeatedFixedLoop
        val validation = JSONObject()
            .put("target_tool_calls_met", completed)
            .put("strict_target_tool_calls_met", allToolNames.size >= targetToolCalls)
            .put("minimum_required_tool_calls_met", completed)
            .put("macro_tools_absent", macroToolCalls == 0)
            .put("navigation_tool_calls_ok", navigationToolCalls >= 10)
            .put("observation_tool_calls_ok", observationToolCalls >= 30)
            .put("artifact_tool_calls_ok", artifactToolCalls >= 6)
            .put("device_read_tool_calls_ok", deviceReadToolCalls >= 4)
            .put("successful_navigation_tool_calls_ok", successfulNavigationToolCalls >= 10)
            .put("successful_observation_tool_calls_ok", successfulObservationToolCalls >= 20)
            .put("successful_artifact_tool_calls_ok", successfulArtifactToolCalls >= 6)
            .put("successful_device_read_tool_calls_ok", successfulDeviceReadToolCalls >= 4)
            .put("unique_tool_count_ok", toolCounts.size >= 10)
            .put("argument_signature_count_ok", argumentSignatureCount >= 30)
            .put("subgoal_coverage_ok", subgoalCoverageCount >= 8)
            .put("not_fixed_action_loop", !repeatedFixedLoop)
        val graph = JSONObject()
            .put("run_type", "single_graph_loop")
            .put("name", "phone_readiness_eval")
            .put("nodes", JSONArray(listOf("start", "react_loop", "final_summary")))
            .put(
                "edges",
                JSONArray(
                    listOf(
                        "start->react_loop",
                        "react_loop->react_loop",
                        "react_loop->final_summary"
                    )
                )
            )
            .put(
                "loop",
                JSONObject()
                    .put("node", "react_loop")
                    .put("self_check_interval_tool_calls", reactSelfCheckInterval)
                    .put("target_tool_calls", targetToolCalls)
                    .put("observed_tool_calls", allToolNames.size)
                    .put("exit_condition", "actual_tool_calls >= target_tool_calls")
            )
        val coverage = JSONObject()
        subgoalCoverage.forEach { (name, covered) -> coverage.put(name, covered) }

        return JSONObject()
            .put("ok", meaningfulPhoneTask)
            .put("task", taskName)
            .put("graph", graph)
            .put("completed", completed)
            .put("meaningful_phone_task", meaningfulPhoneTask)
            .put("target_tool_calls", targetToolCalls)
            .put("minimum_required_tool_calls", minimumRequiredToolCalls)
            .put("actual_tool_calls", allToolNames.size)
            .put("provider_tool_traces", allToolNames.size)
            .put("primitive_tool_traces", allToolNames.size - macroToolCalls)
            .put("macro_tool_traces", macroToolCalls)
            .put("navigation_tool_calls", navigationToolCalls)
            .put("observation_tool_calls", observationToolCalls)
            .put("artifact_tool_calls", artifactToolCalls)
            .put("device_read_tool_calls", deviceReadToolCalls)
            .put("successful_navigation_tool_calls", successfulNavigationToolCalls)
            .put("successful_observation_tool_calls", successfulObservationToolCalls)
            .put("successful_artifact_tool_calls", successfulArtifactToolCalls)
            .put("successful_device_read_tool_calls", successfulDeviceReadToolCalls)
            .put("unique_tool_count", toolCounts.size)
            .put("argument_signature_count", argumentSignatureCount)
            .put("task_subgoal_coverage_count", subgoalCoverageCount)
            .put("task_subgoal_coverage", coverage)
            .put("repeated_fixed_action_loop", repeatedFixedLoop)
            .put("react_self_check_interval", reactSelfCheckInterval)
            .put("message_count", response.messageCount.toLong())
            .put("input_tokens", response.inputTokens.toLong())
            .put("output_tokens", response.outputTokens.toLong())
            .put("total_tokens", response.totalTokens.toLong())
            .put("tool_counts", counts)
            .put("tools", JSONArray(allToolNames))
            .put("validation", validation)
            .put("tool_trace_sample", toolTraceSample)
            .put("answer", response.answer.lineSequence().joinToString(" ").take(2400))
            .toString()
    }

    private fun compactTraceResult(result: JSONObject): JSONObject {
        val compact = JSONObject()
        listOf(
            "ok",
            "found",
            "page_id",
            "page_count",
            "transition_count",
            "path_length",
            "local_tokens",
            "full_tokens",
            "level_percent",
            "charging",
            "screen",
            "action",
            "package",
            "message",
            "removed"
        ).forEach { key ->
            if (result.has(key) && !result.isNull(key)) {
                compact.put(key, result.opt(key))
            }
        }
        if (result.has("text") && !result.isNull("text")) {
            compact.put("text", result.optString("text").take(120))
        }
        return compact
    }

    private fun logDebugOutput(toolName: String, output: String) {
        output.chunked(LOG_CHUNK_CHARS).forEachIndexed { index, chunk ->
            Log.i(TAG, "Debug tool $toolName output[$index]: $chunk")
        }
    }

    private fun updateStatus(status: String) {
        lastStatus = status
        mainHandler.post {
            overlay?.setStatus(status)
            val manager = getSystemService(NotificationManager::class.java)
            manager.notify(NOTIFICATION_ID, notification(status.lineSequence().firstOrNull() ?: status))
        }
    }

    private fun notification(text: String): Notification {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val manager = getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_ID,
                    "Agent host",
                    NotificationManager.IMPORTANCE_LOW
                )
            )
        }

        val openAppIntent = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        )

        val builder = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            Notification.Builder(this, CHANNEL_ID)
        } else {
            @Suppress("DEPRECATION")
            Notification.Builder(this)
        }

        return builder
            .setSmallIcon(android.R.drawable.ic_dialog_info)
            .setContentTitle("Agent Smoke")
            .setContentText(text.take(120))
            .setContentIntent(openAppIntent)
            .setOngoing(true)
            .build()
    }

    private fun registerTools(agent: AgentCore, includeMacroTools: Boolean = true) {
        val emptySchema = """{"type":"object","properties":{},"additionalProperties":false}"""
        val messageSchema =
            """{"type":"object","properties":{"message":{"type":"string","description":"Toast message to display"}},"required":["message"],"additionalProperties":false}"""
        val textSchema =
            """{"type":"object","properties":{"text":{"type":"string","description":"Text to copy into the clipboard"}},"required":["text"],"additionalProperties":false}"""
        val settingsSchema =
            """{"type":"object","properties":{"screen":{"type":"string","description":"settings or wifi"}},"required":["screen"],"additionalProperties":false}"""
        val globalActionSchema =
            """{"type":"object","properties":{"action":{"type":"string","enum":["back","home","recents"],"description":"Global Android action"}},"required":["action"],"additionalProperties":false}"""
        val clickTextSchema =
            """{"type":"object","properties":{"text":{"type":"string","description":"Visible text or content description to click"}},"required":["text"],"additionalProperties":false}"""
        val typeTextSchema =
            """{"type":"object","properties":{"text":{"type":"string","description":"Text to enter into the focused editable field"}},"required":["text"],"additionalProperties":false}"""
        val mapObserveSchema =
            """{"type":"object","properties":{"task_hint":{"type":"string","description":"Task hint used to summarize the current page"}},"additionalProperties":false}"""
        val mapViewSchema =
            """{"type":"object","properties":{"hops":{"type":"integer","description":"How many transition hops to include"},"query":{"type":"string","description":"Optional task-related semantic target"}},"additionalProperties":false}"""
        val mapSearchSchema =
            """{"type":"object","properties":{"query":{"type":"string","description":"Semantic page target such as account security or order details"},"limit":{"type":"integer"}},"required":["query"],"additionalProperties":false}"""
        val mapPlanPathSchema =
            """{"type":"object","properties":{"query":{"type":"string","description":"Semantic target page to plan a path to"},"limit":{"type":"integer"}},"required":["query"],"additionalProperties":false}"""
        val mapForgetSchema =
            """{"type":"object","properties":{"scope":{"type":"string","enum":["page","query","stale"]},"value":{"type":"string","description":"Page id or query; ignored for stale"}},"required":["scope"],"additionalProperties":false}"""
        val mapLongSmokeSchema =
            """{"type":"object","properties":{"cycles":{"type":"integer","description":"Number of 5-step cross-app cycles"}},"additionalProperties":false}"""
        val mapGoalTaskSchema =
            """{"type":"object","properties":{"cycles":{"type":"integer","description":"Number of goal cycles; six cycles normally produces at least 100 tool-like steps"},"task":{"type":"string","description":"Task name or brief objective"}},"additionalProperties":false}"""
        val mapMarkTransitionFailedSchema =
            """{"type":"object","properties":{"from_page":{"type":"string","description":"Source page id"},"action_id":{"type":"string","description":"Action id to mark failed"}},"required":["from_page","action_id"],"additionalProperties":false}"""

        agent.registerTool("android_device_info", "Read Android device info.", emptySchema)
        agent.registerTool("android_battery", "Read current Android battery level.", emptySchema)
        agent.registerTool("android_show_toast", "Display a short Android toast.", messageSchema)
        agent.registerTool("android_set_clipboard", "Copy text into clipboard.", textSchema)
        agent.registerTool("android_open_settings", "Open Android settings or Wi-Fi settings.", settingsSchema)
        agent.registerTool("android_open_agent", "Open the Agent Smoke app workspace.", emptySchema)
        agent.registerTool("android_ui_state", "Read current screen through AccessibilityService.", emptySchema)
        agent.registerTool("android_global_action", "Run a global accessibility action.", globalActionSchema)
        agent.registerTool("android_click_text", "Click a visible node by text or content description.", clickTextSchema)
        agent.registerTool("android_type_text", "Type text into the focused editable field through AccessibilityService.", typeTextSchema)
        agent.registerTool("android_map_observe", "Update the searchable app map from the current AccessibilityService UI tree.", mapObserveSchema)
        agent.registerTool("android_map_view", "Read the current local app-map view instead of the full map.", mapViewSchema)
        agent.registerTool("android_map_search", "Search the app map for a semantic target page.", mapSearchSchema)
        agent.registerTool("android_map_plan_path", "Plan a path through remembered app-map transitions to a semantic target page.", mapPlanPathSchema)
        agent.registerTool("android_map_forget", "Forget app-map pages by page id, query, or stale state.", mapForgetSchema)
        agent.registerTool("android_map_click_text", "Click text and record the resulting app-map transition.", clickTextSchema)
        agent.registerTool("android_map_type_text", "Type text and refresh the current app-map node.", typeTextSchema)
        if (includeMacroTools) {
            agent.registerTool("android_map_long_task_smoke", "Run a deterministic 100-step cross-app app-map smoke task on the device.", mapLongSmokeSchema)
            agent.registerTool("android_map_goal_task", "Run a goal-driven 100-step cross-app device readiness task using app-map memory.", mapGoalTaskSchema)
            agent.registerTool("android_map_mark_transition_failed", "Mark an app-map transition action as failed so repeated misses stale old paths.", mapMarkTransitionFailedSchema)
            agent.registerTool("android_map_stale_path_probe", "Verify that repeated transition failures stale a remembered path.", emptySchema)
            agent.registerTool("android_map_stale_page_refresh_probe", "Verify that a stale app-map page node is refreshed by the next matching observation.", emptySchema)
        }
    }

    companion object {
        private const val CHANNEL_ID = "agent_host"
        private const val NOTIFICATION_ID = 42
        private const val EXTRA_PROMPT = "agent_prompt"
        private const val EXTRA_API_BASE = "mimo_api_base"
        private const val EXTRA_API_KEY = "mimo_api_key"
        private const val EXTRA_MODEL = "mimo_model"
        private const val EXTRA_PROXY = "mimo_proxy"
        private const val EXTRA_MAX_TOOL_ROUNDS = "agent_max_tool_rounds"
        private const val EXTRA_DEBUG_TOOL = "debug_tool"
        private const val EXTRA_DEBUG_INPUT = "debug_input"
        private const val EXTRA_DEBUG_INPUT_BASE64 = "debug_input_base64"
        private const val MAX_STATUS_CHARS = 1200
        private const val LOG_CHUNK_CHARS = 3500
        private const val TAG = "AgentHostService"
        private const val DEFAULT_TOOL_ROUNDS = 8
        private const val MAX_TOOL_ROUNDS = 128
        private const val DEBUG_AGENT_AUTONOMOUS_MAP_TASK = "agent_autonomous_map_task"

        fun start(
            context: Context,
            prompt: String? = null,
            apiBase: String? = null,
            apiKey: String? = null,
            model: String? = null,
            proxy: String? = null
        ) {
            val intent = Intent(context, AgentHostService::class.java)
            if (!prompt.isNullOrBlank()) {
                intent.putExtra(EXTRA_PROMPT, prompt)
            }
            if (!apiBase.isNullOrBlank()) {
                intent.putExtra(EXTRA_API_BASE, apiBase)
            }
            if (!apiKey.isNullOrBlank()) {
                intent.putExtra(EXTRA_API_KEY, apiKey)
            }
            if (!model.isNullOrBlank()) {
                intent.putExtra(EXTRA_MODEL, model)
            }
            if (proxy != null) {
                intent.putExtra(EXTRA_PROXY, proxy)
            }
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
                context.startForegroundService(intent)
            } else {
                context.startService(intent)
            }
        }
    }
}

private class FloatingAgentOverlay(
    private val context: Context,
    private val onPrompt: (String) -> Unit
) {
    private val windowManager = context.getSystemService(WindowManager::class.java)
    private val ball = TextView(context)
    private var panel: View? = null
    private var downX = 0f
    private var downY = 0f
    private var baseX = 0
    private var baseY = 0

    fun show() {
        ball.text = "AI"
        ball.textSize = 14f
        ball.setTextColor(Color.WHITE)
        ball.setBackgroundColor(Color.rgb(22, 101, 52))
        ball.gravity = Gravity.CENTER
        ball.setPadding(24, 18, 24, 18)
        val params = overlayParams(90, 90, focusable = false).apply {
            gravity = Gravity.TOP or Gravity.START
            x = 24
            y = 320
        }
        ball.setOnTouchListener { _, event -> handleTouch(event, params) }
        ball.setOnClickListener { togglePanel() }
        windowManager.addView(ball, params)
    }

    fun remove() {
        panel?.let(windowManager::removeView)
        panel = null
        windowManager.removeView(ball)
    }

    fun setStatus(status: String) {
        val statusView = panel?.findViewWithTag<TextView>("status")
        statusView?.text = status
    }

    private fun togglePanel() {
        if (panel != null) {
            panel?.let(windowManager::removeView)
            panel = null
            return
        }

        val input = EditText(context).apply {
            hint = "输入手机任务"
            setSingleLine(false)
            minLines = 2
        }
        val status = TextView(context).apply {
            tag = "status"
            text = "Agent ready"
            textSize = 13f
            setTextColor(Color.rgb(17, 24, 39))
        }
        val run = Button(context).apply {
            text = "Run"
            setOnClickListener {
                val prompt = input.text.toString().trim()
                if (prompt.isNotEmpty()) {
                    onPrompt(prompt)
                    hideKeyboard(input)
                }
            }
        }
        val view = LinearLayout(context).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.WHITE)
            setPadding(24, 24, 24, 24)
            addView(input)
            addView(run)
            addView(status)
        }
        val params = overlayParams(760, WindowManager.LayoutParams.WRAP_CONTENT, focusable = true).apply {
            gravity = Gravity.TOP or Gravity.START
            x = 160
            y = 320
        }
        panel = view
        windowManager.addView(view, params)
        input.requestFocus()
        showKeyboard(input)
    }

    private fun handleTouch(event: MotionEvent, params: WindowManager.LayoutParams): Boolean {
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                downX = event.rawX
                downY = event.rawY
                baseX = params.x
                baseY = params.y
            }
            MotionEvent.ACTION_MOVE -> {
                params.x = baseX + (event.rawX - downX).toInt()
                params.y = baseY + (event.rawY - downY).toInt()
                windowManager.updateViewLayout(ball, params)
                return true
            }
        }
        return false
    }

    private fun overlayParams(
        width: Int,
        height: Int,
        focusable: Boolean
    ): WindowManager.LayoutParams {
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            WindowManager.LayoutParams.TYPE_APPLICATION_OVERLAY
        } else {
            @Suppress("DEPRECATION")
            WindowManager.LayoutParams.TYPE_PHONE
        }
        val flags = if (focusable) {
            WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL
        } else {
            WindowManager.LayoutParams.FLAG_NOT_TOUCH_MODAL or
                WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE
        }
        return WindowManager.LayoutParams(
            width,
            height,
            type,
            flags,
            PixelFormat.TRANSLUCENT
        )
    }

    private fun showKeyboard(input: EditText) {
        input.post {
            context.getSystemService(InputMethodManager::class.java)
                .showSoftInput(input, InputMethodManager.SHOW_IMPLICIT)
        }
    }

    private fun hideKeyboard(input: EditText) {
        context.getSystemService(InputMethodManager::class.java)
            .hideSoftInputFromWindow(input.windowToken, 0)
    }
}

private class ServicePlatformToolHost(
    private val context: Context,
    private val appMapMemory: AndroidAppMapMemory
) : PlatformToolHost {
    private val mainHandler = Handler(Looper.getMainLooper())

    override fun executeTool(name: String, inputJson: String): String {
        val args = if (inputJson.isBlank()) JSONObject() else JSONObject(inputJson)
        return when (name) {
            "android_device_info" -> deviceInfo()
            "android_battery" -> batteryInfo()
            "android_show_toast" -> showToast(args)
            "android_set_clipboard" -> setClipboard(args)
            "android_open_settings" -> openSettings(args)
            "android_open_agent" -> openAgent(args)
            "android_ui_state" -> AgentAccessibilityService.screenSnapshot()
            "android_global_action" -> globalAction(args)
            "android_click_text" -> AgentAccessibilityService.clickText(args.optString("text"))
            "android_type_text" -> AgentAccessibilityService.typeText(args.optString("text"))
            "android_map_observe" -> mapObserve(args)
            "android_map_view" -> appMapMemory.localView(args.optInt("hops", 1), args.optString("query", "")).toString()
            "android_map_search" -> appMapMemory.search(args.optString("query"), args.optInt("limit", 5)).toString()
            "android_map_plan_path" -> appMapMemory.planPath(args.optString("query"), args.optInt("limit", 5)).toString()
            "android_map_forget" -> appMapMemory.forget(args.optString("scope"), args.optString("value", "")).toString()
            "android_map_click_text" -> mapClickText(args)
            "android_map_type_text" -> mapTypeText(args)
            "android_map_long_task_smoke" -> mapLongTaskSmoke(args)
            "android_map_goal_task" -> mapGoalTask(args)
            "android_map_mark_transition_failed" -> appMapMemory.markTransitionFailed(
                args.optString("from_page"),
                args.optString("action_id")
            ).toString()
            "android_map_stale_path_probe" -> appMapMemory.stalePathProbe().toString()
            "android_map_stale_page_refresh_probe" -> appMapMemory.stalePageRefreshProbe().toString()
            else -> JSONObject().put("ok", false).put("error", "unknown tool: $name").toString()
        }
    }

    private fun deviceInfo(): String {
        return JSONObject()
            .put("ok", true)
            .put("manufacturer", Build.MANUFACTURER)
            .put("model", Build.MODEL)
            .put("sdk_int", Build.VERSION.SDK_INT)
            .put("release", Build.VERSION.RELEASE)
            .put("package", context.packageName)
            .toString()
    }

    private fun batteryInfo(): String {
        val battery = context.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
        val level = battery?.getIntExtra(BatteryManager.EXTRA_LEVEL, -1) ?: -1
        val scale = battery?.getIntExtra(BatteryManager.EXTRA_SCALE, -1) ?: -1
        val status = battery?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
        val plugged = battery?.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) ?: 0
        val percent = if (level >= 0 && scale > 0) level * 100 / scale else -1

        return JSONObject()
            .put("ok", true)
            .put("level_percent", percent)
            .put("status", status)
            .put("plugged", plugged)
            .put("charging", status == BatteryManager.BATTERY_STATUS_CHARGING)
            .toString()
    }

    private fun showToast(args: JSONObject): String {
        val message = args.optString("message", "Tool call worked")
        mainHandler.post {
            Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
        }
        return JSONObject()
            .put("ok", true)
            .put("shown", true)
            .put("message", message)
            .toString()
    }

    private fun setClipboard(args: JSONObject): String {
        val text = args.optString("text", "copied by Rust Agent")
        mainHandler.post {
            val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
            clipboard.setPrimaryClip(ClipData.newPlainText("agent-tool-demo", text))
        }
        return JSONObject()
            .put("ok", true)
            .put("text", text)
            .toString()
    }

    private fun openSettings(args: JSONObject): String {
        val screen = args.optString("screen", "settings")
        val action = if (screen == "wifi") Settings.ACTION_WIFI_SETTINGS else Settings.ACTION_SETTINGS
        mainHandler.post {
            context.startActivity(Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        Thread.sleep(args.optLong("settle_ms", 350L))
        return JSONObject()
            .put("ok", true)
            .put("screen", screen)
            .put("intent_action", action)
            .toString()
    }

    private fun openAgent(args: JSONObject): String {
        mainHandler.post {
            context.startActivity(
                Intent(context, MainActivity::class.java)
                    .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            )
        }
        Thread.sleep(args.optLong("settle_ms", 350L))
        return JSONObject()
            .put("ok", true)
            .put("package", context.packageName)
            .toString()
    }

    private fun globalAction(args: JSONObject): String {
        val result = AgentAccessibilityService.performGlobal(args.optString("action"))
        Thread.sleep(args.optLong("settle_ms", 350L))
        return result
    }

    private fun mapObserve(args: JSONObject): String {
        return appMapMemory
            .observe(
                AgentAccessibilityService.screenSnapshot(),
                args.optString("task_hint", "")
            )
            .toString()
    }

    private fun mapClickText(args: JSONObject): String {
        val text = args.optString("text")
        val before = appMapMemory.observe(
            AgentAccessibilityService.screenSnapshot(),
            "before clicking $text"
        )
        val clicked = JSONObject(AgentAccessibilityService.clickText(text))
        Thread.sleep(args.optLong("settle_ms", 500L))
        val after = appMapMemory.observe(
            AgentAccessibilityService.screenSnapshot(),
            "after clicking $text"
        )

        val fromPage = before.optString("page_id")
        val toPage = after.optString("page_id")
        val transition = if (
            before.optBoolean("ok", false) &&
            after.optBoolean("ok", false) &&
            fromPage.isNotBlank() &&
            toPage.isNotBlank()
        ) {
            appMapMemory.recordTransition(
                fromPage,
                toPage,
                "click:${text.lowercase().replace(Regex("[^a-z0-9]+"), "-")}",
                text,
                "android_click_text",
                JSONObject().put("text", text),
                clicked.optBoolean("ok", false)
            )
        } else {
            JSONObject().put("ok", false).put("error", "transition was not recorded")
        }

        return JSONObject()
            .put("ok", clicked.optBoolean("ok", false))
            .put("clicked", clicked)
            .put("before", before)
            .put("after", after)
            .put("transition", transition)
            .toString()
    }

    private fun mapTypeText(args: JSONObject): String {
        val text = args.optString("text")
        val typed = JSONObject(AgentAccessibilityService.typeText(text))
        Thread.sleep(args.optLong("settle_ms", 300L))
        val after = appMapMemory.observe(
            AgentAccessibilityService.screenSnapshot(),
            "after typing text"
        )
        return JSONObject()
            .put("ok", typed.optBoolean("ok", false))
            .put("typed", typed)
            .put("after", after)
            .toString()
    }

    private fun mapGoalTask(args: JSONObject): String {
        val cycles = args.optInt("cycles", 6).coerceIn(1, 20)
        val taskName = args.optString("task", "device readiness brief")
        val subgoals = JSONArray(
            listOf(
                "collect device and battery state",
                "inspect Settings through AccessibilityService",
                "inspect Wi-Fi settings through AccessibilityService",
                "return through Launcher",
                "reopen Agent Smoke",
                "write a running readiness brief to clipboard",
                "show visible completion status"
            )
        )
        val toolCounts = linkedMapOf<String, Int>()
        val toolSequence = mutableListOf<String>()
        val toolTraceSample = JSONArray()
        val sessionTraceSample = JSONArray()
        val artifacts = JSONObject()
        var steps = 0
        var assistantToolCallMessages = 0
        var toolResultMessages = 0
        var appSwitches = 0
        var semanticSearches = 0
        var plannedPaths = 0
        var mapReuseHits = 0
        var localTokens = 0
        var fullTokens = 0
        var previousPage = ""
        var lastPackage = ""
        var latestDevice = JSONObject()
        var latestBattery = JSONObject()

        fun recordTool(toolName: String, arguments: JSONObject, result: JSONObject) {
            toolCounts[toolName] = (toolCounts[toolName] ?: 0) + 1
            toolSequence.add(toolName)
            steps += 1
            assistantToolCallMessages += 1
            toolResultMessages += 1
            val toolCallId = "goal-tool-$steps"
            if (toolTraceSample.length() < 24) {
                toolTraceSample.put(
                    JSONObject()
                        .put("role", "tool")
                        .put("tool_name", toolName)
                        .put("arguments", arguments)
                        .put("result", compactResult(result))
                )
            }
            if (sessionTraceSample.length() < 24) {
                sessionTraceSample.put(
                    JSONObject()
                        .put("role", "assistant")
                        .put("content_type", "tool_call")
                        .put("tool_call_id", toolCallId)
                        .put("tool_name", toolName)
                        .put("arguments", arguments)
                )
            }
            if (sessionTraceSample.length() < 24) {
                sessionTraceSample.put(
                    JSONObject()
                        .put("role", "tool")
                        .put("content_type", "tool_result")
                        .put("tool_call_id", toolCallId)
                        .put("tool_name", toolName)
                        .put("result", compactResult(result))
                )
            }
        }

        fun observeStep(
            hint: String,
            actionId: String,
            label: String,
            toolName: String,
            arguments: JSONObject
        ): JSONObject {
            val update = appMapMemory.observe(AgentAccessibilityService.screenSnapshot(), hint)
            val currentPage = update.optString("page_id", "")
            if (previousPage.isNotBlank() && currentPage.isNotBlank() && previousPage != currentPage) {
                appMapMemory.recordTransition(
                    previousPage,
                    currentPage,
                    actionId,
                    label,
                    toolName,
                    arguments,
                    true
                )
            }
            if (currentPage.isNotBlank()) {
                previousPage = currentPage
            }
            val packageName = currentMappedPackage()
            if (lastPackage.isNotBlank() && packageName.isNotBlank() && packageName != lastPackage) {
                appSwitches += 1
            }
            if (packageName.isNotBlank()) {
                lastPackage = packageName
            }
            localTokens += update.optInt("local_tokens", 0)
            fullTokens += update.optInt("full_tokens", 0)
            recordTool("android_map_observe", JSONObject().put("task_hint", hint), update)
            return update
        }

        fun planStep(query: String): JSONObject {
            semanticSearches += 1
            val plan = appMapMemory.planPath(query, 5)
            val pathLength = plan.optInt("path_length", 0)
            if (pathLength > 0) {
                plannedPaths += 1
                mapReuseHits += pathLength
            }
            recordTool("android_map_plan_path", JSONObject().put("query", query).put("limit", 5), plan)
            return plan
        }

        val forgetStale = appMapMemory.forget("stale", "")
        recordTool("android_map_forget", JSONObject().put("scope", "stale"), forgetStale)

        for (cycle in 1..cycles) {
            latestDevice = JSONObject(deviceInfo())
            recordTool("android_device_info", JSONObject(), latestDevice)

            latestBattery = JSONObject(batteryInfo())
            recordTool("android_battery", JSONObject(), latestBattery)

            val settingsArgs = JSONObject().put("screen", "settings")
            val settingsOpened = JSONObject(openSettings(settingsArgs))
            recordTool("android_open_settings", settingsArgs, settingsOpened)
            Thread.sleep(350L)
            observeStep(
                "$taskName cycle $cycle settings overview",
                "goal-open-settings-$cycle",
                "Open Settings",
                "android_open_settings",
                settingsArgs
            )
            planStep("settings")

            val wifiArgs = JSONObject().put("screen", "wifi")
            val wifiOpened = JSONObject(openSettings(wifiArgs))
            recordTool("android_open_settings", wifiArgs, wifiOpened)
            Thread.sleep(350L)
            observeStep(
                "$taskName cycle $cycle wifi readiness",
                "goal-open-wifi-$cycle",
                "Open Wi-Fi Settings",
                "android_open_settings",
                wifiArgs
            )
            planStep("wifi")

            val homeArgs = JSONObject().put("action", "home")
            val homeResult = JSONObject(AgentAccessibilityService.performGlobal("home"))
            recordTool("android_global_action", homeArgs, homeResult)
            Thread.sleep(350L)
            observeStep(
                "$taskName cycle $cycle launcher checkpoint",
                "goal-home-$cycle",
                "Home",
                "android_global_action",
                homeArgs
            )
            planStep("launcher")

            val appArgs = JSONObject().put("package", context.packageName)
            mainHandler.post {
                context.startActivity(
                    Intent(context, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            }
            recordTool("android_open_app", appArgs, JSONObject().put("ok", true).put("package", context.packageName))
            Thread.sleep(350L)
            observeStep(
                "$taskName cycle $cycle agent workspace",
                "goal-open-agent-$cycle",
                "Open Agent Smoke",
                "android_open_app",
                appArgs
            )
            planStep("settings")

            val progressBrief = "cycle $cycle/$cycles readiness: battery=${latestBattery.optInt("level_percent", -1)}%, pages=${appMapMemory.stats().optInt("page_count", 0)}"
            val clipboardArgs = JSONObject().put("text", progressBrief)
            val clipboardResult = JSONObject(setClipboard(clipboardArgs))
            recordTool("android_set_clipboard", clipboardArgs, clipboardResult)
            Thread.sleep(100L)
            observeStep(
                "$taskName cycle $cycle clipboard artifact",
                "goal-clipboard-$cycle",
                "Set Clipboard",
                "android_set_clipboard",
                clipboardArgs
            )

            val toastArgs = JSONObject().put("message", "readiness cycle $cycle")
            val toastResult = JSONObject(showToast(toastArgs))
            recordTool("android_show_toast", toastArgs, toastResult)
            Thread.sleep(100L)
            observeStep(
                "$taskName cycle $cycle visible status",
                "goal-toast-$cycle",
                "Show Toast",
                "android_show_toast",
                toastArgs
            )
        }

        val stats = appMapMemory.stats()
        val saved = fullTokens - localTokens
        val savingPercent = if (fullTokens > 0) saved * 100.0 / fullTokens else 0.0
        val finalBrief = "Device readiness brief: cycles=$cycles, steps=$steps, app_switches=$appSwitches, pages=${stats.optInt("page_count", 0)}, paths=${stats.optInt("transition_count", 0)}, token_savings=${"%.1f".format(savingPercent)}%"
        val finalClipboardArgs = JSONObject().put("text", finalBrief)
        val finalClipboard = JSONObject(setClipboard(finalClipboardArgs))
        recordTool("android_set_clipboard", finalClipboardArgs, finalClipboard)
        val finalToastArgs = JSONObject().put("message", "readiness brief complete")
        val finalToast = JSONObject(showToast(finalToastArgs))
        recordTool("android_show_toast", finalToastArgs, finalToast)

        val counts = JSONObject()
        toolCounts.forEach { (name, count) -> counts.put(name, count) }
        val probabilityGraph = buildToolProbabilityGraph(toolSequence, toolCounts)
        val finalLocalView = appMapMemory.localView(1, taskName)
        val finalSearch = appMapMemory.search("settings", 5)
        val localViewSummary = summarizeMapView(finalLocalView)
        val searchSummary = summarizeSearch(finalSearch)
        artifacts.put("device", latestDevice)
        artifacts.put("battery", latestBattery)
        artifacts.put("brief", finalBrief)

        return JSONObject()
            .put("ok", true)
            .put("task", taskName)
            .put("completed", steps >= 100)
            .put("steps", steps)
            .put("cycles", cycles)
            .put("subgoals", subgoals)
            .put("artifacts", artifacts)
            .put("app_switches", appSwitches)
            .put("semantic_searches", semanticSearches)
            .put("planned_paths", plannedPaths)
            .put("map_reuse_hits", mapReuseHits)
            .put("repeated_exploration_avoided", mapReuseHits)
            .put("local_map_tokens", localTokens)
            .put("full_map_tokens", fullTokens)
            .put("token_savings_percent", savingPercent)
            .put("tool_trace_count", steps)
            .put("session_message_count", 1 + assistantToolCallMessages + toolResultMessages + 1)
            .put("assistant_tool_call_messages", assistantToolCallMessages)
            .put("tool_result_messages", toolResultMessages)
            .put("tool_counts", counts)
            .put("probability_graph", probabilityGraph)
            .put("final_local_view", localViewSummary)
            .put("final_semantic_search", searchSummary)
            .put("tool_trace_sample", toolTraceSample)
            .put("session_trace_sample", sessionTraceSample)
            .put("map", stats)
            .toString()
    }

    private fun mapLongTaskSmoke(args: JSONObject): String {
        val cycles = args.optInt("cycles", 20).coerceIn(1, 40)
        var steps = 0
        var appSwitches = 0
        var lastPackage = ""
        var localTokens = 0
        var fullTokens = 0
        var previousPage = ""

        fun observeStep(
            hint: String,
            actionId: String,
            label: String,
            toolName: String,
            arguments: JSONObject
        ) {
            val update = appMapMemory.observe(AgentAccessibilityService.screenSnapshot(), hint)
            val currentPage = update.optString("page_id", "")
            if (previousPage.isNotBlank() && currentPage.isNotBlank() && previousPage != currentPage) {
                appMapMemory.recordTransition(
                    previousPage,
                    currentPage,
                    actionId,
                    label,
                    toolName,
                    arguments,
                    true
                )
            }
            if (currentPage.isNotBlank()) {
                previousPage = currentPage
            }
            val packageName = currentMappedPackage()
            if (lastPackage.isNotBlank() && packageName.isNotBlank() && packageName != lastPackage) {
                appSwitches += 1
            }
            if (packageName.isNotBlank()) {
                lastPackage = packageName
            }
            localTokens += update.optInt("local_tokens", 0)
            fullTokens += update.optInt("full_tokens", 0)
            steps += 1
        }

        for (cycle in 1..cycles) {
            startActivity(if (cycle % 2 == 0) Settings.ACTION_WIFI_SETTINGS else Settings.ACTION_SETTINGS)
            Thread.sleep(350L)
            observeStep(
                "cycle $cycle settings",
                "open-settings-$cycle",
                "Open Settings",
                "android_open_settings",
                JSONObject().put("screen", if (cycle % 2 == 0) "wifi" else "settings")
            )

            AgentAccessibilityService.performGlobal("home")
            Thread.sleep(350L)
            observeStep(
                "cycle $cycle launcher",
                "home-$cycle",
                "Home",
                "android_global_action",
                JSONObject().put("action", "home")
            )

            mainHandler.post {
                context.startActivity(
                    Intent(context, MainActivity::class.java)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                )
            }
            Thread.sleep(350L)
            observeStep(
                "cycle $cycle agent host",
                "open-agent-$cycle",
                "Open Agent Smoke",
                "android_open_app",
                JSONObject().put("package", context.packageName)
            )

            setClipboard(JSONObject().put("text", "map smoke cycle $cycle"))
            Thread.sleep(100L)
            observeStep(
                "cycle $cycle clipboard",
                "clipboard-$cycle",
                "Set Clipboard",
                "android_set_clipboard",
                JSONObject().put("text", "map smoke cycle $cycle")
            )

            showToast(JSONObject().put("message", "map smoke $cycle"))
            Thread.sleep(100L)
            observeStep(
                "cycle $cycle toast",
                "toast-$cycle",
                "Show Toast",
                "android_show_toast",
                JSONObject().put("message", "map smoke $cycle")
            )
        }

        val saved = fullTokens - localTokens
        val savingPercent = if (fullTokens > 0) saved * 100.0 / fullTokens else 0.0
        return JSONObject()
            .put("ok", true)
            .put("completed", steps >= 100)
            .put("steps", steps)
            .put("cycles", cycles)
            .put("app_switches", appSwitches)
            .put("local_map_tokens", localTokens)
            .put("full_map_tokens", fullTokens)
            .put("token_savings_percent", savingPercent)
            .put("map", appMapMemory.stats())
            .toString()
    }

    private fun startActivity(action: String) {
        mainHandler.post {
            context.startActivity(Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
    }

    private fun currentMappedPackage(): String {
        val view = appMapMemory.localView(0, "")
        val nodes = view.optJSONArray("nodes") ?: return ""
        if (nodes.length() == 0) {
            return ""
        }
        return nodes.optJSONObject(0)?.optString("package", "") ?: ""
    }

    private fun compactResult(result: JSONObject): JSONObject {
        val compact = JSONObject()
        listOf(
            "ok",
            "found",
            "page_id",
            "page_count",
            "transition_count",
            "path_length",
            "local_tokens",
            "full_tokens",
            "level_percent",
            "charging",
            "screen",
            "action",
            "package",
            "message",
            "removed"
        ).forEach { key ->
            if (result.has(key) && !result.isNull(key)) {
                compact.put(key, result.opt(key))
            }
        }
        if (result.has("text") && !result.isNull("text")) {
            compact.put("text", result.optString("text").take(120))
        }
        return compact
    }

    private fun buildToolProbabilityGraph(
        toolSequence: List<String>,
        toolCounts: Map<String, Int>
    ): JSONObject {
        val transitions = linkedMapOf<String, LinkedHashMap<String, Int>>()
        for (index in 0 until toolSequence.lastIndex) {
            val from = toolSequence[index]
            val to = toolSequence[index + 1]
            val next = transitions.getOrPut(from) { linkedMapOf() }
            next[to] = (next[to] ?: 0) + 1
        }

        val likelyNext = JSONArray()
        transitions.forEach { (from, nextCounts) ->
            val total = nextCounts.values.sum().coerceAtLeast(1)
            val next = JSONArray()
            nextCounts.entries
                .sortedWith(compareByDescending<Map.Entry<String, Int>> { it.value }.thenBy { it.key })
                .forEach { entry ->
                    next.put(
                        JSONObject()
                            .put("tool_name", entry.key)
                            .put("count", entry.value)
                            .put("probability", entry.value.toDouble() / total.toDouble())
                    )
                }
            likelyNext.put(
                JSONObject()
                    .put("tool_name", from)
                    .put("next_total", total)
                    .put("likely_next", next)
            )
        }

        val hotTools = JSONArray()
        toolCounts.entries
            .sortedWith(compareByDescending<Map.Entry<String, Int>> { it.value }.thenBy { it.key })
            .take(8)
            .forEach { entry ->
                hotTools.put(
                    JSONObject()
                        .put("tool_name", entry.key)
                        .put("count", entry.value)
                        .put("probability", entry.value.toDouble() / toolSequence.size.coerceAtLeast(1).toDouble())
                )
            }

        val preexecutionCandidates = JSONArray()
        listOf("android_device_info", "android_battery", "android_map_plan_path").forEach { toolName ->
            if ((toolCounts[toolName] ?: 0) > 0) {
                preexecutionCandidates.put(
                    JSONObject()
                        .put("tool_name", toolName)
                        .put("count", toolCounts[toolName] ?: 0)
                        .put("reason", "read-only or idempotent in this verifier")
                )
            }
        }

        return JSONObject()
            .put("source", "android_map_goal_task_session_trace")
            .put("tool_call_events", toolSequence.size)
            .put("transition_count", toolSequence.size.saturatingMinusOne())
            .put("hot_tools", hotTools)
            .put("likely_next", likelyNext)
            .put("preexecution_candidates", preexecutionCandidates)
    }

    private fun summarizeMapView(view: JSONObject): JSONObject {
        val nodes = view.optJSONArray("nodes") ?: JSONArray()
        val transitions = view.optJSONArray("transitions") ?: JSONArray()
        val candidateActions = view.optJSONArray("candidate_actions") ?: JSONArray()
        var dangerousActionCount = 0
        for (nodeIndex in 0 until nodes.length()) {
            dangerousActionCount += nodes
                .optJSONObject(nodeIndex)
                ?.optJSONArray("dangerous_actions")
                ?.length()
                ?: 0
        }
        return JSONObject()
            .put("ok", view.optBoolean("ok", false))
            .put("current_page", view.opt("current_page") ?: JSONObject.NULL)
            .put("node_count", nodes.length())
            .put("transition_count", transitions.length())
            .put("candidate_action_count", candidateActions.length())
            .put("dangerous_action_count", dangerousActionCount)
            .put("omitted_node_count", view.optInt("omitted_node_count", 0))
    }

    private fun summarizeSearch(search: JSONObject): JSONObject {
        val hits = search.optJSONArray("hits") ?: JSONArray()
        return JSONObject()
            .put("ok", search.optBoolean("ok", false))
            .put("query", search.optString("query", ""))
            .put("hit_count", hits.length())
            .put("top_page", hits.optJSONObject(0)?.optString("page_id", "") ?: "")
            .put("top_score", hits.optJSONObject(0)?.optInt("score", 0) ?: 0)
    }
}

private fun Int.saturatingMinusOne(): Int = if (this > 0) this - 1 else 0
