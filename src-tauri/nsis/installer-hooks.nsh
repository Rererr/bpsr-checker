; bpsr-checker.exe を強制終了し WinDivert ドライバを停止・削除する共通マクロ。
; 失敗してもインストールは続行する (ベストエフォート)。
!macro killAppAndDriver
  ; 1. アプリ本体を強制終了してハンドルを解放させる
  nsExec::ExecToLog 'taskkill /F /IM bpsr-checker.exe /T'
  Sleep 500

  ; 2. WinDivert ドライバを停止・削除 (2回試行)
  nsExec::ExecToLog 'sc stop WinDivert'
  Sleep 1000
  nsExec::ExecToLog 'sc stop WinDivert'
  Sleep 1000
  nsExec::ExecToLog 'sc delete WinDivert'
  Sleep 500
!macroend

!macro preInit
  !insertmacro killAppAndDriver
!macroend

!macro customUnInstall
  !insertmacro killAppAndDriver

  ; Roaming 配下の window-state を削除する。
  ; Tauri 標準の "Delete the application data" は Local/WebView2 しか消さず、
  ; ウインドウ座標を保持する .window-state.json が Roaming に残り続けるため、
  ; 旧座標(画面外/混在DPI由来の不可視化)を再インストール後も復元してしまう。
  Delete "$APPDATA\com.rererr.bpsr-checker\.window-state.json"
  RMDir "$APPDATA\com.rererr.bpsr-checker"
!macroend
