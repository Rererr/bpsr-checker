; WinDivert カーネルドライバーをインストール前に停止し、
; ファイルロックによる「無視」ダイアログを防ぐ。
; preInit は .onInit (管理者権限昇格済み) の最初に呼ばれる。
!macro preInit
  nsExec::ExecToLog 'sc stop WinDivert'
  nsExec::ExecToLog 'sc delete WinDivert'
  Sleep 2000
!macroend

; アンインストール時も同様にドライバーを停止してからファイルを削除する
!macro customUnInstall
  nsExec::ExecToLog 'sc stop WinDivert'
  nsExec::ExecToLog 'sc delete WinDivert'
  Sleep 2000
!macroend
