package com.example.agentsmoke

import android.app.Activity
import android.content.Intent
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.view.Gravity
import android.widget.Button
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView

class MainActivity : Activity() {
    private lateinit var statusView: TextView
    private lateinit var outputView: TextView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        statusView = TextView(this).apply {
            text = "Agent host starting..."
            textSize = 16f
            setTextColor(Color.rgb(22, 101, 52))
        }
        outputView = TextView(this).apply {
            textSize = 15f
            setTextColor(Color.rgb(17, 24, 39))
            setLineSpacing(6f, 1f)
        }

        val startButton = Button(this).apply {
            text = "Run service demo"
            setOnClickListener {
                startHost(DEFAULT_PROMPT)
                showStatus()
            }
        }
        val overlayButton = Button(this).apply {
            text = "Grant overlay permission"
            setOnClickListener { openOverlaySettings() }
        }
        val accessibilityButton = Button(this).apply {
            text = "Open accessibility settings"
            setOnClickListener {
                startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))
            }
        }

        val container = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            gravity = Gravity.START
            setPadding(40, 48, 40, 40)
            addView(statusView)
            addView(startButton)
            addView(overlayButton)
            addView(accessibilityButton)
            addView(outputView)
        }
        setContentView(ScrollView(this).apply { addView(container) })

        startHost(intent.getStringExtra("agent_prompt"))
        showStatus()
    }

    override fun onResume() {
        super.onResume()
        showStatus()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        startHost(intent.getStringExtra("agent_prompt"))
        showStatus()
    }

    private fun showStatus() {
        val overlay = Settings.canDrawOverlays(this)
        statusView.text = if (intentConfig("mimo_api_key", BuildConfig.MIMO_API_KEY).isBlank()) {
            "Missing MIMO_API_KEY"
        } else {
            "Agent host running"
        }
        outputView.text =
            "Runtime: foreground AgentHostService\n" +
                "Overlay permission: $overlay\n" +
                "Accessibility: open settings and enable Agent Smoke\n\n" +
                "Floating ball appears after overlay permission is granted. " +
                "Accessibility tools become active after the service is enabled."
    }

    private fun startHost(prompt: String?) {
        val serviceIntent = Intent(this, AgentHostService::class.java)
        prompt?.takeIf { it.isNotBlank() }?.let {
            serviceIntent.putExtra("agent_prompt", it)
        }
        for (name in listOf("mimo_api_base", "mimo_api_key", "mimo_model", "mimo_proxy", "debug_tool", "debug_input", "debug_input_base64")) {
            intent.getStringExtra(name)?.let {
                serviceIntent.putExtra(name, it)
            }
        }
        if (intent.hasExtra("agent_max_tool_rounds")) {
            serviceIntent.putExtra(
                "agent_max_tool_rounds",
                intent.getIntExtra("agent_max_tool_rounds", 8)
            )
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            startForegroundService(serviceIntent)
        } else {
            startService(serviceIntent)
        }
    }

    private fun intentConfig(name: String, fallback: String): String {
        return intent.getStringExtra(name)?.takeIf { it.isNotBlank() } ?: fallback
    }

    private fun openOverlaySettings() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.M) {
            startActivity(
                Intent(
                    Settings.ACTION_MANAGE_OVERLAY_PERMISSION,
                    Uri.parse("package:$packageName")
                )
            )
        }
    }

    companion object {
        private const val DEFAULT_PROMPT =
            "Run android_device_info, android_battery, android_ui_state, android_set_clipboard with text service agent works, and android_show_toast with message service agent works. Summarize in Chinese."
    }
}
