param(
    [string]$StableKeyEnv = "MOBILERUN_STABLE_PREFIX_API_KEY",
    [string]$GeneralKeyEnv = "MOBILERUN_GENERAL_POOL_API_KEY",
    [string]$StablePrefixId = "stable-prefix-9f774302604cee3b",
    [string]$GeneralPrefixId = "volatile-prefix-general-pool",
    [string]$ReportPath = "android-shell\key-routing-report.json"
)

$ErrorActionPreference = "Stop"

function Test-KeySlot {
    param([string]$Name)

    $value = [Environment]::GetEnvironmentVariable($Name)
    return [ordered]@{
        env = $Name
        present = -not [string]::IsNullOrWhiteSpace($value)
        length = if ($value) { $value.Length } else { 0 }
    }
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
    $Value | ConvertTo-Json -Depth 12 | Set-Content -Encoding UTF8 -Path $Path
}

$stable = Test-KeySlot -Name $StableKeyEnv
$general = Test-KeySlot -Name $GeneralKeyEnv
$routes = @(
    [ordered]@{
        prefix_id = $StablePrefixId
        account = "stable-prefix-account"
        key_env = $StableKeyEnv
        key_present = $stable.present
    },
    [ordered]@{
        prefix_id = $GeneralPrefixId
        account = "general-pool-account"
        key_env = $GeneralKeyEnv
        key_present = $general.present
    }
)

$report = [ordered]@{
    ok = [bool]($stable.present -and $general.present -and $StableKeyEnv -ne $GeneralKeyEnv)
    stable_slot = $stable
    general_slot = $general
    separate_env_slots = [bool]($StableKeyEnv -ne $GeneralKeyEnv)
    routes = $routes
    generated_at = (Get-Date).ToString("o")
}

Write-JsonReport -Value $report -Path $ReportPath
Write-Host "Wrote sanitized key routing report to $ReportPath"
if (-not $report.ok) {
    exit 2
}
