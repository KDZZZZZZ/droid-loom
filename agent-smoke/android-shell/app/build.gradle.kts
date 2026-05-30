plugins {
    id("com.android.application")
}

val deepseekApiKey = providers.environmentVariable("DEEPSEEK_API_KEY").orElse("")
val deepseekModel = providers.environmentVariable("DEEPSEEK_MODEL").orElse("deepseek-v4-pro")
val deepseekProxy = providers.environmentVariable("DEEPSEEK_PROXY").orElse("")

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

        buildConfigField("String", "DEEPSEEK_API_KEY", "\"${deepseekApiKey.get()}\"")
        buildConfigField("String", "DEEPSEEK_MODEL", "\"${deepseekModel.get()}\"")
        buildConfigField("String", "DEEPSEEK_PROXY", "\"${deepseekProxy.get()}\"")
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
