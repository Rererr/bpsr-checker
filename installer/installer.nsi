; bpsr-checker (Slint版) スタンドアロン NSIS インストーラ。
; Tauri 生成の NSIS を置き換える。WinDivert 同梱・管理者インストール・
; インストール/アンインストール時にアプリ強制終了＋WinDivertドライバ停止。
;
; Tauri→Slint 移行対策: インストール時に旧 Tauri 版(per-user)を検出して
; サイレント削除し、ショートカットを旧版と同じ「スタートメニュー直下」＋
; デスクトップへ作る。これをしないと旧版が別ディレクトリに残り、ユーザーが
; いつものアイコン(旧版を指す)から旧版を起動してしまう。
; 注: 旧版は per-user(HKCU) なので、別アカウントへ UAC 昇格した場合は検出
; できない（単一管理者ユーザーの家庭環境では同一ユーザー昇格で検出できる）。
;
; ビルド: makensis を入れて
;   makensis /DSRCDIR=..\dist-slint\bpsr-checker /DVERSION=1.0.1 installer.nsi
; SRCDIR には bpsr-checker.exe / WinDivert.dll / WinDivert64.sys を置いておく
; （scripts/package-slint.ps1 が用意する）。

Unicode true

!include "MUI2.nsh"
!include "LogicLib.nsh"

!ifndef VERSION
  !define VERSION "0.0.0"
!endif
!ifndef SRCDIR
  !define SRCDIR "..\dist-slint\bpsr-checker"
!endif

!define APPNAME "bpsr-checker"
!define EXENAME "bpsr-checker.exe"
!define DATADIR "$APPDATA\bpsr-checker"  ; Slint版の data dir（旧 com.rererr.* ではない）
!define REGKEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\${APPNAME}"
!ifndef OUTFILE
  !define OUTFILE "${APPNAME}-setup-${VERSION}.exe"  ; 既定。CI/スクリプトは絶対パスで上書き
!endif

Name "${APPNAME} ${VERSION}"
OutFile "${OUTFILE}"
InstallDir "$PROGRAMFILES64\${APPNAME}"
InstallDirRegKey HKLM "Software\${APPNAME}" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "English"

; アプリを強制終了し、WinDivert ドライバを停止する（ベストエフォート・失敗しても続行）。
; 重要: "WinDivert" サービスはマシン全体で共有され、他の WinDivert 利用アプリ
; （姉妹アプリ bpsr-module-optimizer 等）も使う可能性があるため **delete はしない**。
; sc stop は他プロセスが使用中なら拒否されるため、共存中でも相手のキャプチャを壊さない。
; 残留した壊れたサービスはアプリ起動時に自己修復する（recover_stale_service）。
!macro KillAppAndDriver
  nsExec::ExecToLog 'taskkill /F /IM ${EXENAME} /T'
  Sleep 500
  nsExec::ExecToLog 'sc stop WinDivert'
  Sleep 1000
!macroend

; 前後の引用符を除去（レジストリの InstallLocation 等が引用符付きで入る場合がある）。
; スタック経由: Push 文字列 → Call → Pop 結果。
Function UnquoteStr
  Exch $0
  Push $1
  StrCpy $1 $0 1
  ${If} $1 == '"'
    StrCpy $0 $0 "" 1          ; 先頭の " を除去
    StrCpy $1 $0 1 -1
    ${If} $1 == '"'
      StrCpy $0 $0 -1          ; 末尾の " を除去
    ${EndIf}
  ${EndIf}
  Pop $1
  Exch $0
FunctionEnd

; 旧 Tauri 版(per-user, HKCU)を検出してサイレント削除する。
; 新版は別ディレクトリ($PROGRAMFILES64)・別 data dir のため設定には影響しない。
; _?= 付きで同期実行し、Section が走る前に旧版の削除(旧ショートカット含む)を
; 完了させる。新旧でショートカット名が同じため、非同期だと新規作成と競合する。
Function UninstallOldTauri
  ReadRegStr $0 HKCU "${REGKEY}" "UninstallString"
  ${If} $0 == ""
    Return
  ${EndIf}
  ReadRegStr $1 HKCU "${REGKEY}" "InstallLocation"
  Push $1
  Call UnquoteStr
  Pop $1
  ${If} $1 != ""
    ExecWait '$0 /S _?=$1'     ; _?= で同期実行（自己コピー/早期 return を防ぐ）
    Delete "$1\uninstall.exe"  ; _?= 実行では uninstaller が残るので消す
    RMDir "$1"
  ${Else}
    ExecWait '$0 /S'
  ${EndIf}
  ; 取りこぼし対策（旧 per-user の登録とショートカットを念のため除去）。
  DeleteRegKey HKCU "${REGKEY}"
  Delete "$SMPROGRAMS\${APPNAME}.lnk"
FunctionEnd

Function .onInit
  !insertmacro KillAppAndDriver
  Call UninstallOldTauri
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\${EXENAME}"
  File "${SRCDIR}\WinDivert.dll"
  File "${SRCDIR}\WinDivert64.sys"

  WriteUninstaller "$INSTDIR\uninstall.exe"

  ; 旧 v1.0.0 がスタートメニューのサブフォルダに作ったショートカットを掃除。
  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"

  ; ショートカットは旧 Tauri 版と同じ「スタートメニュー直下」＋デスクトップに作る。
  CreateShortcut "$SMPROGRAMS\${APPNAME}.lnk" "$INSTDIR\${EXENAME}"
  CreateShortcut "$DESKTOP\${APPNAME}.lnk" "$INSTDIR\${EXENAME}"

  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${REGKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${REGKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${REGKEY}" "Publisher" "rererr"
  WriteRegStr HKLM "${REGKEY}" "DisplayIcon" "$INSTDIR\${EXENAME}"
  WriteRegStr HKLM "${REGKEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKLM "${REGKEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegStr HKLM "${REGKEY}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKLM "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${REGKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  !insertmacro KillAppAndDriver

  Delete "$INSTDIR\${EXENAME}"
  ; WinDivert の .dll/.sys は駆動中ロックされ得る（自プロセス終了直後や他アプリ使用中）。
  ; ロック時は再起動時削除へフォールバックして残骸を残さない。
  Delete /REBOOTOK "$INSTDIR\WinDivert.dll"
  Delete /REBOOTOK "$INSTDIR\WinDivert64.sys"
  Delete "$INSTDIR\uninstall.exe"
  RMDir /REBOOTOK "$INSTDIR"

  Delete "$SMPROGRAMS\${APPNAME}.lnk"
  Delete "$DESKTOP\${APPNAME}.lnk"

  ; アプリ data（設定・name_cache・選択UID・ウィンドウ位置）を削除。
  ; 旧 Tauri 版は Roaming に座標が残り画面外復元の原因になったため確実に消す。
  RMDir /r "${DATADIR}"

  DeleteRegKey HKLM "${REGKEY}"
  DeleteRegKey HKLM "Software\${APPNAME}"
SectionEnd
