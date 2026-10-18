# Build-machine dependency setup, not an end-user installer.
$ErrorActionPreference = 'Stop'
$version = '1.26.10'
$packages = @{
    "gstreamer-1.0-msvc-x86_64-$version.msi" = 'a863bf3faa49e9f33bd3cc42967b473482d4dc98655ed95cba1ac59f26fb0cfb'
    "gstreamer-1.0-devel-msvc-x86_64-$version.msi" = '8487a115fea3b0b0b4c55a9a413ad1adab0fe78f6681feedc60858a48613a121'
}
foreach ($name in $packages.Keys) {
    $path = Join-Path $env:TEMP $name
    Invoke-WebRequest "https://gstreamer.freedesktop.org/data/pkg/windows/$version/msvc/$name" -OutFile $path
    if ((Get-FileHash $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $packages[$name]) { throw "Checksum mismatch: $name" }
    $installer = Start-Process msiexec.exe -Wait -PassThru -ArgumentList @('/i', "`"$path`"", '/qn', '/norestart', 'ADDLOCAL=ALL', 'INSTALLDIR=C:\gstreamer\1.0\msvc_x86_64')
    if ($installer.ExitCode -notin @(0,3010)) { throw "GStreamer installation failed: $($installer.ExitCode)" }
}
$env:GSTREAMER_1_0_ROOT_MSVC_X86_64 = 'C:\gstreamer\1.0\msvc_x86_64'
