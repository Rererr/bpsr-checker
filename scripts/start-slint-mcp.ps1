[CmdletBinding()]
param(
    [int]$Port = 8080,
    [switch]$Wait,
    [switch]$LaunchOnly,
    [int]$TimeoutSec = 180
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$runScript = Join-Path $repoRoot "scripts\run-mcp.ps1"
if (-not (Test-Path -LiteralPath $runScript)) {
    throw "MCP launcher script not found: $runScript"
}

function Test-McpPort {
    try {
        # A TCP listener alone does not prove that this is the Slint MCP server.
        $response = Invoke-WebRequest `
            -UseBasicParsing `
            -Uri "http://127.0.0.1:$Port/mcp" `
            -Method Post `
            -ContentType "application/json" `
            -Body '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' `
            -TimeoutSec 2
        if ($response.StatusCode -ne 200) {
            return $false
        }
        $payload = $response.Content | ConvertFrom-Json
        return $payload.jsonrpc -eq "2.0" -and $payload.result.serverInfo.name -eq "slint-mcp-embedded"
    } catch {
        return $false
    }
}

function Wait-McpPort {
    param([int]$Seconds)

    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (Test-McpPort) {
            return $true
        }
        Start-Sleep -Milliseconds 250
    }
    return (Test-McpPort)
}

function ConvertTo-PowerShellLiteral {
    param([string]$Value)

    return "'$( $Value.Replace("'", "''") )'"
}

if ($LaunchOnly) {
    $Wait = $false
}

if (Test-McpPort) {
    Write-Output "Slint MCP is already listening on 127.0.0.1:$Port"
    return
}

$mutex = $null
$lockTaken = $false
try {
    $mutex = New-Object -TypeName System.Threading.Mutex -ArgumentList @(
        $false,
        "Local\BpsrChecker-SlintMcp-$Port"
    )
    try {
        $lockTaken = $mutex.WaitOne(0)
    } catch [System.Threading.AbandonedMutexException] {
        $lockTaken = $true
    }

    if (-not $lockTaken) {
        if ($Wait -and (Wait-McpPort -Seconds $TimeoutSec)) {
            Write-Output "Slint MCP became ready on 127.0.0.1:$Port"
            return
        }
        Write-Output "Another Slint MCP launcher is already running"
        return
    }

    if (Test-McpPort) {
        Write-Output "Slint MCP is already listening on 127.0.0.1:$Port"
        return
    }

    $logRoot = if ($env:LOCALAPPDATA) {
        Join-Path $env:LOCALAPPDATA "bpsr-checker"
    } else {
        Join-Path $repoRoot ".codex"
    }
    New-Item -ItemType Directory -Path $logRoot -Force | Out-Null
    $logPath = Join-Path $logRoot "slint-mcp-startup.log"

    $elevatedCommand = "& $(ConvertTo-PowerShellLiteral $runScript) -Port $Port -NoDemo *> $(ConvertTo-PowerShellLiteral $logPath)"
    Start-Process `
        -FilePath "powershell.exe" `
        -ArgumentList @(
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-Command", $elevatedCommand
        ) `
        -WorkingDirectory $repoRoot `
        -Verb RunAs `
        -WindowStyle Hidden | Out-Null

    Write-Output "Slint MCP launch requested on 127.0.0.1:$Port (startup log: $logPath)"

    if ($Wait) {
        if (Wait-McpPort -Seconds $TimeoutSec) {
            Write-Output "Slint MCP is ready on 127.0.0.1:$Port"
            return
        }
        throw "Slint MCP did not start on 127.0.0.1:$Port within $TimeoutSec seconds. See $logPath"
    }
} finally {
    if ($lockTaken -and $null -ne $mutex) {
        $mutex.ReleaseMutex()
    }
    if ($null -ne $mutex) {
        $mutex.Dispose()
    }
}
