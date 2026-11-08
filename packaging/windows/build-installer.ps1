# Run after build.ps1 in x64 Developer PowerShell. Requires NSIS 3 on PATH.
param([string]$Bundle = 'dist/windows-development', [string]$Output = '')
$ErrorActionPreference = 'Stop'
$root = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
Push-Location $root
try {
    $bundlePath = (Resolve-Path $Bundle).Path
    if (-not (Test-Path (Join-Path $bundlePath 'rosorah-studio.exe'))) { throw 'Build the Windows application bundle first.' }
    # The launcher carries the app icon (rosorah.rc), compiled with the Windows SDK's rc.exe.
    $rc = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\rc.exe" | Sort-Object FullName | Select-Object -Last 1
    $res = Join-Path $root 'target/rosorah.res'
    New-Item -ItemType Directory -Force (Split-Path $res) | Out-Null
    & $rc.FullName /nologo /fo $res (Join-Path $PSScriptRoot 'rosorah.rc')
    if ($LASTEXITCODE) { throw 'Icon resource compilation failed' }
    rustc --edition 2024 --target x86_64-pc-windows-msvc -C opt-level=s -C strip=symbols -C "link-arg=$res" (Join-Path $PSScriptRoot 'launcher.rs') -o (Join-Path $bundlePath 'Rosorah.exe')
    if ($LASTEXITCODE) { throw 'Launcher compilation failed' }
    Remove-Item (Join-Path $bundlePath 'Rosorah.pdb') -ErrorAction SilentlyContinue
    # cmd.exe misreads batch files without Windows (CRLF) line endings.
    $cmd = (Get-Content -Raw (Join-Path $PSScriptRoot 'setup.cmd')) -replace "`r?`n", "`r`n"
    [IO.File]::WriteAllText((Join-Path $bundlePath 'setup.cmd'), $cmd)
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
    $version = (Select-String -Path 'Cargo.toml' -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value
    if (-not $Output) { $Output = "dist/Rosorah-Studio-$version-Setup.exe" }
    $outputPath = [IO.Path]::GetFullPath($Output)
    New-Item -ItemType Directory -Force (Split-Path $outputPath) | Out-Null
    & makensis /WX "/DPAYLOAD=$bundlePath" "/DOUTPUT=$outputPath" "/DVERSION=$version" (Join-Path $PSScriptRoot 'installer.nsi')
    if ($LASTEXITCODE) { throw 'Installer compilation failed' }
} finally { Pop-Location }
