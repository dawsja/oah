# oah desktop installer for Windows. Keep this file ASCII-only: `irm | iex` may decode it as Latin-1.
# (PowerShell 5.1+ or PowerShell 7)
#
#   irm https://dl.oah.dev/install-desktop.ps1 | iex
#
# Installs oah for the current user (no admin rights needed). The app updates itself.
#
# Options (environment variables, set before running):
#   OAH_DESKTOP_VERSION  version to install, e.g. 0.1.1 (default: latest)
#   OAH_DOWNLOAD_URL     where release files live (default: https://dl.oah.dev)
#                        layout: $OAH_DOWNLOAD_URL/desktop/latest.txt
#                                $OAH_DOWNLOAD_URL/desktop/v<version>/oah-desktop-<version>-windows-x64-setup.exe[.sha256]

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Fail($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

$Base = if ($env:OAH_DOWNLOAD_URL) { $env:OAH_DOWNLOAD_URL.TrimEnd('/') } else { 'https://dl.oah.dev' }

$arch = $env:PROCESSOR_ARCHITECTURE
if ($env:PROCESSOR_ARCHITEW6432) { $arch = $env:PROCESSOR_ARCHITEW6432 }
if ($arch -ne 'AMD64') { Fail "the desktop app is built for x64 Windows only for now (the CLI works on ARM: irm https://oah.dev/install.ps1 | iex)" }

$version = $env:OAH_DESKTOP_VERSION
if (-not $version -or $version -eq 'latest') {
  try { $version = (Invoke-RestMethod -UseBasicParsing "$Base/desktop/latest.txt").ToString().Trim() } catch { Fail "couldn't determine the latest version from $Base/desktop/latest.txt" }
}
$version = $version.TrimStart('v')

Write-Host ""
Write-Host "  oah desktop installer" -ForegroundColor Yellow
Write-Host "  version $version  platform windows-x64"
Write-Host ""

$file = "oah-desktop-$version-windows-x64-setup.exe"
$url = "$Base/desktop/v$version/$file"
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("oah-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $exe = Join-Path $tmp $file
  Write-Host "  downloading $url"
  try { Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $exe } catch { Fail "download failed: $url" }

  try { $want = ((Invoke-RestMethod -UseBasicParsing "$url.sha256").ToString().Trim() -split '\s+')[0] } catch { Fail "couldn't fetch the checksum for $file" }
  $got = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLowerInvariant()
  if ($want.ToLowerInvariant() -ne $got) { Fail "checksum mismatch for $file" }
  Write-Host "  checksum    ok" -ForegroundColor Green

  # The NSIS installer: /S installs silently for the current user.
  Write-Host "  installing"
  $p = Start-Process -FilePath $exe -ArgumentList '/S' -Wait -PassThru
  if ($p.ExitCode -ne 0) { Fail "the installer exited with code $($p.ExitCode)" }
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

Write-Host ""
Write-Host "  oah desktop $version is installed. Open it from the Start menu (oah)." -ForegroundColor Green
Write-Host "  It shares sessions and settings with the oah CLI, and updates itself."
Write-Host ""
