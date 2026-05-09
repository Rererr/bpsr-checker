# bpsr-checker

Blue Protocol: Star Resonance 向けの軽量 DPS チェッカー (Windows 専用)

## インストール

[Releases](https://github.com/Rererr/bpsr-checker/releases) から最新の `bpsr-checker_x.x.x_x64-setup.exe` をダウンロードして実行してください。
アップデート時は起動中のアプリを終了しなくてもインストール可能です。

## Windows セキュリティ警告について

> **「WindowsによってPCが保護されました」** の警告が表示される場合

アプリへの署名 (コードサイニング) を進めていますが、reputation が蓄積されるまでの過渡期には SmartScreen 警告が表示されることがあります。

**回避手順:**
1. 「詳細情報」をクリック
2. 「実行」ボタンをクリック

なお、ウイルス対策ソフトが WinDivert ドライバ (パケットキャプチャに使用) を検出することがあります。誤検知であるため、除外設定を追加してください。

## 動作要件

- Windows 10 / 11 (x64)
- 管理者権限 (WinDivert カーネルドライバのインストールに必要)

## 使い方

- アプリ起動後、ゲームをプレイするとダメージが自動検出されます。
- `Ctrl+Shift+Z`: ウィンドウのクリックスルーを無効化 (操作できる状態に戻す)
- タスクトレイアイコン: 右クリックでメニュー (終了など)

## License

MIT