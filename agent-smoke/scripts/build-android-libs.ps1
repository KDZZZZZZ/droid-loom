param(
    [string]$AndroidHome = $env:ANDROID_HOME,
    [string]$ApiLevel = "35"
)

$ErrorActionPreference = "Stop"

if ([string]::IsNullOrWhiteSpace($AndroidHome)) {
    throw "ANDROID_HOME is not set. Example: `$env:ANDROID_HOME='D:\Android\Sdk'"
}

$ndkRoot = Join-Path $AndroidHome "ndk"
$ndk = Get-ChildItem -Directory $ndkRoot | Sort-Object Name -Descending | Select-Object -First 1
if ($null -eq $ndk) {
    throw "No Android NDK found under $ndkRoot"
}

$toolchain = Join-Path $ndk.FullName "toolchains\llvm\prebuilt\windows-x86_64\bin"
if (-not (Test-Path $toolchain)) {
    throw "NDK LLVM toolchain not found: $toolchain"
}

rustup target add aarch64-linux-android x86_64-linux-android

$env:CC_aarch64_linux_android = Join-Path $toolchain "aarch64-linux-android$ApiLevel-clang.cmd"
$env:AR_aarch64_linux_android = Join-Path $toolchain "llvm-ar.exe"
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $env:CC_aarch64_linux_android
$env:CARGO_TARGET_AARCH64_LINUX_ANDROID_AR = $env:AR_aarch64_linux_android
cargo build --target aarch64-linux-android

$env:CC_x86_64_linux_android = Join-Path $toolchain "x86_64-linux-android$ApiLevel-clang.cmd"
$env:AR_x86_64_linux_android = Join-Path $toolchain "llvm-ar.exe"
$env:CARGO_TARGET_X86_64_LINUX_ANDROID_LINKER = $env:CC_x86_64_linux_android
$env:CARGO_TARGET_X86_64_LINUX_ANDROID_AR = $env:AR_x86_64_linux_android
cargo build --target x86_64-linux-android

New-Item -ItemType Directory -Force android-shell\app\src\main\jniLibs\arm64-v8a | Out-Null
New-Item -ItemType Directory -Force android-shell\app\src\main\jniLibs\x86_64 | Out-Null

Copy-Item target\aarch64-linux-android\debug\libagent_smoke.so android-shell\app\src\main\jniLibs\arm64-v8a\libagent_smoke.so -Force
Copy-Item target\x86_64-linux-android\debug\libagent_smoke.so android-shell\app\src\main\jniLibs\x86_64\libagent_smoke.so -Force

Write-Host "Android native libraries copied into android-shell/app/src/main/jniLibs"
