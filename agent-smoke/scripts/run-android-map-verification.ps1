param(
    [string]$AdbPath = "D:\Android\Sdk\platform-tools\adb.exe",
    [string]$ApkPath = "android-shell\app\build\outputs\apk\debug\app-debug.apk",
    [int]$GoalCycles = 6,
    [int]$GoalWaitSeconds = 60,
    [int]$SmokeCycles = 20,
    [int]$SmokeWaitSeconds = 60,
    [string]$ReportPath = "android-shell\app-map-verification-report.json"
)

$ErrorActionPreference = "Stop"

function Write-JsonReport {
    param(
        [object]$Value,
        [string]$Path
    )

    $parent = Split-Path -Parent $Path
    if (-not [string]::IsNullOrWhiteSpace($parent) -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $Value | ConvertTo-Json -Depth 24 | Set-Content -Encoding UTF8 -Path $Path
}

function Invoke-Adb {
    param([string[]]$Arguments)

    & $AdbPath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "adb failed with exit code ${LASTEXITCODE}: $($Arguments -join ' ')"
    }
}

function ConvertTo-Base64Json {
    param([hashtable]$Value)

    $json = $Value | ConvertTo-Json -Compress
    return [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($json))
}

function Enable-AgentPermissions {
    Invoke-Adb @("shell", "appops", "set", "com.example.agentsmoke", "SYSTEM_ALERT_WINDOW", "allow") | Out-Null
    Invoke-Adb @("shell", "settings", "put", "secure", "enabled_accessibility_services", "com.example.agentsmoke/com.example.agentsmoke.AgentAccessibilityService") | Out-Null
    Invoke-Adb @("shell", "settings", "put", "secure", "accessibility_enabled", "1") | Out-Null
}

function Read-DebugToolOutput {
    param([string]$ToolName)

    $log = & $AdbPath logcat -d -s AgentHostService
    $lines = @($log | Select-String -Pattern "Debug tool $ToolName output" | ForEach-Object { $_.Line })
    $chunks = foreach ($line in $lines) {
        if ($line -match "output\[\d+\]: (.*)$") {
            $Matches[1]
        }
    }
    $raw = ($chunks -join "")
    if ([string]::IsNullOrWhiteSpace($raw)) {
        return [ordered]@{
            ok = $false
            error = "Missing debug output for $ToolName"
        }
    }
    try {
        return $raw | ConvertFrom-Json -ErrorAction Stop
    } catch {
        return [ordered]@{
            ok = $false
            error = "Could not parse debug output for $ToolName"
            raw_output_length = $raw.Length
        }
    }
}

function Invoke-DebugTool {
    param(
        [string]$ToolName,
        [hashtable]$ToolInput,
        [int]$WaitSeconds,
        [bool]$ResetApp = $true
    )

    if ($ResetApp) {
        Invoke-Adb @("shell", "am", "force-stop", "com.example.agentsmoke") | Out-Null
    }
    Enable-AgentPermissions
    Invoke-Adb @("logcat", "-c") | Out-Null

    $encodedInput = ConvertTo-Base64Json -Value $ToolInput
    Invoke-Adb @(
        "shell", "am", "start",
        "-a", "android.intent.action.MAIN",
        "-c", "android.intent.category.LAUNCHER",
        "-n", "com.example.agentsmoke/.MainActivity",
        "--es", "debug_tool", $ToolName,
        "--es", "debug_input_base64", $encodedInput
    ) | Out-Null
    Start-Sleep -Seconds $WaitSeconds
    return Read-DebugToolOutput -ToolName $ToolName
}

function Test-GoalTask {
    param([object]$Goal)

    $map = $Goal.map
    $localView = $Goal.final_local_view
    $semanticSearch = $Goal.final_semantic_search
    return [ordered]@{
        ok = [bool](
            $Goal.ok `
            -and $Goal.completed `
            -and $Goal.steps -ge 100 `
            -and $Goal.semantic_searches -gt 0 `
            -and $Goal.app_switches -gt 0 `
            -and $Goal.planned_paths -gt 0 `
            -and $Goal.map_reuse_hits -gt 0 `
            -and $Goal.local_map_tokens -gt 0 `
            -and $Goal.full_map_tokens -gt $Goal.local_map_tokens `
            -and $Goal.token_savings_percent -gt 0 `
            -and $Goal.probability_graph.tool_call_events -ge 100 `
            -and $Goal.tool_trace_count -eq $Goal.steps `
            -and $Goal.assistant_tool_call_messages -eq $Goal.steps `
            -and $Goal.tool_result_messages -eq $Goal.steps `
            -and $Goal.session_message_count -eq (2 * $Goal.steps + 2) `
            -and $localView.ok `
            -and $localView.node_count -gt 0 `
            -and $localView.transition_count -gt 0 `
            -and $localView.candidate_action_count -gt 0 `
            -and $semanticSearch.ok `
            -and $semanticSearch.hit_count -gt 0 `
            -and $map.page_count -gt 0 `
            -and $map.transition_count -gt 0
        )
        completed = [bool]$Goal.completed
        steps = [int]$Goal.steps
        semantic_searches = [int]$Goal.semantic_searches
        app_switches = [int]$Goal.app_switches
        planned_paths = [int]$Goal.planned_paths
        map_reuse_hits = [int]$Goal.map_reuse_hits
        local_map_tokens = [int]$Goal.local_map_tokens
        full_map_tokens = [int]$Goal.full_map_tokens
        token_savings_percent = [double]$Goal.token_savings_percent
        page_count = [int]$map.page_count
        transition_count = [int]$map.transition_count
        probability_tool_call_events = [int]$Goal.probability_graph.tool_call_events
        tool_trace_count = [int]$Goal.tool_trace_count
        assistant_tool_call_messages = [int]$Goal.assistant_tool_call_messages
        tool_result_messages = [int]$Goal.tool_result_messages
        session_message_count = [int]$Goal.session_message_count
        local_view_node_count = [int]$localView.node_count
        local_view_transition_count = [int]$localView.transition_count
        candidate_action_count = [int]$localView.candidate_action_count
        dangerous_action_count = [int]$localView.dangerous_action_count
        semantic_search_hit_count = [int]$semanticSearch.hit_count
    }
}

function Test-StaleProbe {
    param([object]$Probe)

    return [ordered]@{
        ok = [bool](
            $Probe.ok `
            -and $Probe.pre_stale_plan.found `
            -and $Probe.pre_stale_plan.path_length -gt 0 `
            -and $Probe.stale_transition_count -gt 0 `
            -and -not $Probe.post_stale_plan.found
        )
        pre_stale_path_length = [int]$Probe.pre_stale_plan.path_length
        stale_transition_count = [int]$Probe.stale_transition_count
        post_stale_found = [bool]$Probe.post_stale_plan.found
    }
}

function Test-StalePageRefreshProbe {
    param([object]$Probe)

    return [ordered]@{
        ok = [bool](
            $Probe.ok `
            -and $Probe.first.ok `
            -and $Probe.mark.ok `
            -and $Probe.refreshed.ok `
            -and $Probe.same_page `
            -and $Probe.replaced_stale_node `
            -and ($Probe.first.page_id -eq $Probe.refreshed.page_id) `
            -and $Probe.page_count -gt 0
        )
        first_page_id = [string]$Probe.first.page_id
        same_page = [bool]$Probe.same_page
        replaced_stale_node = [bool]$Probe.replaced_stale_node
        page_count = [int]$Probe.page_count
    }
}

function Test-SmokeTask {
    param([object]$Smoke)

    $map = $Smoke.map
    return [ordered]@{
        ok = [bool](
            $Smoke.ok `
            -and $Smoke.completed `
            -and $Smoke.steps -ge 100 `
            -and $Smoke.app_switches -gt 0 `
            -and $Smoke.local_map_tokens -gt 0 `
            -and $Smoke.full_map_tokens -gt $Smoke.local_map_tokens `
            -and $Smoke.token_savings_percent -gt 0 `
            -and $map.page_count -gt 0 `
            -and $map.transition_count -gt 0
        )
        completed = [bool]$Smoke.completed
        steps = [int]$Smoke.steps
        app_switches = [int]$Smoke.app_switches
        local_map_tokens = [int]$Smoke.local_map_tokens
        full_map_tokens = [int]$Smoke.full_map_tokens
        token_savings_percent = [double]$Smoke.token_savings_percent
        page_count = [int]$map.page_count
        transition_count = [int]$map.transition_count
    }
}

if (-not (Test-Path -LiteralPath $AdbPath)) {
    throw "adb not found: $AdbPath"
}
if (-not (Test-Path -LiteralPath $ApkPath)) {
    throw "APK not found: $ApkPath"
}

Invoke-Adb @("devices") | Out-Null
Invoke-Adb @("install", "-r", $ApkPath) | Out-Null
try {
    Invoke-Adb @("shell", "pm", "grant", "com.example.agentsmoke", "android.permission.POST_NOTIFICATIONS") | Out-Null
} catch {
    # Old emulator images or already-granted permissions can make this noisy; it is not required.
}

$goal = Invoke-DebugTool `
    -ToolName "android_map_goal_task" `
    -ToolInput @{ cycles = $GoalCycles; task = "device readiness brief" } `
    -WaitSeconds $GoalWaitSeconds

$staleProbe = Invoke-DebugTool `
    -ToolName "android_map_stale_path_probe" `
    -ToolInput @{} `
    -WaitSeconds 3 `
    -ResetApp $false

$stalePageRefreshProbe = Invoke-DebugTool `
    -ToolName "android_map_stale_page_refresh_probe" `
    -ToolInput @{} `
    -WaitSeconds 3 `
    -ResetApp $false

$smoke = Invoke-DebugTool `
    -ToolName "android_map_long_task_smoke" `
    -ToolInput @{ cycles = $SmokeCycles } `
    -WaitSeconds $SmokeWaitSeconds

$goalValidation = Test-GoalTask -Goal $goal
$staleValidation = Test-StaleProbe -Probe $staleProbe
$stalePageRefreshValidation = Test-StalePageRefreshProbe -Probe $stalePageRefreshProbe
$smokeValidation = Test-SmokeTask -Smoke $smoke

$report = [ordered]@{
    ok = [bool]($goalValidation.ok -and $staleValidation.ok -and $stalePageRefreshValidation.ok -and $smokeValidation.ok)
    generated_at = (Get-Date).ToString("o")
    emulator = "adb"
    validation = [ordered]@{
        goal_task = $goalValidation
        stale_path_probe = $staleValidation
        stale_page_refresh_probe = $stalePageRefreshValidation
        smoke_task = $smokeValidation
    }
    goal_task = $goal
    stale_path_probe = $staleProbe
    stale_page_refresh_probe = $stalePageRefreshProbe
    smoke_task = $smoke
}

Write-JsonReport -Value $report -Path $ReportPath
Write-Host "Wrote Android app-map verification report to $ReportPath"
if (-not $report.ok) {
    exit 2
}
