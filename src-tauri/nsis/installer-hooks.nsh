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
!macroend
