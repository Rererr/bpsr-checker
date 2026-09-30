# Launch the already-built debug bpsr-app.exe elevated, in real-capture mode,
# with the embedded Slint MCP server. Local debugging helper only.
# Build first (PowerShell): $env:SLINT_EMIT_DEBUG_INFO='1'; cargo build -p bpsr-app --features mcp
param([int]$Port = 18080)
$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$exe = Join-Path $repoRoot "target\debug\bpsr-app.exe"
if (-not (Test-Path -LiteralPath $exe)) { throw "exe not found: $exe" }
$root = $repoRoot.Replace("'", "''")
$inner = "Set-Location '$root'; `$env:SLINT_MCP_PORT='$Port'; & '.\target\debug\bpsr-app.exe'"
Start-Process powershell -Verb RunAs -ArgumentList '-NoProfile', '-Command', $inner | Out-Null
Write-Output "Requested elevated launch (MCP port $Port)"
