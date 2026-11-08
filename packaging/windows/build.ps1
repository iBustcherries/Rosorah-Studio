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
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue (Join-Path $Output 'browser\__pycache__')
    Get-ChildItem (Join-Path $Output 'browser') -Filter 'test_*' | Remove-Item
    # Chromium for chat, web panels and browser sources, so nothing is downloaded on first use.
    $py = Get-Command py -ErrorAction SilentlyContinue
    if ($py) { & $py.Source -3.13 'packaging/bundle-browser-runtime.py' windows (Join-Path $Output 'browser-runtime') }
    else { & python 'packaging/bundle-browser-runtime.py' windows (Join-Path $Output 'browser-runtime') }
    if ($LASTEXITCODE) { throw 'Browser runtime bundling failed' }
    Copy-Item 'fonts' $Output -Recurse
    Copy-Item 'packaging/windows/setup.cmd', 'packaging/windows/setup.ps1', 'packaging/windows/rosorah.ico' $Output
    # Twitch/YouTube sign-in registrations; kept out of git (see .gitignore).
    if (Test-Path 'packaging/accounts-defaults.json') { Copy-Item 'packaging/accounts-defaults.json' $Output }
    else { Write-Warning 'packaging/accounts-defaults.json is missing, so Twitch and YouTube sign-in will be unavailable in this build.' }
    # GStreamer's runtime installer, bundled so setup never depends on its download server.
    $msi = Join-Path $root 'target/gstreamer-1.26.10.msi'
    if (-not (Test-Path $msi) -or (Get-FileHash $msi -Algorithm SHA256).Hash -ne 'A863BF3FAA49E9F33BD3CC42967B473482D4DC98655ED95CBA1AC59F26FB0CFB') {
        & curl.exe -sSfL --retry 5 -o $msi 'https://gstreamer.freedesktop.org/data/pkg/windows/1.26.10/msvc/gstreamer-1.0-msvc-x86_64-1.26.10.msi'
        if ((Get-FileHash $msi -Algorithm SHA256).Hash -ne 'A863BF3FAA49E9F33BD3CC42967B473482D4DC98655ED95CBA1AC59F26FB0CFB') { throw 'GStreamer runtime download does not match its checksum' }
    }
    New-Item -ItemType Directory -Path (Join-Path $Output 'runtime') | Out-Null
    Copy-Item $msi (Join-Path $Output 'runtime/gstreamer-1.26.10.msi')
    Copy-Item 'docs/windows-third-party' (Join-Path $Output 'THIRD-PARTY') -Recurse
    Copy-Item 'packaging/windows/installed-readme.md' (Join-Path $Output 'START-HERE.md')
    Get-ChildItem $Output -File | Get-FileHash -Algorithm SHA256 | Format-Table -AutoSize | Out-String | Set-Content (Join-Path $Output 'SHA256SUMS.txt')
} finally { Pop-Location }
