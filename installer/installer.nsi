; bpsr-checker (Slint版) スタンドアロン NSIS インストーラ。
; Tauri 生成の NSIS を置き換える。WinDivert 同梱・管理者インストール・
; インストール/アンインストール時にアプリ強制終了＋WinDivertドライバ停止。
;
; ビルド: makensis を入れて
;   makensis /DSRCDIR=..\dist-slint\bpsr-checker /DVERSION=0.8.20 installer.nsi
; SRCDIR には bpsr-checker.exe / WinDivert.dll / WinDivert64.sys を置いておく
; （scripts/package-slint.ps1 が用意する）。

Unicode true

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

!include "MUI2.nsh"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "Japanese"
!insertmacro MUI_LANGUAGE "English"

; アプリ強制終了＋WinDivertドライバ停止・削除（ベストエフォート・失敗しても続行）。
!macro KillAppAndDriver
  nsExec::ExecToLog 'taskkill /F /IM ${EXENAME} /T'
  Sleep 500
  nsExec::ExecToLog 'sc stop WinDivert'
  Sleep 1000
  nsExec::ExecToLog 'sc stop WinDivert'
  Sleep 1000
  nsExec::ExecToLog 'sc delete WinDivert'
  Sleep 500
!macroend

Function .onInit
  !insertmacro KillAppAndDriver
FunctionEnd

Section "Install"
  SetOutPath "$INSTDIR"
  File "${SRCDIR}\${EXENAME}"
  File "${SRCDIR}\WinDivert.dll"
  File "${SRCDIR}\WinDivert64.sys"

  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateDirectory "$SMPROGRAMS\${APPNAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk" "$INSTDIR\${EXENAME}"
  CreateShortcut "$SMPROGRAMS\${APPNAME}\Uninstall.lnk" "$INSTDIR\uninstall.exe"

  WriteRegStr HKLM "Software\${APPNAME}" "InstallDir" "$INSTDIR"
  WriteRegStr HKLM "${REGKEY}" "DisplayName" "${APPNAME}"
  WriteRegStr HKLM "${REGKEY}" "DisplayVersion" "${VERSION}"
  WriteRegStr HKLM "${REGKEY}" "Publisher" "rererr"
  WriteRegStr HKLM "${REGKEY}" "DisplayIcon" "$INSTDIR\${EXENAME}"
  WriteRegStr HKLM "${REGKEY}" "UninstallString" "$INSTDIR\uninstall.exe"
  WriteRegDWORD HKLM "${REGKEY}" "NoModify" 1
  WriteRegDWORD HKLM "${REGKEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  !insertmacro KillAppAndDriver

  Delete "$INSTDIR\${EXENAME}"
  Delete "$INSTDIR\WinDivert.dll"
  Delete "$INSTDIR\WinDivert64.sys"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"

  Delete "$SMPROGRAMS\${APPNAME}\${APPNAME}.lnk"
  Delete "$SMPROGRAMS\${APPNAME}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\${APPNAME}"

  ; アプリ data（設定・name_cache・選択UID・ウィンドウ位置）を削除。
  ; 旧 Tauri 版は Roaming に座標が残り画面外復元の原因になったため確実に消す。
  RMDir /r "${DATADIR}"

  DeleteRegKey HKLM "${REGKEY}"
  DeleteRegKey HKLM "Software\${APPNAME}"
SectionEnd
