package app

import java.nio.file.Paths
import uniffi.agent_smoke.AgentCore

fun main(args: Array<String>) {
    val libraryPath = Paths.get("..", "target", "debug", "agent_smoke.dll")
        .toAbsolutePath()
        .normalize()
        .toString()
    System.setProperty("uniffi.component.agent_smoke.libraryOverride", libraryPath)

    val apiKey = firstEnv("MIMO_API_KEY", "DEEPSEEK_API_KEY")
        ?: error("MIMO_API_KEY is not set")
    val model = firstEnv("MIMO_MODEL", "DEEPSEEK_MODEL") ?: "mimo-v2.5-pro"
    val apiBase = firstEnv("MIMO_API_BASE", "DEEPSEEK_API_BASE")
        ?: "https://api.xiaomimimo.com/v1"
    val prompt = args.joinToString(" ").ifBlank {
        "Say in one sentence that Kotlin called Rust AgentCore through UniFFI."
    }

    AgentCore.newWithBaseUrl(apiBase, apiKey, model).use { agent ->
        println("library: $libraryPath")
        println("model: ${agent.model()}")
        println("user: $prompt")

        val response = agent.prompt(prompt)
        println("assistant: ${response.answer}")
        println("message_count: ${response.messageCount}")
    }
}

private fun firstEnv(vararg names: String): String? =
    names.firstNotNullOfOrNull { name -> System.getenv(name)?.takeIf { it.isNotBlank() } }
