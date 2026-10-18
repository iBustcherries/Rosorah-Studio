# Installs Rosorah's Windows dependencies when they are missing.
# Every download is checksum- or signature-verified before it runs.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$runtime = Join-Path $PSScriptRoot 'runtime'
New-Item -ItemType Directory -Force $runtime | Out-Null

function Test-Download([string]$Path, [string]$Sha256, [scriptblock]$Check) {
    if (-not (Test-Path $Path)) { return $false }
    if ($Sha256) { return (Get-FileHash $Path -Algorithm SHA256).Hash -eq $Sha256 }
    return [bool](& $Check $Path)
}

# Resumes interrupted downloads and retries busy servers. A finished file that fails
# verification is deleted and fetched again; a partial one is resumed.
function Get-Verified([string]$Url, [string]$Path, [string]$Sha256, [scriptblock]$Check) {
    $curl = Join-Path $env:SystemRoot 'System32\curl.exe'
    for ($attempt = 1; $attempt -le 6; $attempt++) {
        if (Test-Download $Path $Sha256 $Check) { return }
        if (Test-Path $curl) {
            # curl's own messages must not stop the script; its exit code decides.
            $ErrorActionPreference = 'Continue'
            & $curl -sS -fL --retry 8 --retry-delay 3 --connect-timeout 30 -C - -o $Path $Url
            $complete = $LASTEXITCODE -eq 0
            $ErrorActionPreference = 'Stop'
        } else {
            try { Invoke-WebRequest -TimeoutSec 300 $Url -OutFile $Path; $complete = $true } catch { $complete = $false }
        }
        if (Test-Download $Path $Sha256 $Check) { return }
        if ($complete -and (Test-Path $Path)) { Remove-Item $Path -Force }
        Write-Output "  Download incomplete (attempt $attempt of 6); retrying..."
        Start-Sleep -Seconds ([Math]::Min(30, 5 * $attempt))
    }
    throw "Could not download a verified copy of $(Split-Path $Url -Leaf). Check your internet connection and run the installer again."
}

function Test-Signer([string]$Path, [string]$Organization) {
    $s = Get-AuthenticodeSignature $Path
    return $s.Status -eq 'Valid' -and $s.SignerCertificate.Subject -match [regex]::Escape($Organization)
}

# 1. Microsoft Visual C++ Runtime
if ((Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64' -ErrorAction SilentlyContinue).Installed -eq 1) {
    Write-Output 'Microsoft Visual C++ Runtime: already installed'
} else {
    Write-Output 'Microsoft Visual C++ Runtime: installing...'
    $p = Join-Path $runtime 'vc_redist.x64.exe'
    Get-Verified 'https://aka.ms/vs/17/release/vc_redist.x64.exe' $p '' { param($f) Test-Signer $f 'O=Microsoft Corporation' }
    $r = Start-Process $p -ArgumentList '/install', '/passive', '/norestart' -Wait -PassThru
    if ($r.ExitCode -notin @(0, 3010, 1638)) { throw "Microsoft runtime install failed: $($r.ExitCode)" }
}

# 2. GStreamer (bundled with the installer; downloaded only if the bundled copy is missing)
$gst = 'C:\gstreamer\1.0\msvc_x86_64\bin\gst-inspect-1.0.exe'
if ((Test-Path $gst) -and ((& $gst --version) -match '1\.26\.10')) {
    Write-Output 'GStreamer 1.26.10: already installed'
} else {
    Write-Output 'GStreamer 1.26.10: installing...'
    $p = Join-Path $runtime 'gstreamer-1.26.10.msi'
    Get-Verified 'https://gstreamer.freedesktop.org/data/pkg/windows/1.26.10/msvc/gstreamer-1.0-msvc-x86_64-1.26.10.msi' $p 'A863BF3FAA49E9F33BD3CC42967B473482D4DC98655ED95CBA1AC59F26FB0CFB' $null
    $arguments = '/i "' + $p + '" ADDLOCAL=ALL INSTALLDIR=C:\gstreamer\1.0\msvc_x86_64 /passive /norestart'
    $r = Start-Process msiexec.exe -ArgumentList $arguments -Wait -PassThru
    if ($r.ExitCode -notin @(0, 3010)) { throw "GStreamer install did not complete: $($r.ExitCode)" }
}

# 3. FFmpeg
$ffmpeg = Join-Path $runtime 'ffmpeg-9.0.2-essentials_build\bin\ffmpeg.exe'
if (Test-Path $ffmpeg) {
    Write-Output 'FFmpeg 9.0.2: already installed'
} else {
    Write-Output 'FFmpeg 9.0.2: installing...'
    $p = Join-Path $runtime 'ffmpeg-9.0.2.zip'
    Get-Verified 'https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-9.0.2-essentials_build.zip' $p '60F467265B1E312373DBCD92200C2618A74850F98D3D078E94296BB3FA2047BA' $null
    Expand-Archive -LiteralPath $p -DestinationPath $runtime -Force
    if (-not (Test-Path $ffmpeg)) { throw 'FFmpeg archive layout did not match' }
}

# 4. Python, for the browser chat reader. Installed for this user only; Rosorah looks here first.
$python = Join-Path $env:LOCALAPPDATA 'Programs\Python\Python313'
if (Test-Path (Join-Path $python 'python.exe')) {
    Write-Output 'Python 3.13: already installed'
} else {
    Write-Output 'Python 3.13.16: installing...'
    $p = Join-Path $runtime 'python-3.13.16-amd64.exe'
    Get-Verified 'https://www.python.org/ftp/python/3.13.16/python-3.13.16-amd64.exe' $p 'FB4F9F5D438B2396DA0086DC70B935C530CB578E37ADC6D354F7AD2037FEE83B' $null
    if (-not (Test-Signer $p 'Python Software Foundation')) { throw 'Python signature verification failed' }
    $arguments = @('/quiet', 'InstallAllUsers=0', 'PrependPath=1', 'Include_test=0', 'Include_doc=0', 'Include_tcltk=0', 'Shortcuts=0', 'AssociateFiles=0', ('TargetDir="' + $python + '"'))
    $r = Start-Process $p -ArgumentList $arguments -Wait -PassThru
    if ($r.ExitCode -notin @(0, 3010)) { throw "Python install did not complete: $($r.ExitCode)" }
    if (-not (Test-Path (Join-Path $python 'python.exe'))) { throw 'Python was not installed where expected' }
}

# Installers are no longer needed once everything is in place.
Remove-Item -ErrorAction SilentlyContinue (Join-Path $runtime 'vc_redist.x64.exe'), (Join-Path $runtime 'gstreamer-1.26.10.msi'), (Join-Path $runtime 'ffmpeg-9.0.2.zip'), (Join-Path $runtime 'python-3.13.16-amd64.exe')
