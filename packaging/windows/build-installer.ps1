# Run after build.ps1 in x64 Developer PowerShell. Requires NSIS 3 on PATH.
param([string]$Bundle = 'dist/windows-development', [string]$Output = 'dist/Rosorah-Studio-0.32.0-preview.10-Setup.exe')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    $bundlePath = (Resolve-Path $Bundle).Path
    if (-not (Test-Path (Join-Path $bundlePath 'rosorah-studio.exe'))) { throw 'Build the Windows application bundle first.' }
    rustc --edition 2024 --target x86_64-pc-windows-msvc -C opt-level=s -C strip=symbols (Join-Path $PSScriptRoot 'launcher.rs') -o (Join-Path $bundlePath 'Rosorah.exe')
    if ($LASTEXITCODE) { throw 'Launcher compilation failed' }
    Copy-Item (Join-Path $PSScriptRoot 'setup.cmd') $bundlePath -Force
    Copy-Item (Join-Path $PSScriptRoot 'installed-readme.md') (Join-Path $bundlePath 'START-HERE.md') -Force
    Remove-Item (Join-Path $bundlePath 'SHA256SUMS.txt') -ErrorAction SilentlyContinue
    $lines = @()
    Get-ChildItem $bundlePath -Recurse -File | ForEach-Object {
        $relative = $_.FullName.Substring($bundlePath.Length + 1)
        $lines += 'Delete "$INSTDIR\' + $relative + '"'
    }
    Get-ChildItem $bundlePath -Recurse -Directory | Sort-Object { $_.FullName.Length } -Descending | ForEach-Object {
        $relative = $_.FullName.Substring($bundlePath.Length + 1)
        $lines += 'RMDir "$INSTDIR\' + $relative + '"'
    }
    $lines | Set-Content -Encoding utf8 (Join-Path $PSScriptRoot 'uninstall-files.nsh')
    $outputPath = [IO.Path]::GetFullPath($Output)
    New-Item -ItemType Directory -Force (Split-Path $outputPath) | Out-Null
    & makensis /WX "/DPAYLOAD=$bundlePath" "/DOUTPUT=$outputPath" (Join-Path $PSScriptRoot 'installer.nsi')
    if ($LASTEXITCODE) { throw 'Installer compilation failed' }
} finally { Pop-Location }
