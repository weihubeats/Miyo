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

#[cfg(target_os = "windows")]
mod windows_compat {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_REMOTESESSION};

    /// Windows 兼容配置：识别旧系统 / Server / RDP 场景，按需回退软件合成与安全窗口样式。
    pub struct CompatProfile {
        pub os_major: u32,
        pub os_minor: u32,
        pub os_build: u32,
        pub is_server: bool,
        pub is_rdp: bool,
        pub software_webview: bool,
        pub native_decorations: bool,
    }

    fn parse_bool_env(key: &str) -> Option<bool> {
        let val = std::env::var(key).ok()?;
        match val.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        }
    }

    pub fn detect() -> CompatProfile {
        let ver = windows_version::OsVersion::current();
        let is_server = windows_version::is_server();
        // 远程桌面会话常走虚拟显示驱动，DirectComposition / D3D11 合成易黑屏。
        let is_rdp = unsafe { GetSystemMetrics(SM_REMOTESESSION) != 0 };
        // Server（含 2016/14393）、Win10 < 1809（17763）或 RDP 默认启用软件渲染兜底，
        // 规避旧系统/虚拟显卡上 WebView2 GPU 合成或 DirectComposition 导致全黑。
        let legacy_or_virtual =
            is_server || ver.major < 10 || (ver.major == 10 && ver.build < 17763) || is_rdp;
        let software_webview = parse_bool_env("MIYO_WEBVIEW_SOFTWARE").unwrap_or(legacy_or_virtual);
        let native_decorations = parse_bool_env("MIYO_NATIVE_DECORATIONS").unwrap_or(false);

        CompatProfile {
            os_major: ver.major,
            os_minor: ver.minor,
            os_build: ver.build,
            is_server,
            is_rdp,
            software_webview,
            native_decorations,
        }
    }
}

fn main() {
    // 排除 VISIBLE 与 DECORATIONS：由 main.rs 显式控制装饰与展示时机，
    // 防止异常退出残存 visible:false 或旧版 decorated:true 覆写启动状态。
    let window_state_flags = tauri_plugin_window_state::StateFlags::all().difference(
        tauri_plugin_window_state::StateFlags::VISIBLE
            | tauri_plugin_window_state::StateFlags::DECORATIONS,
    );

    tauri::Builder::default()
        .plugin(miyo_tauri::plugin::init())
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(window_state_flags)
                .build(),
        )
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 原生菜单仅 macOS：Windows/Linux 下菜单栏渲染在标题栏里，
            // 与应用自绘顶栏割裂；Windows 走无边框窗口 + 前端 WindowControls。
            #[cfg(target_os = "macos")]
            app.set_menu(build_menu(app)?)?;

            let webview_ver = tauri::webview_version()
                .unwrap_or_else(|err| format!("unavailable: {err}"));

            // tauri.conf.json 中 main 设为 create: false，统一在 setup 内按平台兼容策略构建：
            // 避免先建窗再改 decorations 触发的二次样式切换，也便于按系统环境注入 WebView2 启动参数。
            if let Some(main_config) = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .cloned()
            {
                #[allow(unused_mut)]
                let mut builder = tauri::WebviewWindowBuilder::from_config(app.handle(), &main_config)?;

                #[cfg(target_os = "windows")]
                {
                    let compat = windows_compat::detect();
                    tracing::info!(
                        os = %format!("{}.{}.{}", compat.os_major, compat.os_minor, compat.os_build),
                        is_server = compat.is_server,
                        is_rdp = compat.is_rdp,
                        software_webview = compat.software_webview,
                        native_decorations = compat.native_decorations,
                        webview_version = %webview_ver,
                        "windows main window compat profile"
                    );
                    builder = builder.decorations(compat.native_decorations);
                    if compat.software_webview {
                        // wry 一旦覆写 additional_browser_args 就必须自带默认 disable-features；
                        // 且不同浏览器参数的 WebView2 实例不能共用同一 data_directory（否则创建失败）。
                        builder = builder
                            .additional_browser_args(
                                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-gpu --disable-gpu-compositing",
                            )
                            .shadow(false);
                        if let Ok(local_dir) = app.path().app_local_data_dir() {
                            builder = builder.data_directory(local_dir.join("webview2-software"));
                        }
                    }
                }
                #[cfg(not(target_os = "windows"))]
                tracing::info!(webview_version = %webview_ver, "main window startup");

                let win = builder.build()?;
                if win.is_minimized().unwrap_or(false) {
                    let _ = win.unminimize();
                }
                // 窗口以 visible:false 创建：装饰与状态恢复完成后再显示，
                // 避免 Windows 启动时闪现原生标题栏。
                let _ = win.show();
                let _ = win.set_focus();
            }
            Ok(())
        })
        .on_page_load(|webview, payload| {
            if webview.label() == "main" {
                let phase = match payload.event() {
                    tauri::webview::PageLoadEvent::Started => "started",
                    tauri::webview::PageLoadEvent::Finished => "finished",
                };
                tracing::info!(phase, url = %payload.url(), "main webview page load");
            }
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
        .build(tauri::generate_context!())
        .expect("error while building Miyo")
        .run(|app, event| {
            // 应用退出：停 Agent 控制面——先取消全部在途异步运行（给后台任务
            // 落库窗口），再清端口文件（避免残留失效的 agent-port）。
            // 同时清理 Java Attach：尽力 agent.stop（停目标 JVM 内 IPC 服务 +
            // 清 infoFile）并移除全部会话；目标 JVM 本身不受影响。
            if matches!(event, tauri::RunEvent::ExitRequested { .. }) {
                tauri::async_runtime::block_on(miyo_tauri::commands::agent::shutdown_on_exit(app));
                tauri::async_runtime::block_on(
                    miyo_tauri::commands::java_attach::shutdown_attach_on_exit(app),
                );
                // 常驻插件 sidecar：进程组全量终止（防残留）。
                // run 闭包非 async：经 block_on 驱动；try_state 需要 Manager trait。
                tauri::async_runtime::block_on(async {
                    if let Some(state) = app.try_state::<miyo_tauri::state::AppState>() {
                        state.sidecar.shutdown_all().await;
                    }
                });
                // 统一 TaskManager：在途任务全量取消（WS/SSE/gRPC/AI/压测/场景/
                // Mock 等此前不在退出清理内的类别自此全覆盖）。
                if let Some(state) = app.try_state::<miyo_tauri::state::AppState>() {
                    state.tasks.shutdown_all();
                }
            }
        });
}
