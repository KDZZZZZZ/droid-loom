# agent-smoke

Minimal PoC for an Android phone-using agent with a Rust core.

Validated chain:

```text
Android Kotlin App
  -> UniFFI Kotlin bindings
  -> Rust AgentCore
  -> DeepSeek/OpenAI-compatible Chat Completions API
  -> model tool calls
  -> Rust PlatformToolHost callback
  -> Android platform tools
```

## What is implemented

- `AgentCore` owns in-memory conversation state.
- Rust HTTP client calls `deepseek-v4-pro`.
- Tool schemas can be registered in Rust.
- `prompt_with_tools()` runs a minimal tool-call loop.
- Kotlin implements `PlatformToolHost` and executes Android APIs.
- Android demo registers:
  - `android_device_info`
  - `android_battery`
  - `android_show_toast`
  - `android_set_clipboard`
  - `android_open_settings`

## Desktop smoke test

```powershell
$env:DEEPSEEK_API_KEY="your_api_key"
$env:DEEPSEEK_MODEL="deepseek-v4-pro"
cargo run -- "Say hello in one short sentence."
```

## Generate Kotlin bindings

```powershell
cargo build
uniffi-bindgen generate --library target\debug\agent_smoke.dll --language kotlin --out-dir bindings\kotlin
```

## Build Android native libraries

```powershell
$env:ANDROID_HOME="D:\Android\Sdk"
.\scripts\build-android-libs.ps1
```

This builds and copies:

```text
android-shell/app/src/main/jniLibs/arm64-v8a/libagent_smoke.so
android-shell/app/src/main/jniLibs/x86_64/libagent_smoke.so
```

The `.so` files are ignored because they are large rebuildable artifacts.

## Build Android APK

```powershell
$env:ANDROID_HOME="D:\Android\Sdk"
$env:ANDROID_SDK_ROOT="D:\Android\Sdk"
$env:GRADLE_USER_HOME="D:\Android\gradle-cache"
$env:DEEPSEEK_API_KEY="your_api_key"
$env:DEEPSEEK_MODEL="deepseek-v4-pro"
$env:DEEPSEEK_PROXY="http://10.0.2.2:11304"

cd android-shell
gradle --no-daemon :app:assembleDebug
```

`DEEPSEEK_PROXY` is optional for real devices. It is useful for Android emulators when the host machine has a local proxy.

## Notes

- Do not commit API keys.
- `.env` files, Rust targets, Gradle outputs, APKs, and Android `.so` files are ignored.
- The debug APK injects the model API key through `BuildConfig`; do not distribute it.
