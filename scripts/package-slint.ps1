# Slint 版 (bpsr-app) のポータブル配布物を作る。
#   - release ビルド
#   - exe を bpsr-checker.exe にリネームして WinDivert を同梱
#   - zip 化
# 使い方: pwsh scripts/package-slint.ps1
# 配布物の data dir は %APPDATA%\bpsr-checker（インストール不要のポータブル運用）。

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot

# バージョンは slint-app/Cargo.toml を正典とする。
$ver = (Select-String -Path "$root/slint-app/Cargo.toml" -Pattern '^version\s*=\s*"(.+)"').Matches[0].Groups[1].Value
Write-Host "Packaging bpsr-checker (Slint) v$ver"

Write-Host "Building release..."
Push-Location $root
cargo build --release -p bpsr-app
Pop-Location

$exe = Join-Path $root "target/release/bpsr-app.exe"
if (-not (Test-Path $exe)) { throw "build output not found: $exe" }

# WinDivert（配布同梱に必須・exe 隣に置く）。CI は GitHub から DL し src-tauri/ に置く。
$wdDll = Join-Path $root "src-tauri/WinDivert.dll"
$wdSys = Join-Path $root "src-tauri/WinDivert64.sys"
foreach ($f in @($wdDll, $wdSys)) {
    if (-not (Test-Path $f)) { throw "WinDivert not found: $f （WinDivert 2.2.2 x64 を配置してください）" }
}

$out = Join-Path $root "dist-slint/bpsr-checker"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $exe (Join-Path $out "bpsr-checker.exe")
Copy-Item $wdDll $out
Copy-Item $wdSys $out

$zip = Join-Path $root "dist-slint/bpsr-checker-portable-$ver.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $out "*") -DestinationPath $zip

Write-Host "Done. Portable artifact:"
Write-Host "  $zip"
