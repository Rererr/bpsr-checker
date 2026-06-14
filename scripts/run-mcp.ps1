# bpsr-app を Slint 埋め込み MCP サーバー付きで起動するローカル開発用ヘルパー。
#
# Slint テストバックエンド (i-slint-backend-testing) 同梱の MCP サーバーを有効化し、
# Claude Code などの MCP クライアントから起動中の UI を検査・操作できるようにする。
#   - ビルド時 feature: mcp（slint-app の opt-in。slint/mcp は 1.16.1 では未公開）
#   - SLINT_EMIT_DEBUG_INFO=1 : 要素内省メタの埋め込み（ビルド時必須）
#   - SLINT_MCP_PORT          : 設定時のみ MCP サーバーが起動（既定 8080）
#   - エンドポイント          : http://127.0.0.1:<Port>/mcp （localhost 限定バインド）
#
# 既定はデモ起動（BPSR_DEMO=1 + 管理者要求マニフェスト無効化）で UAC 不要。
# -NoDemo を付けると実観測モード（WinDivert 利用・管理者権限が必要）。
#
# 使い方:
#   pwsh scripts/run-mcp.ps1                 # ポート 8080・デモモードで起動
#   pwsh scripts/run-mcp.ps1 -Port 8090      # ポート変更
#   pwsh scripts/run-mcp.ps1 -NoDemo         # 実観測（要管理者・WinDivert 同梱）
#
# 注意: この feature/env はローカルのデバッグ起動専用。release ビルドや
#       配布スクリプト (package-slint.ps1) には絶対に付与しないこと。

[CmdletBinding()]
param(
    [int]$Port = 8080,
    [switch]$NoDemo
)

$ErrorActionPreference = "Stop"

$env:SLINT_EMIT_DEBUG_INFO = "1"
$env:SLINT_MCP_PORT = "$Port"

if ($NoDemo) {
    # 実観測モード: 管理者要求マニフェストを埋め込み（要 UAC 昇格・WinDivert 必要）。
    Remove-Item Env:\BPSR_SKIP_MANIFEST -ErrorAction SilentlyContinue
    Remove-Item Env:\BPSR_DEMO -ErrorAction SilentlyContinue
    Write-Host "[run-mcp] 実観測モードで起動します（管理者権限が必要）。" -ForegroundColor Yellow
} else {
    # デモモード: UAC 回避（os error 740 回避）＋合成データ。
    $env:BPSR_SKIP_MANIFEST = "1"
    $env:BPSR_DEMO = "1"
    Write-Host "[run-mcp] デモモードで起動します（UAC 不要・合成データ）。" -ForegroundColor Cyan
}

Write-Host "[run-mcp] MCP endpoint: http://127.0.0.1:$Port/mcp" -ForegroundColor Green

cargo run -p bpsr-app --features mcp
