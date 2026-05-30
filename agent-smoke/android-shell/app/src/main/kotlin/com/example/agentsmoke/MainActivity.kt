package com.example.agentsmoke

import android.app.Activity
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Color
import android.os.BatteryManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.view.Gravity
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import android.widget.Toast
import org.json.JSONObject
import uniffi.agent_smoke.AgentCore
import uniffi.agent_smoke.AgentException
import uniffi.agent_smoke.PlatformToolHost
import kotlin.concurrent.thread

class MainActivity : Activity() {
    private lateinit var statusView: TextView
    private lateinit var outputView: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        statusView = TextView(this).apply {
            text = "Starting Rust Agent Core..."
            textSize = 16f
            setTextColor(Color.rgb(22, 101, 52))
        }

        outputView = TextView(this).apply {
            textSize = 15f
            setTextColor(Color.rgb(17, 24, 39))
            setLineSpacing(6f, 1f)
        }

        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.START
            setPadding(40, 48, 40, 40)
            addView(statusView)
            addView(outputView)
        }

        setContentView(ScrollView(this).apply { addView(container) })
        runAgentToolDemo()
    }

    private fun runAgentToolDemo() {
        val apiKey = BuildConfig.DEEPSEEK_API_KEY
        if (apiKey.isBlank()) {
            showResult("Missing DEEPSEEK_API_KEY", "Set DEEPSEEK_API_KEY before building the debug APK.")
            return
        }

        thread(name = "agent-tool-demo") {
            try {
                val agent = createAgent(apiKey)
                agent.use {
                    agent.setPlatformToolHost(AndroidPlatformToolHost(this))
                    registerAndroidTools(agent)

                    val registeredTools = agent.listTools().joinToString("\n") { tool ->
                        "- ${tool.name}: ${tool.description}"
                    }
                    val userRequest = intent.getStringExtra("agent_prompt")
                        ?: "Run android_device_info, android_battery, android_show_toast with message " +
                            "\"Tool call worked\", and android_set_clipboard with text " +
                            "\"copied by Rust Agent\". After the tools return, summarize in Chinese."

                    val response = agent.promptWithTools(userRequest)

                    val traces = response.toolTraces.joinToString("\n\n") { trace ->
                        "tool: ${trace.name}\ninput: ${trace.inputJson}\noutput: ${trace.output}"
                    }

                    showResult(
                        "OK: Android platform tools demo",
                        "model: ${agent.model()}\n" +
                            "message_count: ${response.messageCount}\n\n" +
                            "user request:\n$userRequest\n\n" +
                            "registered tools:\n$registeredTools\n\n" +
                            "tool traces:\n$traces\n\n" +
                            "assistant:\n${response.answer}"
                    )
                }
            } catch (error: AgentException) {
                showResult("Agent error", error.message ?: error.javaClass.simpleName)
            } catch (error: Throwable) {
                showResult("Runtime error", error.stackTraceToString())
            }
        }
    }

    private fun createAgent(apiKey: String): AgentCore {
        val proxyUrl = BuildConfig.DEEPSEEK_PROXY
        return if (proxyUrl.isBlank()) {
            AgentCore(apiKey, BuildConfig.DEEPSEEK_MODEL)
        } else {
            AgentCore.newWithBaseUrlAndProxy(
                "https://api.deepseek.com",
                apiKey,
                BuildConfig.DEEPSEEK_MODEL,
                proxyUrl
            )
        }
    }

    private fun registerAndroidTools(agent: AgentCore) {
        val emptySchema = """{"type":"object","properties":{},"additionalProperties":false}"""
        val messageSchema =
            """{"type":"object","properties":{"message":{"type":"string","description":"Toast message to display"}},"required":["message"],"additionalProperties":false}"""
        val textSchema =
            """{"type":"object","properties":{"text":{"type":"string","description":"Text to copy into the clipboard"}},"required":["text"],"additionalProperties":false}"""
        val settingsSchema =
            """{"type":"object","properties":{"screen":{"type":"string","description":"settings or wifi"}},"required":["screen"],"additionalProperties":false}"""

        agent.registerTool(
            "android_device_info",
            "Read Android emulator/device manufacturer, model, SDK version, and package name.",
            emptySchema
        )
        agent.registerTool(
            "android_battery",
            "Read current Android battery level and charging status.",
            emptySchema
        )
        agent.registerTool(
            "android_show_toast",
            "Display a short Android toast message on screen.",
            messageSchema
        )
        agent.registerTool(
            "android_set_clipboard",
            "Copy text into the Android clipboard.",
            textSchema
        )
        agent.registerTool(
            "android_open_settings",
            "Open Android settings or Wi-Fi settings using an Activity intent.",
            settingsSchema
        )
    }

    private fun showResult(status: String, output: String) {
        runOnUiThread {
            statusView.text = status
            outputView.text = output
        }
    }
}

class AndroidPlatformToolHost(private val activity: Activity) : PlatformToolHost {
    private val mainHandler = Handler(Looper.getMainLooper())

    override fun executeTool(name: String, inputJson: String): String {
        val args = if (inputJson.isBlank()) JSONObject() else JSONObject(inputJson)
        return when (name) {
            "android_device_info" -> deviceInfo()
            "android_battery" -> batteryInfo()
            "android_show_toast" -> showToast(args)
            "android_set_clipboard" -> setClipboard(args)
            "android_open_settings" -> openSettings(args)
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
            .put("package", activity.packageName)
            .toString()
    }

    private fun batteryInfo(): String {
        val battery = activity.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
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
            Toast.makeText(activity, message, Toast.LENGTH_SHORT).show()
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
            val clipboard = activity.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
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
            activity.startActivity(Intent(action).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        return JSONObject()
            .put("ok", true)
            .put("screen", screen)
            .put("intent_action", action)
            .toString()
    }
}
