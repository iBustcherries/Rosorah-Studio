# Run in x64 Native Tools / Developer PowerShell for Visual Studio.
param([string]$Output = 'dist/windows-development')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    if (-not $env:GSTREAMER_1_0_ROOT_MSVC_X86_64) { throw 'Install the matching GStreamer MSVC x64 runtime and development packages first.' }
    $gst = $env:GSTREAMER_1_0_ROOT_MSVC_X86_64
    $env:PATH = "$gst\bin;$env:PATH"
    $env:PKG_CONFIG_PATH = "$gst\lib\pkgconfig"
    foreach ($element in @('videoconvert','videoscale','appsink','appsrc','audioconvert','audioresample','wasapisink','mfvideosrc','decodebin','souphttpsrc')) {
        & "$gst\bin\gst-inspect-1.0.exe" $element | Out-Null
        if ($LASTEXITCODE) { throw "Missing media plugin: $element" }
    }
    cargo check --locked --target x86_64-pc-windows-msvc --workspace --all-targets
    if ($LASTEXITCODE) { throw 'Windows checks failed' }
    cargo test --locked --target x86_64-pc-windows-msvc -p auroracast-core -p rosorah-capture -p rosorah-windows
    if ($LASTEXITCODE) { throw 'Portable/native service tests failed' }
    cargo build --locked --release --target x86_64-pc-windows-msvc -p rosorah-studio -p rosorah-windows
    if ($LASTEXITCODE) { throw 'Windows build failed' }
    # A development folder: depends on the installed GStreamer runtime. Do not
    # label it a portable release/installer until runtime bundling is validated.
    if (Test-Path $Output) { throw "Output already exists: $Output" }
    New-Item -ItemType Directory -Path $Output | Out-Null
    Copy-Item 'target/x86_64-pc-windows-msvc/release/rosorah-studio.exe' $Output
    Copy-Item 'target/x86_64-pc-windows-msvc/release/rosorah-windows-probe.exe' $Output
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    cargo build --locked --release --target x86_64-pc-windows-msvc -p rosorah-camera --target-dir target/camera
    Remove-Item Env:RUSTFLAGS
    Copy-Item 'target/camera/x86_64-pc-windows-msvc/release/rosorah_camera.dll' (Join-Path $Output 'rosorah-camera.dll')
    Copy-Item 'browser' $Output -Recurse
    Copy-Item 'fonts' $Output -Recurse
    Copy-Item 'docs/windows-integration-checkpoint.md' (Join-Path $Output 'README.md')
    Copy-Item 'packaging/windows/run.cmd' $Output
    Copy-Item 'packaging/windows/setup.cmd' $Output
    Copy-Item 'docs/windows-third-party' (Join-Path $Output 'THIRD-PARTY') -Recurse
    Copy-Item 'docs/release-0.31-preview6.md' $Output
    Copy-Item 'docs/windows-preview-start.md' (Join-Path $Output 'START-HERE.md')
    Get-ChildItem $Output -File | Get-FileHash -Algorithm SHA256 | Format-Table -AutoSize | Out-String | Set-Content (Join-Path $Output 'SHA256SUMS.txt')
} finally { Pop-Location }
