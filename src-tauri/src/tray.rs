// System tray — sempre presente. Menu com:
//   Mostrar painel | Iniciar com Windows (toggle) | Abrir site Ecxon | Sair

use tauri::{
    image::Image,
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

const SITE_URL: &str = "https://www.ecxon.com.br";

// Ícone embarcado no binário (32x32 PNG) — funciona em release sem depender
// de path runtime do bundle. default_window_icon() pode ser None em release.
const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app)?;

    let icon = Image::from_bytes(TRAY_ICON_PNG)
        .unwrap_or_else(|_| app.default_window_icon().cloned().expect("sem ícone"));

    let _tray = TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .icon_as_template(false)
        .tooltip("Ecxon Connect")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main(app),
            "autostart" => toggle_autostart(app),
            "site" => {
                let _ = app.opener().open_url(SITE_URL, None::<&str>);
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Click esquerdo no ícone -> mostrar a janela.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn build_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let show = MenuItem::with_id(app, "show", "Mostrar painel", true, None::<&str>)?;
    let autostart_enabled = app
        .autolaunch()
        .is_enabled()
        .unwrap_or(false);
    let autostart = CheckMenuItem::with_id(
        app,
        "autostart",
        "Iniciar com Windows",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let site = MenuItem::with_id(app, "site", "Abrir site Ecxon", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Sair", true, None::<&str>)?;

    Menu::with_items(app, &[&show, &autostart, &site, &sep, &quit])
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn toggle_autostart(app: &AppHandle) {
    let manager = app.autolaunch();
    let enabled = manager.is_enabled().unwrap_or(false);
    let result = if enabled {
        manager.disable()
    } else {
        manager.enable()
    };
    if let Err(e) = result {
        log::error!("falha ao alternar autostart: {e}");
    }
}
