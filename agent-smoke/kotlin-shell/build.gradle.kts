plugins {
    kotlin("jvm") version "2.2.21"
    application
}

dependencies {
    implementation("net.java.dev.jna:jna:5.18.1")
}

kotlin {
    jvmToolchain(21)
}

sourceSets {
    main {
        kotlin.srcDir("../bindings/kotlin")
    }
}

application {
    mainClass.set("app.MainKt")
}
