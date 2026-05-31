plugins {
    id("com.android.application")
}

val mimoApiBase = providers.environmentVariable("MIMO_API_BASE")
    .orElse(providers.environmentVariable("DEEPSEEK_API_BASE"))
    .orElse("https://api.xiaomimimo.com/v1")
val mimoApiKey = providers.environmentVariable("MIMO_API_KEY")
    .orElse(providers.environmentVariable("DEEPSEEK_API_KEY"))
    .orElse("")
val mimoModel = providers.environmentVariable("MIMO_MODEL")
    .orElse(providers.environmentVariable("DEEPSEEK_MODEL"))
    .orElse("mimo-v2.5-pro")
val mimoProxy = providers.environmentVariable("MIMO_PROXY")
    .orElse(providers.environmentVariable("DEEPSEEK_PROXY"))
    .orElse("")

android {
    namespace = "com.example.agentsmoke"
    compileSdk = 35

    buildFeatures {
        buildConfig = true
    }

    defaultConfig {
        applicationId = "com.example.agentsmoke"
        minSdk = 24
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"

        buildConfigField("String", "MIMO_API_BASE", "\"${mimoApiBase.get()}\"")
        buildConfigField("String", "MIMO_API_KEY", "\"${mimoApiKey.get()}\"")
        buildConfigField("String", "MIMO_MODEL", "\"${mimoModel.get()}\"")
        buildConfigField("String", "MIMO_PROXY", "\"${mimoProxy.get()}\"")
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_21
        targetCompatibility = JavaVersion.VERSION_21
    }
}

dependencies {
    implementation("net.java.dev.jna:jna:5.18.1@aar")
}

android.sourceSets["main"].kotlin.srcDir("../../bindings/kotlin")
