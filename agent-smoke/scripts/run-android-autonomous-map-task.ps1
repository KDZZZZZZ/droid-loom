param(
    [string]$AdbPath = "D:\Android\Sdk\platform-tools\adb.exe",
    [string]$ApkPath = "android-shell\app\build\outputs\apk\debug\app-debug.apk",
    [string]$ApiKeyEnv = "",
    [string]$ApiBase = $env:MIMO_API_BASE,
    [string]$Model = $env:MIMO_MODEL,
    [string]$Proxy = $env:MIMO_PROXY,
    [int]$TargetToolCalls = 100,
    [int]$MinimumRequiredToolCalls = 95,
    [int]$ReactSelfCheckInterval = 10,
    [int]$MaxToolRounds = 128,
    [int]$WaitSeconds = 900,
    [string]$ReportPath = "android-shell\app-map-autonomous-report.json"
)

$ErrorActionPreference = "Stop"

function Resolve-ApiKey {
    param([string]$PreferredEnv)

    $names = @()
    if (-not [string]::IsNullOrWhiteSpace($PreferredEnv)) {
        $names += $PreferredEnv
    }
    $names += @("MIMO_API_KEY", "DEEPSEEK_API_KEY")

    foreach ($name in $names) {
        $value = [Environment]::GetEnvironmentVariable($name)
        if (-not [string]::IsNullOrWhiteSpace($value)) {
            return @{ Name = $name; Value = $value }
        }
    }
    return $null
}

function Write-JsonReport {
    param(
        [object]$Value,
        [string]$Path
    )

    $parent = Split-Path -Parent $Path
    if (-not [string]::IsNullOrWhiteSpace($parent) -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $Value | ConvertTo-Json -Depth 16 | Set-Content -Encoding UTF8 -Path $Path
}

function Invoke-Adb {
    param([string[]]$Arguments)
    & $AdbPath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "adb failed with exit code ${LASTEXITCODE}: $($Arguments -join ' ')"
    }
}

function Read-DebugOutput {
    $log = & $AdbPath logcat -d -s AgentHostService
    $lines = @($log | Select-String -Pattern "Debug tool agent_autonomous_map_task output" | ForEach-Object { $_.Line })
    $chunks = foreach ($line in $lines) {
        if ($line -match "output\[\d+\]: (.*)$") {
            $Matches[1]
        }
    }
    return ($chunks -join "")
}

function Wait-DebugOutput {
    param([int]$TimeoutSeconds)

    $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
    do {
        Start-Sleep -Seconds 5
        $raw = Read-DebugOutput
        if (-not [string]::IsNullOrWhiteSpace($raw)) {
            return $raw
        }
    } while ((Get-Date) -lt $deadline)
    return ""
}

$key = Resolve-ApiKey -PreferredEnv $ApiKeyEnv
if ($null -eq $key) {
    $result = [ordered]@{
        ok = $false
        skipped = $true
        reason = "Missing MIMO_API_KEY or DEEPSEEK_API_KEY"
    }
    Write-JsonReport -Value $result -Path $ReportPath
    Write-Host "Skipped provider autonomous map task: missing API key env."
    exit 2
}

if (-not (Test-Path -LiteralPath $AdbPath)) {
    throw "adb not found: $AdbPath"
}

if ([string]::IsNullOrWhiteSpace($ApiBase)) {
    $ApiBase = "https://api.xiaomimimo.com/v1"
}
if ([string]::IsNullOrWhiteSpace($Model)) {
    $Model = "mimo-v2.5-pro"
}

Invoke-Adb @("devices") | Out-Null
Invoke-Adb @("install", "-r", $ApkPath) | Out-Null
Invoke-Adb @("shell", "pm", "grant", "com.example.agentsmoke", "android.permission.POST_NOTIFICATIONS") 2>$null
Invoke-Adb @("shell", "appops", "set", "com.example.agentsmoke", "SYSTEM_ALERT_WINDOW", "allow")
Invoke-Adb @("shell", "settings", "put", "secure", "enabled_accessibility_services", "com.example.agentsmoke/com.example.agentsmoke.AgentAccessibilityService")
Invoke-Adb @("shell", "settings", "put", "secure", "accessibility_enabled", "1")
Invoke-Adb @("logcat", "-c")

$debugInput = @{
    target_tool_calls = $TargetToolCalls
    minimum_required_tool_calls = $MinimumRequiredToolCalls
    react_self_check_interval = $ReactSelfCheckInterval
    max_tool_rounds = $MaxToolRounds
} | ConvertTo-Json -Compress
$debugInputBase64 = [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($debugInput))

$startArgs = @(
    "shell", "am", "start",
    "-a", "android.intent.action.MAIN",
    "-c", "android.intent.category.LAUNCHER",
    "-n", "com.example.agentsmoke/.MainActivity",
    "--es", "debug_tool", "agent_autonomous_map_task",
    "--es", "debug_input_base64", $debugInputBase64,
    "--es", "mimo_api_base", $ApiBase,
    "--es", "mimo_api_key", $key.Value,
    "--es", "mimo_model", $Model,
    "--ei", "agent_max_tool_rounds", "$MaxToolRounds"
)
if (-not [string]::IsNullOrWhiteSpace($Proxy)) {
    $startArgs += @("--es", "mimo_proxy", $Proxy)
}

Invoke-Adb $startArgs | Out-Null
$rawOutput = Wait-DebugOutput -TimeoutSeconds $WaitSeconds
$parsed = $null
try {
    $parsed = $rawOutput | ConvertFrom-Json -ErrorAction Stop
} catch {
    $parsed = [ordered]@{
        ok = $false
        error = "Could not parse agent_autonomous_map_task log output"
        raw_output_length = $rawOutput.Length
    }
}

$validation = [ordered]@{
    target_tool_calls = $TargetToolCalls
    minimum_required_tool_calls = $MinimumRequiredToolCalls
    actual_tool_calls = if ($parsed -and $parsed.actual_tool_calls -ne $null) { [int]$parsed.actual_tool_calls } else { 0 }
    provider_tool_traces = if ($parsed -and $parsed.provider_tool_traces -ne $null) { [int]$parsed.provider_tool_traces } else { 0 }
    macro_tool_traces = if ($parsed -and $parsed.macro_tool_traces -ne $null) { [int]$parsed.macro_tool_traces } else { -1 }
    meaningful_phone_task = if ($parsed -and $parsed.meaningful_phone_task -ne $null) { [bool]$parsed.meaningful_phone_task } else { $false }
    navigation_tool_calls = if ($parsed -and $parsed.navigation_tool_calls -ne $null) { [int]$parsed.navigation_tool_calls } else { 0 }
    observation_tool_calls = if ($parsed -and $parsed.observation_tool_calls -ne $null) { [int]$parsed.observation_tool_calls } else { 0 }
    artifact_tool_calls = if ($parsed -and $parsed.artifact_tool_calls -ne $null) { [int]$parsed.artifact_tool_calls } else { 0 }
    device_read_tool_calls = if ($parsed -and $parsed.device_read_tool_calls -ne $null) { [int]$parsed.device_read_tool_calls } else { 0 }
    successful_navigation_tool_calls = if ($parsed -and $parsed.successful_navigation_tool_calls -ne $null) { [int]$parsed.successful_navigation_tool_calls } else { 0 }
    successful_observation_tool_calls = if ($parsed -and $parsed.successful_observation_tool_calls -ne $null) { [int]$parsed.successful_observation_tool_calls } else { 0 }
    successful_artifact_tool_calls = if ($parsed -and $parsed.successful_artifact_tool_calls -ne $null) { [int]$parsed.successful_artifact_tool_calls } else { 0 }
    successful_device_read_tool_calls = if ($parsed -and $parsed.successful_device_read_tool_calls -ne $null) { [int]$parsed.successful_device_read_tool_calls } else { 0 }
    unique_tool_count = if ($parsed -and $parsed.unique_tool_count -ne $null) { [int]$parsed.unique_tool_count } else { 0 }
    argument_signature_count = if ($parsed -and $parsed.argument_signature_count -ne $null) { [int]$parsed.argument_signature_count } else { 0 }
    task_subgoal_coverage_count = if ($parsed -and $parsed.task_subgoal_coverage_count -ne $null) { [int]$parsed.task_subgoal_coverage_count } else { 0 }
    repeated_fixed_action_loop = if ($parsed -and $parsed.repeated_fixed_action_loop -ne $null) { [bool]$parsed.repeated_fixed_action_loop } else { $true }
}
$validation["ok"] = [bool](
    $parsed.ok `
    -and $validation.actual_tool_calls -ge $MinimumRequiredToolCalls `
    -and $validation.provider_tool_traces -ge $MinimumRequiredToolCalls `
    -and $validation.macro_tool_traces -eq 0 `
    -and $validation.meaningful_phone_task `
    -and $validation.navigation_tool_calls -ge 10 `
    -and $validation.observation_tool_calls -ge 30 `
    -and $validation.artifact_tool_calls -ge 6 `
    -and $validation.device_read_tool_calls -ge 4 `
    -and $validation.successful_navigation_tool_calls -ge 10 `
    -and $validation.successful_observation_tool_calls -ge 20 `
    -and $validation.successful_artifact_tool_calls -ge 6 `
    -and $validation.successful_device_read_tool_calls -ge 4 `
    -and $validation.unique_tool_count -ge 10 `
    -and $validation.argument_signature_count -ge 30 `
    -and $validation.task_subgoal_coverage_count -ge 8 `
    -and -not $validation.repeated_fixed_action_loop
)

$report = [ordered]@{
    ok = [bool]$validation.ok
    key_env = $key.Name
    api_base = $ApiBase
    model = $Model
    target_tool_calls = $TargetToolCalls
    minimum_required_tool_calls = $MinimumRequiredToolCalls
    react_self_check_interval = $ReactSelfCheckInterval
    max_tool_rounds = $MaxToolRounds
    generated_at = (Get-Date).ToString("o")
    validation = $validation
    result = $parsed
}
Write-JsonReport -Value $report -Path $ReportPath
Write-Host "Wrote sanitized autonomous map task report to $ReportPath"
if (-not $report.ok) {
    exit 2
}
