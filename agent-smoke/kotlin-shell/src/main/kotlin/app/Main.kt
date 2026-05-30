package app

import java.nio.file.Paths
import uniffi.agent_smoke.AgentCore

fun main(args: Array<String>) {
    val libraryPath = Paths.get("..", "target", "debug", "agent_smoke.dll")
        .toAbsolutePath()
        .normalize()
        .toString()
    System.setProperty("uniffi.component.agent_smoke.libraryOverride", libraryPath)

    val apiKey = System.getenv("DEEPSEEK_API_KEY")
        ?: error("DEEPSEEK_API_KEY is not set")
    val model = System.getenv("DEEPSEEK_MODEL") ?: "deepseek-v4-pro"
    val prompt = args.joinToString(" ").ifBlank {
        "Say in one sentence that Kotlin called Rust AgentCore through UniFFI."
    }

    AgentCore(apiKey, model).use { agent ->
        println("library: $libraryPath")
        println("model: ${agent.model()}")
        println("user: $prompt")

        val response = agent.prompt(prompt)
        println("assistant: ${response.answer}")
        println("message_count: ${response.messageCount}")
    }
}
