# Install au.exe from a pinned GitHub release (Windows x86_64 only).
# The archive is verified against SHA256SUMS before anything is extracted.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Version = if ($env:ANDROID_USE_VERSION) { $env:ANDROID_USE_VERSION } else { '1.0.0' }
$Base = "https://github.com/austinintelligence/android-use/releases/download/v$Version"
$Asset = "android-use-v$Version-windows-x86_64.zip"
$InstallDir = if ($env:AU_INSTALL_DIR) { $env:AU_INSTALL_DIR } else { Join-Path $env:LOCALAPPDATA 'AndroidUse\bin' }

if ([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture -ne 'X64') {
  throw 'Unsupported platform: this installer supports Windows x86_64 only.'
}

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ([guid]::NewGuid().ToString())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
  Invoke-WebRequest -Uri "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset)
  Invoke-WebRequest -Uri "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp 'SHA256SUMS')

  # Verify before extracting anything from the archive.
  $Expected = Get-Content (Join-Path $Tmp 'SHA256SUMS') | ForEach-Object {
    $parts = $_ -split '\s+'
    if ($parts.Count -ge 2 -and $parts[1].TrimStart('*') -eq $Asset) { $parts[0] }
  } | Select-Object -First 1
  if (-not $Expected) { throw "SHA256SUMS has no entry for $Asset. Aborting." }
  $Actual = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Asset)).Hash
  if ($Actual -ine $Expected) { throw "SHA256 mismatch for $Asset (expected $Expected, got $Actual). Aborting." }

  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $Zip = [IO.Compression.ZipFile]::OpenRead((Join-Path $Tmp $Asset))
  try {
    [IO.Compression.ZipFileExtensions]::ExtractToFile($Zip.GetEntry('au.exe'), (Join-Path $Tmp 'au.exe'), $true)
  } finally {
    $Zip.Dispose()
  }

  New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
  Copy-Item (Join-Path $Tmp 'au.exe') (Join-Path $InstallDir 'au.exe') -Force
  Write-Output "Installed au to $InstallDir. Next: au setup"
  if (-not (($env:Path -split ';') -contains $InstallDir)) {
    Write-Output "Add $InstallDir to your PATH to run au from anywhere."
  }
} finally {
  Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
