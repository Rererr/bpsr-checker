# dev 専用: WinDivert ドライバを停止し、ロックされた .sys を解放する。
#
# 駆動中（サービス RUNNING）はその WinDivert64.sys がカーネルにロックされ、次回の
# `cargo build` でのドライバ再コピー（target/**/WinDivert64.sys）が失敗する。rebuild の
# 前にこのスクリプトを実行してロックを解放する。
#
# 設計方針（本番と同じ「善良な利用者」）: サービスは **delete しない**（マシン全体で共有され
# 他の WinDivert 利用アプリと共存するため）。壊れた残留サービスはアプリ起動時に自己修復する
# （core/src/capture/windivert.rs の recover_stale_service）。-Force 指定時のみ、停止中で他の
# 誰も使っていないことが確実な場合にサービスを削除する（完全リセット用）。
#
# 要管理者権限。
[CmdletBinding()]
param([switch]$Force)

$ErrorActionPreference = 'SilentlyContinue'

Write-Host 'Stopping WinDivert driver (dev cleanup)...'
sc.exe stop WinDivert | Out-Null
Start-Sleep -Milliseconds 600

$svc = Get-Service -Name WinDivert -ErrorAction SilentlyContinue
if ($null -eq $svc) {
    Write-Host 'WinDivert service not present. (clean)'
    return
}

Write-Host ("WinDivert service state: {0}" -f $svc.Status)

if ($Force) {
    if ($svc.Status -eq 'Stopped') {
        Write-Host 'Deleting stopped WinDivert service (-Force)...'
        sc.exe delete WinDivert | Out-Null
    }
    else {
        Write-Warning 'WinDivert still running (in use by another process?). Skipping delete to avoid breaking it.'
    }
}
