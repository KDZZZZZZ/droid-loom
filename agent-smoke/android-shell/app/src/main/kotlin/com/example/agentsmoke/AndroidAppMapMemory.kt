package com.example.agentsmoke

import org.json.JSONArray
import org.json.JSONObject
import java.util.ArrayDeque
import java.util.Locale
import java.util.TreeSet
import kotlin.math.max

class AndroidAppMapMemory {
    private val pages = linkedMapOf<String, PageNode>()
    private val transitions = mutableListOf<PageTransition>()
    private var currentPageId: String? = null
    private var stepCounter = 0

    @Synchronized
    fun observe(uiStateJson: String, taskHint: String = ""): JSONObject {
        val state = JSONObject(uiStateJson)
        if (!state.optBoolean("ok", false)) {
            return JSONObject()
                .put("ok", false)
                .put("error", state.optString("error", "ui_state is not ok"))
        }

        stepCounter += 1
        val packageName = state.optString("package", "unknown")
        val elements = functionalElements(extractElements(state))
        if (elements.isEmpty()) {
            return JSONObject()
                .put("ok", false)
                .put("error", "ui_state has no functional nodes")
        }

        val fingerprint = fingerprint(packageName, elements)
        val pageId = "page:$fingerprint"
        val previous = pages[pageId]
        val node = PageNode(
            id = pageId,
            packageName = packageName,
            summary = summarize(packageName, taskHint, elements),
            fingerprint = fingerprint,
            keyButtons = elements.filter { it.clickable && !it.dangerous }.take(8),
            inputFields = elements.filter { it.editable }.take(6),
            navigationEntries = elements.filter { looksLikeNavigation(it) }.take(6),
            dangerousActions = elements.filter { it.dangerous }.take(6),
            visits = (previous?.visits ?: 0) + 1,
            stale = false,
            lastSeenStep = stepCounter
        )
        pages[pageId] = node
        currentPageId = pageId

        val actions = candidateActions(pageId, elements)
        return JSONObject()
            .put("ok", true)
            .put("page_id", pageId)
            .put("created", previous == null)
            .put("replaced_stale_node", previous?.stale == true)
            .put("page_count", pages.size)
            .put("transition_count", transitions.size)
            .put("candidate_actions", actionsToJson(actions))
            .put("local_tokens", estimateLocalTokens(localViewJson(pageId, 1, taskHint)))
            .put("full_tokens", estimateFullMapTokens())
    }

    @Synchronized
    fun recordTransition(
        fromPage: String,
        toPage: String,
        actionId: String,
        label: String,
        toolName: String,
        arguments: JSONObject,
        success: Boolean
    ): JSONObject {
        val existing = transitions.firstOrNull {
            it.fromPage == fromPage && it.toPage == toPage && it.actionId == actionId
        }
        if (existing != null) {
            if (success) {
                existing.successCount += 1
                existing.stale = false
            } else {
                existing.failureCount += 1
                existing.stale = existing.failureCount >= existing.successCount + 2
            }
        } else {
            transitions.add(
                PageTransition(
                    fromPage = fromPage,
                    toPage = toPage,
                    actionId = actionId,
                    label = label,
                    toolName = toolName,
                    arguments = arguments.toString(),
                    successCount = if (success) 1 else 0,
                    failureCount = if (success) 0 else 1,
                    stale = !success
                )
            )
        }
        return JSONObject()
            .put("ok", true)
            .put("from_page", fromPage)
            .put("to_page", toPage)
            .put("action_id", actionId)
            .put("transition_count", transitions.size)
            .put("stale_transition_count", staleTransitionCount())
    }

    @Synchronized
    fun markTransitionFailed(fromPage: String, actionId: String): JSONObject {
        var changed = 0
        transitions
            .filter { it.fromPage == fromPage && it.actionId == actionId }
            .forEach {
                it.failureCount += 1
                it.stale = it.failureCount >= it.successCount + 2
                changed += 1
            }
        return JSONObject()
            .put("ok", true)
            .put("from_page", fromPage)
            .put("action_id", actionId)
            .put("changed", changed)
            .put("stale_transition_count", staleTransitionCount())
    }

    @Synchronized
    fun markPageStale(pageId: String): JSONObject {
        val page = pages[pageId]
            ?: return JSONObject()
                .put("ok", false)
                .put("page_id", pageId)
                .put("error", "page not found")
        page.stale = true
        return JSONObject()
            .put("ok", true)
            .put("page_id", pageId)
            .put("stale", true)
    }

    @Synchronized
    fun localView(hops: Int, query: String = ""): JSONObject {
        val pageId = currentPageId
            ?: return JSONObject()
                .put("ok", true)
                .put("current_page", JSONObject.NULL)
                .put("nodes", JSONArray())
                .put("transitions", JSONArray())
                .put("candidate_actions", JSONArray())
        return localViewJson(pageId, hops.coerceAtLeast(0), query)
    }

    @Synchronized
    fun search(query: String, limit: Int = 5): JSONObject {
        val hits = semanticSearch(query, limit)
        return JSONObject()
            .put("ok", true)
            .put("query", query)
            .put("hits", JSONArray(hits.map { it.toJson() }))
    }

    @Synchronized
    fun planPath(query: String, limit: Int = 5): JSONObject {
        val startPage = currentPageId
            ?: return JSONObject()
                .put("ok", true)
                .put("found", false)
                .put("query", query)
                .put("current_page", JSONObject.NULL)
                .put("target_page", JSONObject.NULL)
                .put("path", JSONArray())
                .put("path_length", 0)

        val target = semanticSearch(query, limit.coerceAtLeast(1)).firstOrNull()
            ?: return JSONObject()
                .put("ok", true)
                .put("found", false)
                .put("query", query)
                .put("current_page", startPage)
                .put("target_page", JSONObject.NULL)
                .put("path", JSONArray())
                .put("path_length", 0)

        val path = shortestPath(startPage, target.pageId)
        return JSONObject()
            .put("ok", true)
            .put("found", path != null)
            .put("query", query)
            .put("current_page", startPage)
            .put("target_page", target.pageId)
            .put("target_score", target.score)
            .put("path", JSONArray((path ?: emptyList()).map { it.toJson() }))
            .put("path_length", path?.size ?: 0)
    }

    @Synchronized
    fun forget(scope: String, value: String): JSONObject {
        val ids = when (scope) {
            "page" -> setOf(value)
            "query" -> semanticSearch(value, Int.MAX_VALUE).map { it.pageId }.toSet()
            "stale" -> pages.values.filter { it.stale }.map { it.id }.toSet()
            else -> emptySet()
        }
        ids.forEach { pages.remove(it) }
        transitions.removeAll { ids.contains(it.fromPage) || ids.contains(it.toPage) }
        if (ids.contains(currentPageId)) {
            currentPageId = null
        }
        return JSONObject()
            .put("ok", true)
            .put("removed", ids.size)
            .put("page_count", pages.size)
            .put("transition_count", transitions.size)
    }

    @Synchronized
    fun stats(): JSONObject {
        return JSONObject()
            .put("ok", true)
            .put("current_page", currentPageId ?: JSONObject.NULL)
            .put("page_count", pages.size)
            .put("transition_count", transitions.size)
            .put("stale_transition_count", staleTransitionCount())
            .put("full_tokens", estimateFullMapTokens())
    }

    @Synchronized
    fun stalePathProbe(): JSONObject {
        stepCounter += 1
        val fromPage = "page:probe-stale-source-$stepCounter"
        val toPage = "page:probe-stale-target-$stepCounter"
        pages[fromPage] = probePage(
            fromPage,
            "probe.stale.source",
            "probe stale source page with obsolete navigation",
            "Open obsolete target",
            stepCounter
        )
        pages[toPage] = probePage(
            toPage,
            "probe.stale.target",
            "probe stale target page",
            "Probe complete",
            stepCounter
        )
        val actionId = "probe:stale-path"
        val record = recordTransition(
            fromPage,
            toPage,
            actionId,
            "Probe stale path",
            "android_map_stale_path_probe",
            JSONObject().put("probe", true),
            true
        )
        val query = "probe stale target page"
        currentPageId = fromPage
        val preStalePlan = planPath(query, 5)
        val failureOne = markTransitionFailed(fromPage, actionId)
        val failureTwo = markTransitionFailed(fromPage, actionId)
        val failureThree = markTransitionFailed(fromPage, actionId)
        val secondPlan = planPath(pages[toPage]?.summary ?: pages[toPage]?.packageName ?: toPage, 5)
        return JSONObject()
            .put("ok", true)
            .put("record", record)
            .put("pre_stale_plan", preStalePlan)
            .put("failure_one", failureOne)
            .put("failure_two", failureTwo)
            .put("failure_three", failureThree)
            .put("post_stale_plan", secondPlan)
            .put("stale_transition_count", staleTransitionCount())
    }

    @Synchronized
    fun stalePageRefreshProbe(): JSONObject {
        val uiState = JSONObject()
            .put("ok", true)
            .put("package", "probe.stale.refresh")
            .put(
                "nodes",
                JSONArray(
                    listOf(
                        JSONObject()
                            .put("text", "Account security")
                            .put("class", "android.widget.Button")
                            .put("clickable", true),
                        JSONObject()
                            .put("text", "Password")
                            .put("class", "android.widget.EditText")
                            .put("editable", true)
                    )
                )
            )
        val first = observe(uiState.toString(), "stale page refresh initial")
        val pageId = first.optString("page_id", "")
        val mark = markPageStale(pageId)
        val refreshed = observe(uiState.toString(), "stale page refresh updated")
        return JSONObject()
            .put("ok", true)
            .put("first", first)
            .put("mark", mark)
            .put("refreshed", refreshed)
            .put("same_page", pageId.isNotBlank() && pageId == refreshed.optString("page_id", ""))
            .put("replaced_stale_node", refreshed.optBoolean("replaced_stale_node", false))
            .put("page_count", pages.size)
    }

    private fun localViewJson(pageId: String, hops: Int, query: String): JSONObject {
        val reachable = reachablePages(pageId, hops).toMutableSet()
        if (query.isNotBlank()) {
            semanticSearch(query, 4).forEach { reachable.add(it.pageId) }
        }
        val nodes = reachable.mapNotNull { pages[it] }.sortedBy { it.id }
        val visible = nodes.map { it.id }.toSet()
        val visibleTransitions = transitions.filter {
            !it.stale && visible.contains(it.fromPage) && visible.contains(it.toPage)
        }
        val candidates = pages[pageId]?.let {
            candidateActions(
                it.id,
                it.keyButtons + it.inputFields + it.navigationEntries + it.dangerousActions
            )
        } ?: emptyList()

        return JSONObject()
            .put("ok", true)
            .put("current_page", pageId)
            .put("nodes", JSONArray(nodes.map { it.toJson() }))
            .put("transitions", JSONArray(visibleTransitions.map { it.toJson() }))
            .put("candidate_actions", actionsToJson(candidates))
            .put("omitted_node_count", max(0, pages.size - nodes.size))
    }

    private fun staleTransitionCount(): Int = transitions.count { it.stale }

    private fun probePage(
        id: String,
        packageName: String,
        summary: String,
        buttonLabel: String,
        step: Int
    ): PageNode {
        val element = UiElement(
            stableId = "$id:button",
            label = buttonLabel,
            className = "android.widget.Button",
            clickable = true,
            editable = false,
            dangerous = false,
            bounds = ""
        )
        return PageNode(
            id = id,
            packageName = packageName,
            summary = summary,
            fingerprint = id.removePrefix("page:"),
            keyButtons = listOf(element),
            inputFields = emptyList(),
            navigationEntries = listOf(element),
            dangerousActions = emptyList(),
            visits = 1,
            stale = false,
            lastSeenStep = step
        )
    }

    private fun semanticSearch(query: String, limit: Int): List<SearchHit> {
        val queryTokens = tokenize(query)
        if (queryTokens.isEmpty() && query.isBlank()) {
            return emptyList()
        }
        return pages.values.mapNotNull { page ->
            val haystack = buildString {
                append(page.packageName).append(' ')
                append(page.summary).append(' ')
                append(page.keyButtons.joinToString(" ") { it.label })
                append(' ')
                append(page.inputFields.joinToString(" ") { it.label })
            }
            val pageTokens = tokenize(haystack)
            val overlap = queryTokens.count { pageTokens.contains(it) }
            val substringBonus = if (
                query.isNotBlank() &&
                haystack.lowercase(Locale.US).contains(query.lowercase(Locale.US))
            ) 3 else 0
            val matched = overlap * 2 + substringBonus
            if (matched > 0) {
                SearchHit(page.id, matched + if (page.stale) 0 else 1, page.summary)
            } else {
                null
            }
        }.sortedWith(compareByDescending<SearchHit> { it.score }.thenBy { it.pageId })
            .take(limit)
    }

    private fun reachablePages(start: String, hops: Int): Set<String> {
        val seen = linkedSetOf(start)
        val queue = ArrayDeque<Pair<String, Int>>()
        queue.add(start to 0)
        while (!queue.isEmpty()) {
            val (page, depth) = queue.removeFirst()
            if (depth >= hops) continue
            transitions
                .filter { it.fromPage == page && !it.stale }
                .forEach {
                    if (seen.add(it.toPage)) {
                        queue.add(it.toPage to depth + 1)
                    }
                }
        }
        return seen
    }

    private fun shortestPath(start: String, target: String): List<PageTransition>? {
        if (start == target) {
            return emptyList()
        }
        val seen = linkedSetOf(start)
        val queue = ArrayDeque<String>()
        val previous = linkedMapOf<String, PageTransition>()
        queue.add(start)
        while (!queue.isEmpty()) {
            val page = queue.removeFirst()
            transitions
                .filter { it.fromPage == page && !it.stale }
                .forEach { transition ->
                    if (seen.add(transition.toPage)) {
                        previous[transition.toPage] = transition
                        if (transition.toPage == target) {
                            return rebuildPath(start, target, previous)
                        }
                        queue.add(transition.toPage)
                    }
                }
        }
        return null
    }

    private fun rebuildPath(
        start: String,
        target: String,
        previous: Map<String, PageTransition>
    ): List<PageTransition>? {
        val path = mutableListOf<PageTransition>()
        var page = target
        while (page != start) {
            val transition = previous[page] ?: return null
            path.add(transition)
            page = transition.fromPage
        }
        path.reverse()
        return path
    }

    private fun estimateFullMapTokens(): Int {
        val chars = pages.values.sumOf { page ->
            page.summary.length +
                page.keyButtons.sumOf { it.label.length } +
                page.inputFields.sumOf { it.label.length } +
                page.dangerousActions.sumOf { it.label.length }
        } + transitions.sumOf { it.fromPage.length + it.toPage.length + it.label.length }
        return max(1, chars / 4)
    }

    private fun estimateLocalTokens(view: JSONObject): Int {
        var chars = 0
        val nodes = view.optJSONArray("nodes") ?: JSONArray()
        for (index in 0 until nodes.length()) {
            val node = nodes.optJSONObject(index) ?: continue
            chars += node.optString("summary").length
            val buttons = node.optJSONArray("key_buttons") ?: JSONArray()
            for (buttonIndex in 0 until buttons.length()) {
                chars += buttons.optJSONObject(buttonIndex)?.optString("label", "")?.length ?: 0
            }
            val inputs = node.optJSONArray("input_fields") ?: JSONArray()
            for (inputIndex in 0 until inputs.length()) {
                chars += inputs.optJSONObject(inputIndex)?.optString("label", "")?.length ?: 0
            }
        }
        val transitions = view.optJSONArray("transitions") ?: JSONArray()
        for (index in 0 until transitions.length()) {
            chars += transitions.optJSONObject(index)?.optString("label", "")?.length ?: 0
        }
        val actions = view.optJSONArray("candidate_actions") ?: JSONArray()
        for (index in 0 until actions.length()) {
            val action = actions.optJSONObject(index) ?: continue
            chars += action.optString("label").length + action.optString("tool_name").length
        }
        return max(1, chars / 4)
    }
}

private data class UiElement(
    val stableId: String,
    val label: String,
    val className: String,
    val clickable: Boolean,
    val editable: Boolean,
    val dangerous: Boolean,
    val bounds: String
) {
    fun toJson(): JSONObject {
        return JSONObject()
            .put("id", stableId)
            .put("label", label)
            .put("class", className)
            .put("clickable", clickable)
            .put("editable", editable)
            .put("dangerous", dangerous)
            .put("bounds", bounds)
    }
}

private data class PageNode(
    val id: String,
    val packageName: String,
    val summary: String,
    val fingerprint: String,
    val keyButtons: List<UiElement>,
    val inputFields: List<UiElement>,
    val navigationEntries: List<UiElement>,
    val dangerousActions: List<UiElement>,
    val visits: Int,
    var stale: Boolean,
    val lastSeenStep: Int
) {
    fun toJson(): JSONObject {
        return JSONObject()
            .put("id", id)
            .put("package", packageName)
            .put("summary", summary)
            .put("fingerprint", fingerprint)
            .put("key_buttons", JSONArray(keyButtons.map { it.toJson() }))
            .put("input_fields", JSONArray(inputFields.map { it.toJson() }))
            .put("navigation_entries", JSONArray(navigationEntries.map { it.toJson() }))
            .put("dangerous_actions", JSONArray(dangerousActions.map { it.toJson() }))
            .put("visits", visits)
            .put("stale", stale)
            .put("last_seen_step", lastSeenStep)
    }
}

private data class PageTransition(
    val fromPage: String,
    val toPage: String,
    val actionId: String,
    val label: String,
    val toolName: String,
    val arguments: String,
    var successCount: Int,
    var failureCount: Int,
    var stale: Boolean
) {
    fun toJson(): JSONObject {
        return JSONObject()
            .put("from_page", fromPage)
            .put("to_page", toPage)
            .put("action_id", actionId)
            .put("label", label)
            .put("tool_name", toolName)
            .put("arguments", JSONObject(arguments))
            .put("success_count", successCount)
            .put("failure_count", failureCount)
            .put("stale", stale)
    }
}

private data class CandidateAction(
    val id: String,
    val label: String,
    val toolName: String,
    val arguments: JSONObject,
    val dangerous: Boolean
) {
    fun toJson(): JSONObject {
        return JSONObject()
            .put("id", id)
            .put("label", label)
            .put("tool_name", toolName)
            .put("arguments", arguments)
            .put("dangerous", dangerous)
    }
}

private data class SearchHit(
    val pageId: String,
    val score: Int,
    val summary: String
) {
    fun toJson(): JSONObject {
        return JSONObject()
            .put("page_id", pageId)
            .put("score", score)
            .put("summary", summary)
    }
}

private fun extractElements(state: JSONObject): List<UiElement> {
    val nodes = state.optJSONArray("nodes") ?: state.optJSONArray("elements") ?: JSONArray()
    val elements = mutableListOf<UiElement>()
    for (index in 0 until nodes.length()) {
        val node = nodes.optJSONObject(index) ?: continue
        val text = node.optString("text", "").trim()
        val description = node.optString("description", "").trim()
        val label = if (text.isNotBlank()) text else description
        val className = node.optString("class", node.optString("class_name", ""))
        elements.add(
            UiElement(
                stableId = node.optString("id", "node:$index:${sanitize(label + className)}"),
                label = label,
                className = className,
                clickable = node.optBoolean("clickable", false),
                editable = node.optBoolean("editable", false),
                dangerous = isDangerous(label),
                bounds = node.optString("bounds", "")
            )
        )
    }
    return elements
}

private fun functionalElements(elements: List<UiElement>): List<UiElement> {
    val seen = TreeSet<String>()
    return elements.filter {
        it.clickable || it.editable || it.dangerous || looksLikeNavigation(it)
    }.filter {
        seen.add("${it.label.lowercase(Locale.US)}|${it.className}|${it.clickable}|${it.editable}")
    }.take(40)
}

private fun candidateActions(pageId: String, elements: List<UiElement>): List<CandidateAction> {
    val actions = mutableListOf<CandidateAction>()
    elements.filter { it.label.isNotBlank() && !it.dangerous }.forEach { element ->
        if (element.clickable) {
            actions.add(
                CandidateAction(
                    id = "click:${sanitize(element.label)}",
                    label = element.label,
                    toolName = "android_click_text",
                    arguments = JSONObject().put("text", element.label),
                    dangerous = false
                )
            )
        }
        if (element.editable) {
            actions.add(
                CandidateAction(
                    id = "type:${sanitize(element.label)}",
                    label = element.label,
                    toolName = "android_type_text",
                    arguments = JSONObject().put("text", "<task_text>"),
                    dangerous = false
                )
            )
        }
    }
    actions.add(
        CandidateAction(
            id = "system:back:$pageId",
            label = "Back",
            toolName = "android_global_action",
            arguments = JSONObject().put("action", "back"),
            dangerous = false
        )
    )
    return actions
}

private fun actionsToJson(actions: List<CandidateAction>): JSONArray {
    return JSONArray(actions.map { it.toJson() })
}

private fun summarize(packageName: String, taskHint: String, elements: List<UiElement>): String {
    val buttons = elements.filter { it.clickable && !it.dangerous }.map { it.label }.filter { it.isNotBlank() }.take(6)
    val inputs = elements.filter { it.editable }.map { it.label }.filter { it.isNotBlank() }.take(4)
    val dangerous = elements.filter { it.dangerous }.map { it.label }.filter { it.isNotBlank() }.take(4)
    return "package=$packageName; task_hint=$taskHint; buttons=[${buttons.joinToString(", ")}]; inputs=[${inputs.joinToString(", ")}]; dangerous=[${dangerous.joinToString(", ")}]"
}

private fun fingerprint(packageName: String, elements: List<UiElement>): String {
    val labels = elements.map { sanitize(it.label) }.filter { it.isNotBlank() }.distinct().sorted().take(12)
    val source = "$packageName:${labels.joinToString("|")}"
    return "${sanitize(packageName)}-${source.hashCode().toString(16)}"
}

private fun looksLikeNavigation(element: UiElement): Boolean {
    val label = element.label.lowercase(Locale.US)
    return label in setOf("back", "home", "settings", "search", "account", "orders") ||
        element.className.lowercase(Locale.US).contains("tab")
}

private fun isDangerous(label: String): Boolean {
    val lowered = label.lowercase(Locale.US)
    return listOf("delete", "cancel", "remove", "logout", "sign out", "pay", "purchase")
        .any { lowered.contains(it) }
}

private fun tokenize(text: String): Set<String> {
    return text.split(Regex("[^A-Za-z0-9]+"))
        .filter { it.length >= 2 }
        .map { it.lowercase(Locale.US) }
        .toSet()
}

private fun sanitize(text: String): String {
    return text.lowercase(Locale.US)
        .replace(Regex("[^a-z0-9]+"), "-")
        .trim('-')
}
