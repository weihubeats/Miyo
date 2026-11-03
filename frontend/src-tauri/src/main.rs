//! Miyo Tauri 2 应用入口：装配 miyo-tauri 插件（数据库初始化 + 全部 Command）。
//!
//! 原生菜单仅 macOS：应用菜单「About Miyo」（点击后向 WebView 广播
//! `miyo://about` 事件以打开自定义关于弹窗）/「Hide Miyo」/「Quit Miyo」，
//! 另保留 Edit / View / Window 标准菜单（撤消/重做/剪切/复制/粘贴/全屏/最小化等）。
//! Windows 下隐藏原生装饰（标题栏 + 菜单栏），由前端自绘顶栏 + WindowControls 承担。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{Emitter, Manager};

const ABOUT_EVENT: &str = "miyo://about";

#[cfg(target_os = "macos")]
fn build_menu(app: &tauri::App) -> tauri::Result<Menu<tauri::Wry>> {
    let about = MenuItem::with_id(app, "about", "About Miyo", true, None::<&str>)?;
    let app_menu = Submenu::with_items(
        app,
        "Miyo",
        true,
        &[
            &about,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some("Hide Miyo"))?,
            &PredefinedMenuItem::hide_others(app, Some("Hide Others"))?,
            &PredefinedMenuItem::show_all(app, Some("Show All"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some("Quit Miyo"))?,
        ],
    )?;

    let edit_menu = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, Some("Undo"))?,
            &PredefinedMenuItem::redo(app, Some("Redo"))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some("Cut"))?,
            &PredefinedMenuItem::copy(app, Some("Copy"))?,
            &PredefinedMenuItem::paste(app, Some("Paste"))?,
            &PredefinedMenuItem::select_all(app, Some("Select All"))?,
        ],
    )?;

    let view_menu = {
        let fullscreen = PredefinedMenuItem::fullscreen(app, Some("Toggle Full Screen"))?;
        #[cfg(debug_assertions)]
        {
            let devtools = MenuItem::with_id(
                app,
                "toggle-devtools",
                "Toggle Developer Tools",
                true,
                Some("CmdOrCtrl+Alt+I"),
            )?;
            Submenu::with_items(app, "View", true, &[&fullscreen, &devtools])?
        }
        #[cfg(not(debug_assertions))]
        {
            Submenu::with_items(app, "View", true, &[&fullscreen])?
        }
    };

    let window_menu = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some("Minimize"))?,
            &PredefinedMenuItem::close_window(app, Some("Close Window"))?,
        ],
    )?;

    Menu::with_items(app, &[&app_menu, &edit_menu, &view_menu, &window_menu])
}

fn main() {
    tauri::Builder::default()
        .plugin(miyo_tauri::plugin::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 原生菜单仅 macOS：Windows/Linux 下菜单栏渲染在标题栏里，
            // 与应用自绘顶栏割裂；Windows 走无边框窗口 + 前端 WindowControls。
            #[cfg(target_os = "macos")]
            app.set_menu(build_menu(app)?)?;
            #[cfg(target_os = "windows")]
            {
                if let Some(win) = app.get_webview_window("main") {
                    let _ = win.set_decorations(false);
                }
            }
            // 窗口以 visible:false 创建：装饰调整完成后再显示，
            // 避免 Windows 启动时闪现原生标题栏。
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.show();
            }
            Ok(())
        })
        .on_menu_event(|app, event| {
            if event.id().0 == "about" {
                let _ = app.emit(ABOUT_EVENT, ());
            } else if event.id().0 == "toggle-devtools" {
                if let Some(window) = app.get_webview_window("main") {
                    if window.is_devtools_open() {
                        window.close_devtools();
                    } else {
                        window.open_devtools();
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Miyo");
}
