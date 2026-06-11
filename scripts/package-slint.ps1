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

# WinDivert（配布同梱に必須・exe 隣に置く）。windivert/ を優先し src-tauri/ をフォールバック
# （src-tauri 削除後も windivert/ で動くようにする）。CI は GitHub から windivert/ へ DL。
function Resolve-WinDivert([string]$name) {
    foreach ($d in @("windivert", "src-tauri")) {
        $p = Join-Path $root "$d/$name"
        if (Test-Path $p) { return $p }
    }
    throw "WinDivert not found: $name （windivert/ か src-tauri/ に WinDivert 2.2.2 x64 を配置してください）"
}
$wdDll = Resolve-WinDivert "WinDivert.dll"
$wdSys = Resolve-WinDivert "WinDivert64.sys"

$out = Join-Path $root "dist-slint/bpsr-checker"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item $exe (Join-Path $out "bpsr-checker.exe")
Copy-Item $wdDll $out
Copy-Item $wdSys $out

$zip = Join-Path $root "dist-slint/bpsr-checker-portable-$ver.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path (Join-Path $out "*") -DestinationPath $zip

Write-Host "Portable artifact: $zip"

# NSIS インストーラ（makensis があれば生成）。OutFile は絶対パスで明示。
# PATH 未登録でも標準導入先を探索する（CI の choco install 直後は PATH が未更新で
# Get-Command が空振りするため。choco の nsis は Program Files (x86)\NSIS に入る）。
$makensis = Get-Command makensis -ErrorAction SilentlyContinue
if (-not $makensis) {
    foreach ($p in @("$env:ProgramFiles\NSIS\makensis.exe", "${env:ProgramFiles(x86)}\NSIS\makensis.exe")) {
        if (Test-Path $p) { $makensis = Get-Command $p; break }
    }
}
if ($makensis) {
    Write-Host "Building NSIS installer..."
    $setup = Join-Path $root "dist-slint/bpsr-checker-setup-$ver.exe"
    & $makensis.Source "/DVERSION=$ver" "/DSRCDIR=$out" "/DOUTFILE=$setup" (Join-Path $root "installer/installer.nsi")
    if (Test-Path $setup) { Write-Host "Installer: $setup" }
} else {
    Write-Host "makensis not found; skipped installer (portable zip のみ生成)."
}

Write-Host "Done."
