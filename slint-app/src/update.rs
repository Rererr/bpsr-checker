//! アプリ内更新（GitHub Releases）。
//!
//! - **確認**: リリース一覧の先頭（＝最新版）の `tag_name` を `CARGO_PKG_VERSION` と比較する。
//!   バージョンの正典は `slint-app/Cargo.toml` なので、比較対象は常にビルド時の値。
//!   一覧はそのまま設定パネルのバージョン選択（過去版へ戻す）にも使う。
//! - **更新**: インストーラ版のみ。setup.exe を一時ディレクトリへ落とし、GitHub API が
//!   返す `asset.digest`（"sha256:…"）と実ファイルのハッシュを照合し、**検証に通ったときだけ**
//!   `/S`（サイレント）で起動する。ポータブル版は実行中 exe の自己置換になるため対象外で、
//!   リリースページを開く導線のみを出す。
//! - **なぜ検証が要るか**: 配布物は未署名で、Defender が誤検知して隔離した場合に
//!   「壊れた/消えたファイルを実行してアプリだけ終了する」事故を起こしうる。ハッシュ一致を
//!   確認してから初めて自プロセスを終了する順序をこのモジュールの契約とする。
//!
//! ネットワーク I/O はすべてブロッキング。呼び出しは専用スレッドから行い、結果は
//! `slint::Weak::upgrade_in_event_loop` で UI スレッドへ返すこと（main.rs 側の責務）。

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

/// リリース一覧の API（未認証で叩ける。レート制限は 60 req/h/IP なので
/// 起動時 1 回＋手動確認では到達しない）。draft は未認証では返らない。
const RELEASES_API: &str = "https://api.github.com/repos/Rererr/bpsr-checker/releases";
/// 一覧で持つ件数（設定パネルのバージョン選択に出す上限）。
pub const RELEASE_LIST_LIMIT: usize = 5;
/// ユーザーが手動でダウンロードするためのページ（ポータブル版・更新失敗時の導線）。
pub const RELEASES_PAGE: &str = "https://github.com/Rererr/bpsr-checker/releases/latest";
/// GitHub API は User-Agent 必須（無いと 403）。
const USER_AGENT: &str = concat!("bpsr-checker/", env!("CARGO_PKG_VERSION"));
/// ダウンロードの上限。配布物は 10MB 前後なので、応答が壊れていても書き続けない。
const DOWNLOAD_LIMIT: u64 = 128 * 1024 * 1024;
/// API 応答（JSON）の読み込み上限。
const API_BODY_LIMIT: usize = 4 * 1024 * 1024;
/// UI へ出すハッシュの先頭桁数（完全な値はログに残す）。
const HASH_PREVIEW_LEN: usize = 12;
/// 通信のタイムアウト（確認は短く、ダウンロードは長め）。
const CHECK_TIMEOUT_SECS: u64 = 15;
const DOWNLOAD_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("通信に失敗しました: {0}")]
    Http(String),
    #[error("応答を解釈できませんでした: {0}")]
    Parse(String),
    #[error("インストーラが見つかりませんでした")]
    NoInstaller,
    #[error("ダウンロードしたファイルの検証に失敗しました: {0}")]
    Verify(String),
    #[error("ファイル操作に失敗しました: {0}")]
    Io(#[from] std::io::Error),
    #[error("更新プログラムを起動できませんでした: {0}")]
    Launch(String),
}

impl UpdateError {
    /// 表示言語に合わせたユーザー向け文言。ja は Display（thiserror の定義）をそのまま使い、
    /// 日本語文をここに二重定義しない。ログには常に Display（日本語）を使う。
    pub fn user_message(&self) -> String {
        if crate::is_ja() {
            return self.to_string();
        }
        match self {
            UpdateError::Http(e) => format!("Network request failed: {e}"),
            UpdateError::Parse(e) => format!("Could not read the response: {e}"),
            UpdateError::NoInstaller => "No installer was found in the release.".to_string(),
            UpdateError::Verify(e) => format!("The downloaded file failed verification: {e}"),
            UpdateError::Io(e) => format!("File operation failed: {e}"),
            UpdateError::Launch(e) => format!("Could not start the updater: {e}"),
        }
    }
}

/// `major.minor.patch`。プレリリース識別子（`-rc1` 等）は比較に使わず切り捨てる。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// "v1.25.1" / "1.25.1" / "1.26.0-rc1" を受理する。3 要素に満たない・数値でない場合は None。
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').or_else(|| s.strip_prefix('V')).unwrap_or(s);
        // プレリリース/ビルドメタデータを落とす（1.26.0-rc1+abc → 1.26.0）
        let core = s.split(['-', '+']).next()?;
        let mut it = core.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some(Version(major, minor, patch))
    }

    pub fn text(&self) -> String {
        format!("{}.{}.{}", self.0, self.1, self.2)
    }
}

/// 実行中のバイナリのバージョン。`Cargo.toml` の値がビルド時に埋め込まれるため、
/// パースに失敗するのは「バージョン表記を壊したままリリースした」ビルド不変条件の破れ。
pub fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION が x.y.z 形式ではない")
}

/// リリースに添付された配布物 1 件。
#[derive(Debug, Clone)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// GitHub API の `digest`（"sha256:<hex>"）から取り出した hex。無ければ None
    /// （＝検証できないのでダウンロードして実行はしない）。
    pub sha256: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub version: Version,
    pub page_url: String,
    /// NSIS インストーラ（`bpsr-checker-setup-<ver>.exe`）。ポータブル zip は
    /// アプリ内更新の対象外（実行中 exe の自己置換になる）なので保持しない。
    pub installer: Option<Asset>,
}

impl Release {
    /// 実行中のバージョンより新しいか。
    pub fn is_newer_than_current(&self) -> bool {
        self.version > current()
    }

    /// 実行中のバージョンそのものか（バージョン選択で「現在」を示す）。
    pub fn is_current(&self) -> bool {
        self.version == current()
    }

    /// アプリ内で入れ替えられるか。インストーラが添付され、かつ SHA-256 が公開されている
    /// ことが条件（`download_verified` はハッシュが無い配布物を拒否するため、digest の無い
    /// リリースを「入れ替えられる」と見せると必ず失敗するボタンになる）。
    /// インストーラ版で動いているかは別条件で、呼び出し側が `is_installed_build()` と併せて判定する。
    pub fn can_install(&self) -> bool {
        self.installer.as_ref().is_some_and(|a| a.sha256.is_some())
    }
}

// ── GitHub API 応答（必要なフィールドのみ）──

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    /// プレリリースは配布対象ではないので一覧から落とす（draft は未認証では返らない）。
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    /// 2025 年以降の GitHub API が返す "sha256:<hex>"（実測で存在を確認済み）。
    #[serde(default)]
    digest: Option<String>,
}

impl From<ApiAsset> for Asset {
    fn from(a: ApiAsset) -> Asset {
        let sha256 = a
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .map(|h| h.to_ascii_lowercase());
        Asset { name: a.name, url: a.browser_download_url, size: a.size, sha256 }
    }
}

/// 新しい順に最大 `limit` 件のリリースを取得する（ブロッキング）。
/// 先頭が最新版。1 回の呼び出しで「最新の確認」と「バージョン選択の候補」を兼ねる。
pub fn fetch_releases(limit: usize) -> Result<Vec<Release>, UpdateError> {
    // 取りこぼしを防ぐため API へは少し多めに要求する（プレリリースを落とすため）。
    let per_page = (limit * 2).clamp(1, 100);
    let mut resp = ureq::get(format!("{RELEASES_API}?per_page={per_page}"))
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(CHECK_TIMEOUT_SECS)))
        .build()
        .call()
        .map_err(|e| UpdateError::Http(e.to_string()))?;
    let body = resp
        .body_mut()
        .with_config()
        .limit(API_BODY_LIMIT as u64)
        .read_to_string()
        .map_err(|e| UpdateError::Http(e.to_string()))?;
    parse_releases(&body, limit)
}

/// API 応答（JSON 配列）→ バージョン降順の Release 一覧。
/// 単体テストのために I/O から切り離してある。タグが x.y.z 形式でないものは無視する
/// （1 件でも壊れていたら全体を落とす、という挙動にはしない）。
fn parse_releases(body: &str, limit: usize) -> Result<Vec<Release>, UpdateError> {
    let api: Vec<ApiRelease> =
        serde_json::from_str(body).map_err(|e| UpdateError::Parse(e.to_string()))?;
    let mut out: Vec<Release> = api
        .into_iter()
        .filter(|r| !r.prerelease)
        .filter_map(|r| {
            let version = Version::parse(&r.tag_name)?;
            let installer = r.assets.into_iter().find_map(|a| {
                let lower = a.name.to_ascii_lowercase();
                (lower.ends_with(".exe") && lower.contains("setup")).then(|| a.into())
            });
            Some(Release { version, page_url: r.html_url, installer })
        })
        .collect();
    if out.is_empty() {
        return Err(UpdateError::Parse("公開されたリリースが見つかりません".to_string()));
    }
    out.sort_by(|a, b| b.version.cmp(&a.version));
    out.truncate(limit);
    Ok(out)
}

/// 実行中の exe がインストーラ版かどうか。
///
/// NSIS インストーラは `$INSTDIR\uninstall.exe` を必ず書き出すのに対し、ポータブル zip には
/// 入っていない（`scripts/package-slint.ps1` が詰めるのは exe と WinDivert のみ）。
/// レジストリを読むより判定材料が近く、失敗時は「ポータブル扱い＝手動更新の導線」に倒れる。
pub fn is_installed_build() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let Some(dir) = exe.parent() else {
        return false;
    };
    dir.join("uninstall.exe").is_file()
}

/// ダウンロード先のディレクトリ（`%TEMP%\bpsr-checker-update`）。
fn download_dir() -> PathBuf {
    std::env::temp_dir().join("bpsr-checker-update")
}

/// 配布物をダウンロードし、SHA-256 を照合してからパスを返す（ブロッキング）。
///
/// `progress(done, total)` はダウンロード中に呼ばれる。UI スレッドへ渡す前提なので
/// 呼び出し側で間引くこと。検証に失敗したファイルは削除して Err を返す
/// （＝壊れた実体を残さない）。
pub fn download_verified(
    asset: &Asset,
    mut progress: impl FnMut(u64, u64),
) -> Result<PathBuf, UpdateError> {
    let Some(expected) = asset.sha256.clone() else {
        return Err(UpdateError::Verify(
            "配布物のハッシュが公開されていないため検証できません".to_string(),
        ));
    };
    if asset.size > DOWNLOAD_LIMIT {
        return Err(UpdateError::Verify(format!(
            "配布物のサイズが想定外です（{} バイト）",
            asset.size
        )));
    }

    let dir = download_dir();
    // 前回の残骸（検証失敗・中断）を残さない。ディレクトリごと作り直す。
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&asset.name);

    let mut resp = ureq::get(&asset.url)
        .header("User-Agent", USER_AGENT)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS)))
        .build()
        .call()
        .map_err(|e| UpdateError::Http(e.to_string()))?;

    let total = asset.size;
    let mut reader = resp.body_mut().as_reader();
    let mut file = std::fs::File::create(&dest)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done: u64 = 0;
    loop {
        let n = reader.read(&mut buf).map_err(|e| UpdateError::Http(e.to_string()))?;
        if n == 0 {
            break;
        }
        done += n as u64;
        if done > DOWNLOAD_LIMIT {
            drop(file);
            let _ = std::fs::remove_file(&dest);
            return Err(UpdateError::Verify("応答が想定サイズを超えました".to_string()));
        }
        file.write_all(&buf[..n])?;
        hasher.update(&buf[..n]);
        progress(done, total);
    }
    file.flush()?;
    drop(file);

    let actual = hex_lower(&hasher.finalize());
    if actual != expected {
        let _ = std::fs::remove_file(&dest);
        // 完全なハッシュはログへ。UI のカードには 64 桁を 2 本並べても読めないので短縮する。
        log::warn!(
            "SHA-256 不一致: asset={} expected={expected} actual={actual}",
            asset.name
        );
        return Err(UpdateError::Verify(format!(
            "SHA-256 が一致しません（{}… / {}…）",
            &expected[..HASH_PREVIEW_LEN.min(expected.len())],
            &actual[..HASH_PREVIEW_LEN.min(actual.len())]
        )));
    }
    if total != 0 && done != total {
        let _ = std::fs::remove_file(&dest);
        return Err(UpdateError::Verify(format!(
            "サイズが一致しません（期待 {total} / 実際 {done}）"
        )));
    }
    Ok(dest)
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// 検証済みインストーラを `/S`（サイレント）で起動する。
///
/// 呼び出し側はこの関数が Ok を返してから自プロセスを終了すること。インストーラは
/// `.onInit` で `taskkill /F /IM bpsr-checker.exe` を行うため、こちらが残っていても
/// 強制終了されるが、その場合は保存されていない状態が失われる。
///
/// **`cmd /C start` を挟んで自プロセスのツリーから切り離す**のが要点。直接 spawn すると
/// インストーラは自プロセスの子になり、`taskkill /T`（プロセスツリー）を使う版の
/// インストーラでは**インストーラ自身が巻き添えで落ちる**。現行の `installer.nsi` からは
/// `/T` を外したが、バージョン選択で v1.25.1 以前の配布物を入れる経路が残るため、
/// 呼び出し側でも切り離しておく（`start` は起動後すぐ戻り、cmd も即終了する）。
#[cfg(windows)]
pub fn launch_installer(path: &Path) -> Result<(), UpdateError> {
    use std::os::windows::process::CommandExt;
    /// コンソール窓を出さない（release は windows subsystem なので cmd が窓を作ってしまう）。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    if !path.is_file() {
        return Err(UpdateError::Launch(format!(
            "インストーラが見つかりません: {}",
            path.display()
        )));
    }
    let Some(path_str) = path.to_str() else {
        return Err(UpdateError::Launch("インストーラのパスを解釈できません".to_string()));
    };
    // コマンドラインは raw_arg で自前に組み立てる。std の引数クォートは空白とタブしか対象に
    // しないため、`&` を含むユーザー名（Windows では作成できる）の %TEMP% を素で渡すと
    // cmd がそこでコマンドを切り、起動できないまま spawn だけ成功する。
    // "" は start のウィンドウタイトル（パスを引用符で囲むときに必須）。パスに `"` は
    // Windows のファイル名として存在しないので、この囲みは安全に閉じる。
    std::process::Command::new("cmd")
        .arg("/C")
        .raw_arg(format!("start \"\" \"{path_str}\" /S"))
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map(|_| ())
        .map_err(|e| UpdateError::Launch(e.to_string()))
}

#[cfg(not(windows))]
pub fn launch_installer(path: &Path) -> Result<(), UpdateError> {
    Err(UpdateError::Launch(format!(
        "この OS では更新を適用できません: {}",
        path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parses_tag_and_plain_forms() {
        assert_eq!(Version::parse("v1.25.1"), Some(Version(1, 25, 1)));
        assert_eq!(Version::parse("1.25.1"), Some(Version(1, 25, 1)));
        assert_eq!(Version::parse(" v1.26.0-rc1 "), Some(Version(1, 26, 0)));
        assert_eq!(Version::parse("1.25"), None);
        assert_eq!(Version::parse("1.25.1.2"), None);
        assert_eq!(Version::parse("latest"), None);
    }

    /// 文字列比較だと "1.9.0" > "1.25.1" になってしまう。数値比較であることを固定する。
    #[test]
    fn version_orders_numerically() {
        assert!(Version(1, 25, 1) > Version(1, 9, 0));
        assert!(Version(2, 0, 0) > Version(1, 99, 99));
        assert!(Version(1, 25, 2) > Version(1, 25, 1));
        assert_eq!(Version(1, 25, 1), Version(1, 25, 1));
    }

    fn release_json(tag: &str, prerelease: bool, digest: Option<&str>) -> String {
        let dig = digest.map(|d| format!(r#","digest":"{d}""#)).unwrap_or_default();
        format!(
            r#"{{"tag_name":"{tag}","html_url":"https://example.invalid/{tag}","prerelease":{prerelease},
            "assets":[
              {{"name":"bpsr-checker-portable-x.zip","browser_download_url":"https://example.invalid/p.zip","size":10{dig}}},
              {{"name":"bpsr-checker-setup-x.exe","browser_download_url":"https://example.invalid/s.exe","size":20{dig}}}
            ]}}"#
        )
    }

    #[test]
    fn parses_release_assets_and_digest() {
        let body = format!("[{}]", release_json("v1.26.0", false, Some("sha256:BBBB")));
        let list = parse_releases(&body, 5).expect("parse");
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, Version(1, 26, 0));
        let inst = list[0].installer.clone().expect("installer asset");
        assert_eq!(inst.size, 20);
        assert_eq!(inst.sha256.as_deref(), Some("bbbb")); // hex は小文字へ正規化
    }

    /// digest が無いリリース（旧アセット）では検証できないので None のまま持つ。
    #[test]
    fn missing_digest_is_none() {
        let body = format!("[{}]", release_json("v1.0.0", false, None));
        let list = parse_releases(&body, 5).expect("parse");
        assert!(list[0].installer.clone().expect("installer").sha256.is_none());
    }

    /// 一覧はバージョン降順で、プレリリースとタグ形式外は落とし、limit 件に切り詰める。
    #[test]
    fn releases_are_sorted_filtered_and_truncated() {
        let body = format!(
            "[{},{},{},{},{}]",
            release_json("v1.9.0", false, None),   // 文字列比較なら最大に見える版
            release_json("v1.26.0", false, None),
            release_json("v1.27.0", true, None),   // プレリリースは除外
            release_json("nightly", false, None),  // タグ形式外は無視
            release_json("v1.25.1", false, None),
        );
        let list = parse_releases(&body, 2).expect("parse");
        let versions: Vec<Version> = list.iter().map(|r| r.version).collect();
        assert_eq!(versions, vec![Version(1, 26, 0), Version(1, 25, 1)]);
    }

    /// 公開リリースが 1 件も読めない応答は、空一覧ではなくエラーにする
    /// （UI が「最新版です」と誤って表示しないため）。
    #[test]
    fn empty_release_list_is_an_error() {
        assert!(parse_releases("[]", 5).is_err());
        let only_pre = format!("[{}]", release_json("v2.0.0", true, None));
        assert!(parse_releases(&only_pre, 5).is_err());
    }

    /// digest の無い配布物は `download_verified` が必ず拒否するので、入れ替え可能と見せない。
    #[test]
    fn can_install_requires_installer_and_digest() {
        let with_digest = format!("[{}]", release_json("v1.26.0", false, Some("sha256:AAAA")));
        assert!(parse_releases(&with_digest, 5).unwrap()[0].can_install());

        let without_digest = format!("[{}]", release_json("v1.26.0", false, None));
        assert!(!parse_releases(&without_digest, 5).unwrap()[0].can_install());

        let no_installer = r#"[{"tag_name":"v1.26.0","html_url":"https://example.invalid","prerelease":false,
            "assets":[{"name":"bpsr-checker-portable-1.26.0.zip","browser_download_url":"https://example.invalid/p.zip","size":1,"digest":"sha256:AAAA"}]}]"#;
        assert!(!parse_releases(no_installer, 5).unwrap()[0].can_install());
    }

    #[test]
    fn hex_lower_formats_padded() {
        assert_eq!(hex_lower(&[0x00, 0x0f, 0xff]), "000fff");
    }

    /// `launch_installer` の引数組み立てを、空白を含むパスで実際に確認する
    /// （`cmd /C start "" "<path>" /S` の引用がずれると、無関係なパスを起動しかねない）。
    /// 本物のインストーラは走らせられないので、引数を書き出すバッチで代用する。
    #[cfg(windows)]
    #[test]
    #[ignore = "子プロセス（cmd）を起動する"]
    fn launch_installer_passes_silent_flag_with_spaced_path() {
        let dir = std::env::temp_dir().join("bpsr update launch test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("作業ディレクトリ");
        let marker = dir.join("args.txt");
        let bat = dir.join("bpsr-checker-setup-0.0.0.bat");
        // 受け取った引数をそのまま marker へ書き出すだけのバッチ。
        // `start` は .bat を `cmd /K`（実行後もウィンドウを残す）で開くため、最後に exit で
        // 自分を閉じる。これを忘れるとテストプロセスの標準出力を握ったまま居座り、
        // cargo test が終わらない（実際のインストーラは .exe なので /K は付かない）。
        std::fs::write(
            &bat,
            format!("@echo %* > \"{}\"\r\n@exit\r\n", marker.display()),
        )
        .expect("バッチ生成");

        launch_installer(&bat).expect("起動できること");

        // 起動は非同期なので marker ができるまで少し待つ。
        let mut text = String::new();
        for _ in 0..50 {
            if let Ok(s) = std::fs::read_to_string(&marker) {
                text = s;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(text.contains("/S"), "サイレント指定が渡ること（実際: {text:?}）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 公開中のリリースに対して、確認からダウンロード・ハッシュ照合までを実際に通す。
    /// ネットワークと数 MB のダウンロードを伴うので既定では走らせない
    /// （`cargo test -p bpsr-app -- --ignored downloads_published_installer` で手動実行）。
    #[test]
    #[ignore = "ネットワークアクセスと数MBのダウンロードを伴う"]
    fn downloads_published_installer_and_verifies_digest() {
        let list = fetch_releases(RELEASE_LIST_LIMIT).expect("リリース一覧を取得できること");
        assert!(!list.is_empty(), "公開リリースが 1 件以上あること");
        let asset = list[0].installer.clone().expect("インストーラが添付されていること");
        assert!(asset.sha256.is_some(), "GitHub API が digest を返すこと");
        let mut seen_progress = false;
        let path = download_verified(&asset, |done, total| {
            assert!(done <= total.max(done));
            seen_progress = true;
        })
        .expect("ダウンロードと SHA-256 照合が通ること");
        assert!(seen_progress, "進捗コールバックが呼ばれること");
        let len = std::fs::metadata(&path).expect("保存されたファイル").len();
        assert_eq!(len, asset.size, "保存サイズが公開値と一致すること");
        let _ = std::fs::remove_dir_all(download_dir());
    }
}
