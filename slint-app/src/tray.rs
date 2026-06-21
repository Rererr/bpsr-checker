//! タスクトレイ（Windows）。クリックスルー切替・メイン表示/非表示・終了を提供。
//! Slint の winit イベントループがメッセージを汲むため、トレイは初回 Timer tick
//! （ループ稼働後）に生成し、メニューイベントはポーリングで処理する。

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub struct Tray {
    // TrayIcon はドロップするとアイコンが消えるため保持し続ける。
    _tray: TrayIcon,
    pub click_through: CheckMenuItem,
    pub id_click_through: MenuId,
    pub id_show_hide: MenuId,
    pub id_quit: MenuId,
}

/// トレイを生成（イベントループ稼働後に呼ぶこと）。失敗時は None。
/// メニュー文言は表示言語に追従（ja 以外は en。zh/ko は保留中のため en）。
pub fn create() -> Option<Tray> {
    use bpsr_core::engine::runtime_settings::{self, Lang};
    let ja = runtime_settings::display_lang() == Lang::Ja;
    let (s_click, s_show, s_quit) = if ja {
        ("クリックスルー", "メインを表示/非表示", "終了")
    } else {
        ("Click-through", "Show / Hide Main", "Quit")
    };
    let menu = Menu::new();
    let click_through = CheckMenuItem::new(s_click, true, false, None);
    let show_hide = MenuItem::new(s_show, true, None);
    let quit = MenuItem::new(s_quit, true, None);

    let res = menu
        .append(&click_through)
        .and_then(|_| menu.append(&PredefinedMenuItem::separator()))
        .and_then(|_| menu.append(&show_hide))
        .and_then(|_| menu.append(&PredefinedMenuItem::separator()))
        .and_then(|_| menu.append(&quit));
    if let Err(e) = res {
        log::warn!("tray menu append failed: {e}");
        return None;
    }

    let id_click_through = click_through.id().clone();
    let id_show_hide = show_hide.id().clone();
    let id_quit = quit.id().clone();

    let tray = match TrayIconBuilder::new()
        .with_tooltip("bpsr-checker")
        .with_menu(Box::new(menu))
        .with_icon(load_icon())
        .build()
    {
        Ok(t) => t,
        Err(e) => {
            log::warn!("tray build failed: {e}");
            return None;
        }
    };

    Some(Tray {
        _tray: tray,
        click_through,
        id_click_through,
        id_show_hide,
        id_quit,
    })
}

/// 埋め込みリソース(id=1)のアイコン。失敗時はアクセント色の単色アイコン。
fn load_icon() -> Icon {
    if let Ok(icon) = Icon::from_resource(1, Some((32, 32))) {
        return icon;
    }
    let mut rgba = vec![0u8; 16 * 16 * 4];
    for px in rgba.chunks_mut(4) {
        px[0] = 0x4f;
        px[1] = 0xc3;
        px[2] = 0xf7;
        px[3] = 0xff;
    }
    Icon::from_rgba(rgba, 16, 16).expect("fallback tray icon")
}
