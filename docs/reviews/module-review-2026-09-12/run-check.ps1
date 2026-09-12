# Imperative shell: execute one validation command and preserve its raw evidence.
param(
  [Parameter(Mandatory = $true)][string]$Name,
  [Parameter(Mandatory = $true)][string]$Program,
  [string[]]$Arguments = @(),
  [string]$WorkingDirectory = 'D:\Programs\Kokoro-Engine'
)
$ErrorActionPreference = 'Continue'
$evidenceDirectory = Join-Path $PSScriptRoot 'evidence'
New-Item -ItemType Directory -Force -Path $evidenceDirectory | Out-Null
$logPath = Join-Path $evidenceDirectory ($Name + '.log')
$resultPath = Join-Path $evidenceDirectory ($Name + '.json')
$started = [DateTimeOffset]::UtcNow
Push-Location -LiteralPath $WorkingDirectory
try {
  & $Program @Arguments *> $logPath
  $resultCode = $LASTEXITCODE
  if ($null -eq $resultCode) { $resultCode = 1 }
} finally {
  Pop-Location
}
$finished = [DateTimeOffset]::UtcNow
[ordered]@{
  command = (@($Program) + $Arguments) -join ' '
  workingDirectory = $WorkingDirectory
  startedUtc = $started.ToString('o')
  finishedUtc = $finished.ToString('o')
  durationSeconds = ($finished - $started).TotalSeconds
  exitCode = $resultCode
  log = $logPath
} | ConvertTo-Json | Set-Content -LiteralPath $resultPath -Encoding UTF8
Get-Content -LiteralPath $resultPath
Get-Content -LiteralPath $logPath -Tail 45
exit $resultCode
