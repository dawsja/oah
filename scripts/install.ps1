# oah installer for Windows. Keep this file ASCII-only: `irm | iex` may decode it as Latin-1.
# (PowerShell 5.1+ or PowerShell 7)
#
#   irm https://oah.dev/install.ps1 | iex
#
# Options (environment variables, set before running):
#   OAH_VERSION        version to install, e.g. 0.1.0 (default: latest)
#   OAH_INSTALL        install root (default: ~\.config\oah); the binary goes in $OAH_INSTALL\bin
#   OAH_DOWNLOAD_URL   where release files live (default: https://dl.oah.dev - the R2 bucket)
#                      layout: $OAH_DOWNLOAD_URL/latest.txt
#                              $OAH_DOWNLOAD_URL/v<version>/oah-windows-<arch>.tar.gz[.sha256]
#                      GitHub releases use the same layout:
#                      $env:OAH_DOWNLOAD_URL = "https://github.com/dawsja/oah/releases/download"
#   OAH_NO_MODIFY_PATH set to 1 to leave your user PATH alone

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

function Fail($msg) { Write-Host "error: $msg" -ForegroundColor Red; exit 1 }

$Base = if ($env:OAH_DOWNLOAD_URL) { $env:OAH_DOWNLOAD_URL.TrimEnd('/') } else { 'https://dl.oah.dev' }
$Root = if ($env:OAH_INSTALL) { $env:OAH_INSTALL } else { Join-Path $HOME '.config\oah' }
$BinDir = Join-Path $Root 'bin'
$GitHubLatest = 'https://github.com/dawsja/oah/releases/latest/download/latest.txt'

# --- platform ----------------------------------------------------------------------
$osArch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString()
switch ($osArch) {
  'X64' { $arch = 'x64' }
  'Arm64' { $arch = 'arm64' }
  default { Fail "unsupported architecture: $osArch" }
}
$target = "windows-$arch"
if (-not (Get-Command tar.exe -ErrorAction SilentlyContinue)) { Fail 'tar.exe is required (included with Windows 10 1803 and later)' }

# --- version -----------------------------------------------------------------------
$version = if ($env:OAH_VERSION) { $env:OAH_VERSION } else { 'latest' }
if ($version -eq 'latest') {
  $version = $null
  foreach ($u in @("$Base/latest.txt", $GitHubLatest)) {
    try { $version = (Invoke-WebRequest -UseBasicParsing -Uri $u).Content; break } catch {}
  }
  if ($version -is [byte[]]) { $version = [Text.Encoding]::UTF8.GetString($version) }
  if (-not $version) { Fail "couldn't determine the latest version from $Base/latest.txt" }
}
$version = "$version".Trim().TrimStart('v')

Write-Host ''
Write-Host '  oah' -ForegroundColor Magenta -NoNewline; Write-Host ' installer' -ForegroundColor DarkGray
Write-Host "  version $version  platform $target" -ForegroundColor DarkGray
Write-Host ''

# --- download & verify -------------------------------------------------------------
$tmp = Join-Path ([IO.Path]::GetTempPath()) ("oah-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $tmp | Out-Null
try {
  $file = "oah-$target.tar.gz"
  $url = "$Base/v$version/$file"
  $archive = Join-Path $tmp $file
  Write-Host "  downloading $url" -ForegroundColor DarkGray
  try { Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $archive } catch { Fail "download failed: $url" }

  try {
    $sum = (Invoke-WebRequest -UseBasicParsing -Uri "$url.sha256").Content
    if ($sum -is [byte[]]) { $sum = [Text.Encoding]::UTF8.GetString($sum) }
    $want = ("$sum".Trim() -split '\s+')[0].ToLower()
    $got = (Get-FileHash -Algorithm SHA256 -Path $archive).Hash.ToLower()
    if ($want -ne $got) { Fail "checksum mismatch for $file (expected $want, got $got)" }
    Write-Host '  checksum    ok' -ForegroundColor DarkGray
  } catch [System.Net.WebException], [Microsoft.PowerShell.Commands.HttpResponseException] {
    # No checksum published for this release.
  }

  # --- install ---------------------------------------------------------------------
  & tar.exe -xzf $archive -C $tmp
  if ($LASTEXITCODE) { Fail "couldn't extract $file" }
  $exe = Join-Path $tmp 'oah.exe'
  if (-not (Test-Path $exe)) { Fail "archive doesn't contain oah.exe" }
  New-Item -ItemType Directory -Force -Path $BinDir | Out-Null
  $dest = Join-Path $BinDir 'oah.exe'
  try { Move-Item -Force $exe $dest } catch { Fail "couldn't replace $dest - is oah running? Close it and try again." }
  Unblock-File $dest -ErrorAction SilentlyContinue
  Write-Host "  installed   $dest" -ForegroundColor DarkGray
} finally {
  Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
}

# --- PATH --------------------------------------------------------------------------
# Edit the raw registry value so entries like %USERPROFILE%\... stay unexpanded.
$onPath = ($env:Path -split ';') -contains $BinDir
if (-not $onPath -and $env:OAH_NO_MODIFY_PATH -ne '1') {
  $envKey = Get-Item 'HKCU:\Environment'
  $raw = $envKey.GetValue('Path', '', 'DoNotExpandEnvironmentNames')
  if (-not (($raw -split ';') -contains $BinDir)) {
    $newPath = if ($raw) { "$BinDir;$raw" } else { $BinDir }
    Set-ItemProperty -Path 'HKCU:\Environment' -Name Path -Value $newPath -Type ExpandString
    # Tell Windows the environment changed so new terminals pick it up.
    [Environment]::SetEnvironmentVariable('OAH_INSTALL_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('OAH_INSTALL_REFRESH', $null, 'User')
    Write-Host "  PATH        added $BinDir to your user PATH" -ForegroundColor DarkGray
  }
  $env:Path = "$BinDir;$env:Path"
}

Write-Host ''
Write-Host "  oah $version is installed." -ForegroundColor Green
Write-Host '  Run:'
Write-Host ''
Write-Host '    oah'
Write-Host ''
