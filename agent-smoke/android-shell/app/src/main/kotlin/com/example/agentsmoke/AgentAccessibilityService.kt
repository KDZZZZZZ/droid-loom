package com.example.agentsmoke

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.AccessibilityServiceInfo
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference
import org.json.JSONArray
import org.json.JSONObject

class AgentAccessibilityService : AccessibilityService() {
    override fun onServiceConnected() {
        val configured = serviceInfo
        configured.flags = configured.flags or
            AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS or
            AccessibilityServiceInfo.FLAG_REPORT_VIEW_IDS
        serviceInfo = configured
        instance = this
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) = Unit

    override fun onInterrupt() = Unit

    override fun onDestroy() {
        if (instance === this) {
            instance = null
        }
        super.onDestroy()
    }

    companion object {
        @Volatile
        private var instance: AgentAccessibilityService? = null
        private val mainHandler = Handler(Looper.getMainLooper())

        fun screenSnapshot(): String {
            if (Looper.myLooper() == Looper.getMainLooper()) {
                return screenSnapshotOnMain()
            }

            var lastSnapshot = ""
            repeat(20) {
                lastSnapshot = onMain { screenSnapshotOnMain() }
                if (snapshotIsReady(lastSnapshot)) {
                    return lastSnapshot
                }
                Thread.sleep(150L)
            }
            return lastSnapshot
        }

        fun performGlobal(actionName: String): String = onMain { performGlobalOnMain(actionName) }

        fun clickText(text: String): String = onMain { clickTextOnMain(text) }

        fun typeText(text: String): String = onMain { typeTextOnMain(text) }

        private fun screenSnapshotOnMain(): String {
            val service = instance
                ?: return unavailable()
            val windows = service.windows
            if (!windows.isNullOrEmpty()) {
                val allNodes = JSONArray()
                val windowSnapshots = JSONArray()
                var activePackage = ""
                for ((index, window) in windows.withIndex()) {
                    val root = window.root ?: continue
                    val nodes = JSONArray()
                    collectNodes(root, nodes, 0, 80)
                    val packageName = root.packageName?.toString().orEmpty()
                    if (window.isActive || activePackage.isBlank()) {
                        activePackage = packageName
                    }
                    for (nodeIndex in 0 until nodes.length()) {
                        allNodes.put(nodes.getJSONObject(nodeIndex))
                    }
                    windowSnapshots.put(
                        JSONObject()
                            .put("index", index)
                            .put("type", window.type)
                            .put("layer", window.layer)
                            .put("active", window.isActive)
                            .put("focused", window.isFocused)
                            .put("package", packageName)
                            .put("nodes", nodes)
                    )
                    root.recycle()
                }
                if (allNodes.length() > 0) {
                    return JSONObject()
                        .put("ok", true)
                        .put("package", activePackage)
                        .put("nodes", allNodes)
                        .put("windows", windowSnapshots)
                        .toString()
                }
            }

            val root = service.rootInActiveWindow
                ?: return JSONObject().put("ok", false).put("error", "no active window").toString()

            val nodes = JSONArray()
            collectNodes(root, nodes, 0, 80)
            val activePackage = root.packageName?.toString().orEmpty()
            root.recycle()
            return JSONObject()
                .put("ok", true)
                .put("package", activePackage)
                .put("nodes", nodes)
                .toString()
        }

        private fun performGlobalOnMain(actionName: String): String {
            val service = instance
                ?: return unavailable()
            val action = when (actionName) {
                "back" -> GLOBAL_ACTION_BACK
                "home" -> GLOBAL_ACTION_HOME
                "recents" -> GLOBAL_ACTION_RECENTS
                else -> return JSONObject()
                    .put("ok", false)
                    .put("error", "unsupported global action: $actionName")
                    .toString()
            }
            return JSONObject()
                .put("ok", service.performGlobalAction(action))
                .put("action", actionName)
                .toString()
        }

        private fun clickTextOnMain(text: String): String {
            val service = instance
                ?: return unavailable()
            val roots = currentRoots(service)
            if (roots.isEmpty()) {
                return JSONObject().put("ok", false).put("error", "no active window").toString()
            }

            var node: AccessibilityNodeInfo? = null
            for (root in roots) {
                node = findClickableText(root, text)
                if (node != null) {
                    break
                }
            }

            val clicked = node?.performAction(AccessibilityNodeInfo.ACTION_CLICK) == true
            val result = JSONObject()
                .put("ok", clicked)
                .put("text", text)
            if (!clicked) {
                result.put("error", "clickable node not found")
            }
            node?.recycle()
            roots.forEach(AccessibilityNodeInfo::recycle)
            return result.toString()
        }

        private fun typeTextOnMain(text: String): String {
            val service = instance
                ?: return unavailable()
            val roots = currentRoots(service)
            if (roots.isEmpty()) {
                return JSONObject().put("ok", false).put("error", "no active window").toString()
            }

            var node: AccessibilityNodeInfo? = null
            for (root in roots) {
                node = findNode(root) { candidate ->
                    candidate.isFocused && candidate.isEditable
                } ?: findNode(root) { candidate ->
                    candidate.isEditable
                }
                if (node != null) {
                    break
                }
            }

            val args = Bundle().apply {
                putCharSequence(
                    AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE,
                    text
                )
            }
            val typed = node?.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args) == true
            val result = JSONObject()
                .put("ok", typed)
                .put("text", text)
            if (!typed) {
                result.put("error", "editable node not found")
            }
            node?.recycle()
            roots.forEach(AccessibilityNodeInfo::recycle)
            return result.toString()
        }

        private fun onMain(action: () -> String): String {
            if (Looper.myLooper() == Looper.getMainLooper()) {
                return action()
            }

            val result = AtomicReference<String>()
            val latch = CountDownLatch(1)
            mainHandler.post {
                try {
                    result.set(action())
                } catch (error: Throwable) {
                    result.set(
                        JSONObject()
                            .put("ok", false)
                            .put("error", error.message ?: error.javaClass.simpleName)
                            .toString()
                    )
                } finally {
                    latch.countDown()
                }
            }
            if (!latch.await(3, TimeUnit.SECONDS)) {
                return JSONObject()
                    .put("ok", false)
                    .put("error", "accessibility action timed out")
                    .toString()
            }
            return result.get()
        }

        private fun currentRoots(service: AgentAccessibilityService): List<AccessibilityNodeInfo> {
            val roots = mutableListOf<AccessibilityNodeInfo>()
            val windows = service.windows
            if (!windows.isNullOrEmpty()) {
                for (window in windows) {
                    window.root?.let(roots::add)
                }
            }
            if (roots.isEmpty()) {
                service.rootInActiveWindow?.let(roots::add)
            }
            return roots
        }

        private fun findClickableText(root: AccessibilityNodeInfo, text: String): AccessibilityNodeInfo? {
            return findNode(root) { candidate ->
                candidate.text?.toString() == text ||
                    candidate.contentDescription?.toString() == text
            }?.let(::nearestClickable)
                ?: findNode(root) { candidate ->
                    candidate.text?.toString()?.contains(text, ignoreCase = true) == true ||
                        candidate.contentDescription?.toString()?.contains(text, ignoreCase = true) == true
                }?.let(::nearestClickable)
        }

        private fun collectNodes(
            node: AccessibilityNodeInfo,
            output: JSONArray,
            depth: Int,
            remaining: Int
        ): Int {
            if (remaining <= 0) {
                return 0
            }

            val bounds = android.graphics.Rect()
            node.getBoundsInScreen(bounds)
            output.put(
                JSONObject()
                    .put("text", node.text?.toString().orEmpty())
                    .put("description", node.contentDescription?.toString().orEmpty())
                    .put("class", node.className?.toString().orEmpty())
                    .put("clickable", node.isClickable)
                    .put("editable", node.isEditable)
                    .put("focused", node.isFocused)
                    .put("depth", depth)
                    .put("bounds", "${bounds.left},${bounds.top},${bounds.right},${bounds.bottom}")
            )

            var used = 1
            for (index in 0 until node.childCount) {
                val child = node.getChild(index) ?: continue
                used += collectNodes(child, output, depth + 1, remaining - used)
                child.recycle()
                if (used >= remaining) {
                    break
                }
            }
            return used
        }

        private fun findNode(
            node: AccessibilityNodeInfo,
            predicate: (AccessibilityNodeInfo) -> Boolean
        ): AccessibilityNodeInfo? {
            if (predicate(node)) {
                return AccessibilityNodeInfo.obtain(node)
            }
            for (index in 0 until node.childCount) {
                val child = node.getChild(index) ?: continue
                val found = findNode(child, predicate)
                child.recycle()
                if (found != null) {
                    return found
                }
            }
            return null
        }

        private fun nearestClickable(node: AccessibilityNodeInfo): AccessibilityNodeInfo? {
            var current: AccessibilityNodeInfo? = AccessibilityNodeInfo.obtain(node)
            while (current != null) {
                if (current.isClickable) {
                    return current
                }
                val parent = current.parent
                current.recycle()
                current = parent
            }
            return null
        }

        private fun unavailable(): String {
            return JSONObject()
                .put("ok", false)
                .put("error", "accessibility service is not enabled")
                .toString()
        }

        private fun snapshotIsReady(snapshot: String): Boolean {
            return try {
                val parsed = JSONObject(snapshot)
                parsed.optBoolean("ok", false) &&
                    (parsed.optJSONArray("nodes")?.length() ?: 0) > 0
            } catch (_: Throwable) {
                false
            }
        }
    }
}
