# agent-smoke

Minimal PoC for an Android phone-using agent with the shared `agent-core` crate.

Validated chain:

```text
Android Kotlin App
  -> UniFFI Kotlin bindings
  -> Rust UniFFI shell (`agent-smoke/src/lib.rs`)
  -> shared Rust agent-core (`crates/agent-core`)
  -> MiMo/OpenAI-compatible Chat Completions API
  -> model tool calls
  -> Rust PlatformToolHost callback
  -> Android platform tools
```

## What is implemented

- `AgentCore` is the Android-facing UniFFI shell.
- Shared message/content/provider mapping lives in `crates/agent-core`.
- Rust HTTP client calls MiMo OpenAI-compatible Chat Completions through the shared provider request path.
- Tool schemas can be registered in Rust.
- `prompt_with_tools()` runs a minimal tool-call loop with up to 8 tool-call rounds for local phone-using smoke tests.
- Kotlin implements `PlatformToolHost` and executes Android APIs.
- Android shell now has a foreground `AgentHostService`, a global floating ball, and an
  `AgentAccessibilityService` bridge for cross-app phone-using tools.
- Android demo registers:
  - `android_device_info`
  - `android_battery`
  - `android_show_toast`
  - `android_set_clipboard`
  - `android_open_settings`
  - `android_ui_state`
  - `android_click_text`
  - `android_type_text`
  - `android_global_action`

## Desktop smoke test

```powershell
$env:MIMO_API_KEY="your_api_key"
$env:MIMO_MODEL="mimo-v2.5-pro"
$env:MIMO_API_BASE="https://api.xiaomimimo.com/v1"
cargo run -p agent-smoke -- "Say hello in one short sentence."
```

## Generate Kotlin bindings

```powershell
cargo build -p agent-smoke
uniffi-bindgen generate --library target\debug\agent_smoke.dll --language kotlin --out-dir agent-smoke\bindings\kotlin
```

Run these two commands from the repository root. The repository is now a Cargo workspace, so the debug DLL is written to the root `target` directory.

## Build Android native libraries

```powershell
$env:ANDROID_HOME="D:\Android\Sdk"
cd agent-smoke
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
$env:MIMO_API_KEY="your_api_key"
$env:MIMO_MODEL="mimo-v2.5-pro"
$env:MIMO_API_BASE="https://api.xiaomimimo.com/v1"
$env:MIMO_PROXY="http://10.0.2.2:11304"

cd agent-smoke\android-shell
gradle --no-daemon :app:assembleDebug
```

`MIMO_PROXY` is optional for real devices. It is useful for Android emulators when the host machine has a local proxy.

## Android host mode

The Android app starts `AgentHostService` as the real runtime. `MainActivity` only starts the
service and opens permission screens. For cross-app operation, grant overlay and accessibility:

```powershell
adb shell appops set com.example.agentsmoke SYSTEM_ALERT_WINDOW allow
adb shell settings put secure enabled_accessibility_services com.example.agentsmoke/com.example.agentsmoke.AgentAccessibilityService
adb shell settings put secure accessibility_enabled 1
```

After overlay permission is active, the foreground service shows a floating `AI` button. Tap it
to send commands while Settings or another app is in front.
`android_ui_state` reads all available accessibility windows first, so the floating ball can stay
visible without hiding the underlying app tree from the agent.

The Android host also exposes app-map tools:

- `android_map_observe`
- `android_map_view`
- `android_map_search`
- `android_map_plan_path`
- `android_map_forget`
- `android_map_click_text`
- `android_map_type_text`
- `android_map_long_task_smoke`
- `android_map_goal_task`

Local debug smoke for the real AccessibilityService map path:

```powershell
D:\Android\Sdk\platform-tools\adb.exe shell am start `
  -a android.intent.action.MAIN `
  -c android.intent.category.LAUNCHER `
  -n com.example.agentsmoke/.MainActivity `
  --es debug_tool android_map_long_task_smoke
```

Current verification-script output: `completed=true`, `steps=100`, `app_switches=50`,
`page_count=7`, `transition_count=54`, `local_map_tokens=32954`,
`full_map_tokens=85604`, `token_savings_percent=61.5`.

Goal-driven local debug task for the real AccessibilityService map path:

```powershell
D:\Android\Sdk\platform-tools\adb.exe shell am start `
  -a android.intent.action.MAIN `
  -c android.intent.category.LAUNCHER `
  -n com.example.agentsmoke/.MainActivity `
  --es debug_tool android_map_goal_task
```

Current verification-script output: `completed=true`, `steps=111`, `app_switches=18`,
`semantic_searches=24`, `planned_paths=7`, `map_reuse_hits=15`,
`page_count=6`, `transition_count=19`, `local_map_tokens=10281`,
`full_map_tokens=15608`, `token_savings_percent=34.1`,
`session_message_count=224`, `assistant_tool_call_messages=111`,
`tool_result_messages=111`, `tool_trace_count=111`,
`probability_graph.tool_call_events=111`, `probability_graph.transition_count=110`.
The same report validates a final local view with `node_count=6`,
`transition_count=19`, `candidate_action_count=5`, and semantic search
`hit_count=4`.

The verification script fails if `completed=true` is reported without non-zero
map pages, transitions, path reuse, local-view nodes, local-view candidate
actions, semantic-search hits, and token savings. This catches startup races
where the AccessibilityService is enabled but not yet bound after a force-stop.

The same report verifies stale path maintenance: `pre_stale_path_length=1`,
`stale_transition_count=1`, `post_stale_found=false`.
It also verifies stale page refresh: `same_page=true`,
`replaced_stale_node=true`.

To reproduce both no-key Android map checks and write a sanitized report:

```powershell
cd agent-smoke
.\scripts\run-android-map-verification.ps1
```

The report is written to `agent-smoke/android-shell/app-map-verification-report.json`.

Provider-driven long map task entry:

```powershell
D:\Android\Sdk\platform-tools\adb.exe shell am start `
  -a android.intent.action.MAIN `
  -c android.intent.category.LAUNCHER `
  -n com.example.agentsmoke/.MainActivity `
  --es debug_tool agent_autonomous_map_task `
  --ei agent_max_tool_rounds 32
```

`agent_autonomous_map_task` requires a runtime provider key. It now runs one
single ReAct graph with a loop node (`start -> react_loop -> final_summary`), and
the acceptance target is at least 100 provider-visible primitive phone tool calls.
Macro/debug tools such as
`android_map_goal_task` and `android_map_long_task_smoke` are not registered for this
run, so the model cannot satisfy the goal with one wrapper call. The report is only
`ok=true` when `actual_tool_calls >= 100`, `macro_tool_traces == 0`, and the trace
contains navigation, observation, device-read, and artifact tools.

For reproducible provider runs, use the script below. It reads the key from `MIMO_API_KEY`
or `DEEPSEEK_API_KEY`, passes it as a runtime-only debug extra, and writes a sanitized JSON
report without the key. Structured debug input is sent with `debug_input_base64` so adb shell
quoting cannot corrupt JSON arguments:

```powershell
$env:MIMO_API_KEY="<runtime key>"
cd agent-smoke
.\scripts\run-android-autonomous-map-task.ps1
```

The report is written to `agent-smoke/android-shell/app-map-autonomous-report.json`.
Useful knobs are `-TargetToolCalls 100`, `-MinimumRequiredToolCalls 95`,
`-ReactSelfCheckInterval 10`, `-MaxToolRounds 128`, and `-WaitSeconds 900`.

Current provider-run output: `graph=single_graph_loop`,
`nodes=start->react_loop->final_summary`, `actual_tool_calls=96/100`,
`provider_tool_traces=96`, `macro_tool_traces=0`,
`repeated_fixed_action_loop=false`, `task_subgoal_coverage_count=10/10`,
`argument_signature_count=61`, `unique_tool_count=13`, `total_tokens=3608111`.
Tool counts: `android_device_info=2`, `android_battery=2`, `android_open_agent=3`,
`android_ui_state=20`, `android_map_observe=17`, `android_map_view=2`,
`android_set_clipboard=3`, `android_show_toast=6`, `android_open_settings=3`,
`android_map_search=10`, `android_map_plan_path=3`, `android_global_action=6`,
`android_click_text=19`.

For local simulator smoke tests, the debug app can also read runtime-only intent extras:

```powershell
D:\Android\Sdk\platform-tools\adb.exe shell am start `
  -n com.example.agentsmoke/.MainActivity `
  --es mimo_api_key "<your_api_key>" `
  --es agent_prompt "Run android_device_info, android_battery, android_set_clipboard, and android_show_toast. Summarize in Chinese."
```

Do not use runtime key extras outside local debug runs. The app never prints the key in the UI.

## Notes

- Do not commit API keys.
- `.env` files, Rust targets, Gradle outputs, APKs, and Android `.so` files are ignored.
- The debug APK can receive the model API key through `BuildConfig` or a debug-only intent extra; do not distribute keyed builds or command history containing keys.
- `DEEPSEEK_*` environment variables are still accepted as compatibility fallback, but `MIMO_*` is the preferred configuration.
