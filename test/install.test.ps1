# Run with Windows PowerShell 5.1 or PowerShell 7; no test modules required.
# Each scenario runs in a child process because the installer uses exit on failure.
param(
  [string]$Scenario,
  [string]$FixtureRoot
)

$ErrorActionPreference = 'Stop'
$installer = Join-Path (Split-Path $PSScriptRoot -Parent) 'scripts\install.ps1'
$cases = @(
  @{ Name = 'x64'; Native = 'AMD64'; Arch = 'x64' },
  @{ Name = 'arm64'; Native = 'ARM64'; Arch = 'arm64' },
  @{ Name = 'wow64-x64'; Native = 'x86'; Wow64 = 'AMD64'; Arch = 'x64' },
  @{ Name = 'wow64-arm64'; Native = 'x86'; Wow64 = 'ARM64'; Arch = 'arm64' },
  @{ Name = 'unsupported'; Native = 'x86'; Error = 'unsupported or undetectable architecture' },
  @{ Name = 'missing'; Error = 'unsupported or undetectable architecture' },
  @{ Name = 'latest-bytes'; Native = 'AMD64'; Arch = 'x64'; Bytes = $true },
  @{ Name = 'pinned'; Native = 'AMD64'; Arch = 'x64'; Pinned = $true },
  @{ Name = 'no-checksum'; Native = 'AMD64'; Arch = 'x64'; Checksum = 'missing' },
  @{ Name = 'bad-checksum'; Native = 'AMD64'; Arch = 'x64'; Checksum = 'bad'; Error = 'checksum mismatch' },
  @{ Name = 'unexpected-error'; Native = 'AMD64'; Arch = 'x64'; Checksum = 'unexpected'; Error = 'fixture verification failure' }
)
if ($PSVersionTable.PSVersion.Major -ge 7) {
  $cases += @{ Name = 'no-checksum-ps7'; Native = 'AMD64'; Arch = 'x64'; Checksum = 'missing-ps7' }
}

if ($Scenario) {
  $case = $cases | Where-Object { $_.Name -eq $Scenario }
  if (-not $case) { throw "unknown test scenario: $Scenario" }
  $env:PROCESSOR_ARCHITECTURE = $case.Native
  $env:PROCESSOR_ARCHITEW6432 = $case.Wow64
  $env:OAH_INSTALL = Join-Path $FixtureRoot $Scenario
  $env:OAH_DOWNLOAD_URL = 'https://installer.invalid'
  $env:OAH_NO_MODIFY_PATH = '1'
  $env:OAH_VERSION = if ($case.Pinned) { 'v1.2.3' } else { $null }
  $archiveFixture = Join-Path $FixtureRoot 'fixture.tar.gz'
  $expectedUrl = "https://installer.invalid/v1.2.3/oah-windows-$($case.Arch).tar.gz"

  # Mock only the network; exercise extraction, verification and file installation.
  function Invoke-WebRequest {
    param([switch]$UseBasicParsing, [string]$Uri, [string]$OutFile)
    if ($Uri -eq 'https://installer.invalid/latest.txt') {
      if ($case.Pinned) { throw 'pinned versions must not request latest.txt' }
      $content = "1.2.3`n"
      if ($case.Bytes) { $content = [Text.Encoding]::UTF8.GetBytes($content) }
      return @{ Content = $content }
    }
    if ($Uri -eq $expectedUrl -and $OutFile) {
      Copy-Item $archiveFixture $OutFile
      return
    }
    if ($Uri -eq "$expectedUrl.sha256") {
      switch ($case.Checksum) {
        'missing' { throw [System.Net.WebException]::new('fixture checksum not found') }
        'missing-ps7' {
          $response = [System.Net.Http.HttpResponseMessage]::new([System.Net.HttpStatusCode]::NotFound)
          throw [Microsoft.PowerShell.Commands.HttpResponseException]::new('fixture checksum not found', $response)
        }
        'unexpected' { throw [System.InvalidOperationException]::new('fixture verification failure') }
        'bad' { return @{ Content = ('0' * 64) } }
      }
      $hash = (Get-FileHash -Algorithm SHA256 $archiveFixture).Hash
      $content = "$hash  oah-windows-$($case.Arch).tar.gz`n"
      if ($case.Bytes) { return @{ Content = [Text.Encoding]::UTF8.GetBytes($content) } }
      return @{ Content = $content }
    }
    throw "unexpected request: $Uri"
  }

  try {
    # Match the advertised irm | iex execution style, including its scope.
    Get-Content -Raw $installer | Invoke-Expression
    $dest = Join-Path $env:OAH_INSTALL 'bin\oah.exe'
    if (-not (Test-Path $dest)) { throw 'installer did not create oah.exe' }
    if ((Get-Content -Raw $dest) -ne 'installer test fixture') { throw 'installed file differs from fixture' }
  } catch {
    Write-Output $_.Exception.Message
    exit 1
  }
  exit 0
}

$root = Join-Path ([IO.Path]::GetTempPath()) ('oah-installer-test-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $root | Out-Null
try {
  [IO.File]::WriteAllText((Join-Path $root 'oah.exe'), 'installer test fixture')
  & tar.exe -czf (Join-Path $root 'fixture.tar.gz') -C $root oah.exe
  if ($LASTEXITCODE) { throw 'could not create test archive' }
  $engine = (Get-Process -Id $PID).Path
  foreach ($case in $cases) {
    $output = & $engine -NoProfile -File $PSCommandPath -Scenario $case.Name -FixtureRoot $root 2>&1
    $code = $LASTEXITCODE
    $text = $output -join "`n"
    if ($case.Error) {
      if ($code -eq 0 -or $text -notmatch [regex]::Escape($case.Error)) {
        throw "scenario $($case.Name): expected failure '$($case.Error)', got exit $code`n$text"
      }
      if (Test-Path (Join-Path $root "$($case.Name)\bin\oah.exe")) {
        throw "scenario $($case.Name): installed a file despite failure"
      }
    } elseif ($code -ne 0) {
      throw "scenario $($case.Name): exit $code`n$text"
    }
    Write-Host "PASS $($case.Name)"
  }
} finally {
  Remove-Item -Recurse -Force $root
}
# Expected-failure children leave LASTEXITCODE nonzero; signal suite success to CI.
exit 0
