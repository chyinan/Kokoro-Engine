param(
  [Parameter(Mandatory=$true)][string]$PlaywrightModule,
  [switch]$SkipBuild,
  [switch]$KeepRunning
)
$ErrorActionPreference = 'Stop'
$chatRepo = Split-Path -Parent $PSScriptRoot
Set-Location -LiteralPath $chatRepo
$chatOutput = Join-Path $chatRepo 'output/playwright/chat-lifecycle'
New-Item -ItemType Directory -Path $chatOutput -Force | Out-Null
$chatExecutable = Join-Path $chatRepo 'src-tauri/target/debug/tauri-appkokoro-engine.exe'
# Stop only this fixture's recorded executable/command, never arbitrary PID reuse.
foreach ($chatPidFile in @('app.pid','fixture.pid')) {
  $chatPidPath = Join-Path $chatOutput $chatPidFile
  if (Test-Path -LiteralPath $chatPidPath) {
    $chatPrevious = Get-CimInstance Win32_Process -Filter ('ProcessId = ' + [int](Get-Content -LiteralPath $chatPidPath))
    if ($chatPrevious -and ($chatPrevious.ExecutablePath -eq $chatExecutable -or $chatPrevious.CommandLine -like '*scripts/chat-lifecycle-model-fixture.mjs*')) { Stop-Process -Id $chatPrevious.ProcessId }
  }
}
$chatRun = Get-Date -Format 'yyyyMMdd-HHmmss'
foreach ($chatArtifact in @('native-report.json','controlled-model-calls.json')) {
  $chatSource = Join-Path $chatOutput $chatArtifact
  if (Test-Path -LiteralPath $chatSource) { Copy-Item -LiteralPath $chatSource -Destination (Join-Path $chatOutput ($chatRun + '-previous-' + $chatArtifact)) }
}
$env:TAURI_CONFIG = Get-Content -Raw (Join-Path $PSScriptRoot 'chat-lifecycle.tauri.conf.json')
if (-not $SkipBuild) {
  cargo build --manifest-path src-tauri/Cargo.toml *> (Join-Path $chatOutput 'build.log')
  if ($LASTEXITCODE -ne 0) { throw 'Isolated native build failed; see build.log' }
}
$env:KOKORO_TEST_DATA_ROOT = Join-Path $chatOutput ('data-' + $chatRun)
$env:WEBVIEW2_USER_DATA_FOLDER = Join-Path $env:TEMP ('kokoro-chat-validation-' + $chatRun)
$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=19333'
$env:PLAYWRIGHT_MODULE = $PlaywrightModule
$chatData = Join-Path $env:KOKORO_TEST_DATA_ROOT 'com.chyin.kokoro'
New-Item -ItemType Directory -Path $chatData -Force | Out-Null
@{ active_provider='fixture'; system_provider='fixture'; providers=@(@{ id='fixture'; provider_type='openai'; enabled=$true; supports_native_tools=$true; api_key='controlled-fixture'; base_url='http://127.0.0.1:19334/v1'; model='controlled-sse'; extra=@{} }) } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $chatData 'llm_config.json') -Encoding utf8
'{"enabled":false}' | Set-Content -LiteralPath (Join-Path $chatData 'proactive_enabled.json') -Encoding utf8
$chatVite = $null
if (-not (Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)) {
  $chatVite = Start-Process -FilePath (Get-Command node).Source -ArgumentList 'node_modules/vite/bin/vite.js' -WorkingDirectory $chatRepo -WindowStyle Hidden -RedirectStandardOutput (Join-Path $chatOutput 'vite.log') -RedirectStandardError (Join-Path $chatOutput 'vite.stderr.log') -PassThru
  $chatVite.Id | Set-Content -LiteralPath (Join-Path $chatOutput 'vite.pid')
}
$chatModel = Start-Process -FilePath (Get-Command node).Source -ArgumentList 'scripts/chat-lifecycle-model-fixture.mjs' -WorkingDirectory $chatRepo -WindowStyle Hidden -RedirectStandardOutput (Join-Path $chatOutput 'model.log') -RedirectStandardError (Join-Path $chatOutput 'model.stderr.log') -PassThru
$chatModel.Id | Set-Content -LiteralPath (Join-Path $chatOutput 'fixture.pid')
$chatApp = Start-Process -FilePath $chatExecutable -WorkingDirectory $chatRepo -WindowStyle Hidden -RedirectStandardOutput (Join-Path $chatOutput 'app.stdout.log') -RedirectStandardError (Join-Path $chatOutput 'app.stderr.log') -PassThru
$chatApp.Id | Set-Content -LiteralPath (Join-Path $chatOutput 'app.pid')
Get-FileHash -LiteralPath $chatExecutable | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $chatOutput 'executable.json')
@{ run=$chatRun; dataRoot=$env:KOKORO_TEST_DATA_ROOT; webviewProfile=$env:WEBVIEW2_USER_DATA_FOLDER; appPid=$chatApp.Id; fixturePid=$chatModel.Id } | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $chatOutput 'run-info.json')
try {
  $chatDeadline = (Get-Date).AddSeconds(30)
  while (-not (Get-NetTCPConnection -LocalPort 19333 -State Listen -ErrorAction SilentlyContinue)) {
    if ((Get-Date) -gt $chatDeadline) { throw 'WebView2 CDP startup timed out' }
    Start-Sleep -Milliseconds 100
  }
  node scripts/chat-lifecycle-native-check.mjs *> (Join-Path $chatOutput 'native-check.log')
  if ($LASTEXITCODE -ne 0) { throw 'Native assertions failed; see native-check.log and native-report.json' }
  Get-Content -LiteralPath (Join-Path $chatOutput 'native-check.log')
} finally {
  if (-not $KeepRunning) {
    Stop-Process -Id $chatApp.Id,$chatModel.Id -ErrorAction SilentlyContinue
    if ($chatVite) { Stop-Process -Id $chatVite.Id -ErrorAction SilentlyContinue }
  }
}
