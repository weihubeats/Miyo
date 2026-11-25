//! Miyo Tauri 2 应用入口：装配 miyo-tauri 插件（数据库初始化 + 全部 Command）。
//!
//! 原生菜单仅 macOS：应用菜单「About Miyo」（点击后向 WebView 广播
//! `miyo://about` 事件以打开自定义关于弹窗）/「Hide Miyo」/「Quit Miyo」，
//! 另保留 Edit / View / Window 标准菜单（撤消/重做/剪切/复制/粘贴/全屏/最小化等）。
//! Windows 下隐藏原生装饰（标题栏 + 菜单栏），由前端自绘顶栏 + WindowControls 承担。
//!
//! ## Windows 黑屏兜底（issue #192）
//!
//! 后端起来、窗口显示，但前端一个像素都画不出来时，用户只能看到窗口底色。
//! 常见两因：系统只有 IE 内核（WebView2 运行时未装/被策略禁用）、旧版显卡驱动上
//! WebView2 的 GPU 合成挂掉。首帧超过 8 秒未完成时按序给出可操作的出口——
//! 二进制上不能弹网页弹窗，这里只做「不让用户干瞪眼」的最小干预：
//! 1. 启动时探测注册表里的 WebView2 运行时，缺失即以原生错误框告知；
//! 2. 页面加载完成 8 秒仍未收到前端 `miyo:frontend-ready` 时，弹原生框给出
//!    「安装 WebView2 / 关闭硬件加速 / 日志目录」三条可操作指引（不自动重载——
//!    窗口里可能已有用户数据在途，重载有副作用）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "macos")]
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};

use tauri::{Emitter, Listener, Manager};

const ABOUT_EVENT: &str = "miyo://about";

/// 前端首帧就绪事件（`main.ts` 挂载成功后广播）。
const FRONTEND_READY_EVENT: &str = "miyo:frontend-ready";

/// 页面加载完成后等待前端就绪的上限：超过即认为前端没渲染出来。
const FRONTEND_READY_TIMEOUT_SECS: u64 = 8;

/// 前端是否已就绪（`miyo:frontend-ready` 到达后置 true）。
static FRONTEND_READY: AtomicBool = AtomicBool::new(false);

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

/// Windows：系统缺 WebView2 运行时（只有 IE 内核）时，前端必然画不出来。
/// 启动时先探一次注册表，缺失直接给原生错误框——比让用户对着黑窗猜有用。
#[cfg(target_os = "windows")]
fn warn_if_webview2_missing() {
    // tauri::webview_version() 走已安装的运行时；取不到版本即视为缺失/不可用
    if let Ok(version) = tauri::webview_version() {
        tracing::info!(webview_version = %version, "webview runtime detected");
        return;
    }
    tracing::error!("WebView2 runtime not detected; the window will stay blank");
    let msg = "未检测到 Microsoft Edge WebView2 运行时，界面无法绘制（窗口会一片空白/黑屏）。\n\n\
               请安装「Microsoft Edge WebView2 Runtime」常青版后重新打开 Miyo：\n\
               https://developer.microsoft.com/microsoft-edge/webview2/\n\n\
               安装过程中企业策略可能拦截 WebView2 安装（Windows Server 场景常见），\
               需要管理员在组策略中允许后重试。";
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Miyo 缺少 WebView2 运行时")
        .set_description(msg)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
}

/// 首帧看门狗：页面加载完成 8 秒后仍没有 `miyo:frontend-ready`，说明前端没渲染出来。
/// 给原生错误框（带日志路径与三条可操作建议），并写日志——这类问题用户往往
/// 连「设置」都点不到，日志路径必须由程序自己告诉用户。
fn spawn_first_frame_watchdog(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(FRONTEND_READY_TIMEOUT_SECS));
        if FRONTEND_READY.load(Ordering::SeqCst) {
            return;
        }
        let log_dir = miyo_storage::db::log_dir();
        tracing::error!(
            log_dir = %log_dir.display(),
            "frontend did not report ready within {}s (blank window)",
            FRONTEND_READY_TIMEOUT_SECS
        );
        let msg = format!(
            "界面在 {} 秒内没有完成加载，窗口可能是空白/黑屏。\n\n\
             1) 安装或修复 Microsoft Edge WebView2 Runtime：\n\
             https://developer.microsoft.com/microsoft-edge/webview2/\n\
             2) 若系统较旧或使用远程桌面/虚拟显卡，可尝试关闭硬件加速：\n\
             创建名为 MIYO_WEBVIEW_SOFTWARE 的环境变量，值设为 1，然后重新打开 Miyo。\n\
             3) 仍无法使用请把以下目录里的日志发给开发者：\n\n{}\n\n\
             窗口不会退出，可能仍可继续操作。",
            FRONTEND_READY_TIMEOUT_SECS,
            log_dir.display()
        );
        let _ = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title("Miyo 界面加载超时")
            .set_description(&msg)
            .set_buttons(rfd::MessageButtons::Ok)
            .show();
        let _ = app.emit("miyo://frontend-timeout", ());
    });
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
            // 前端就绪信号（main.ts 挂载后广播）：看门狗据此判定「真的画出来了没有」
            app.listen_any(FRONTEND_READY_EVENT, |_event| {
                FRONTEND_READY.store(true, Ordering::SeqCst);
                tracing::info!("frontend ready");
            });
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
                    warn_if_webview2_missing();
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
                spawn_first_frame_watchdog(app.handle().clone());
                // 窗口以 visible:false 创建：装饰与状态恢复完成后再显示，
                // 避免 Windows 启动时闪现原生标题栏。
                let _ = win.show();
                let _ = win.set_focus();
            }
            Ok(())
        })
        .on_page_load(|webview, payload| {
            // 前端就绪由 main.ts 广播；每次重新加载前先复位
            if webview.label() == "main" {
                let phase = match payload.event() {
                    tauri::webview::PageLoadEvent::Started => "started",
                    tauri::webview::PageLoadEvent::Finished => "finished",
                };
                if phase == "started" {
                    FRONTEND_READY.store(false, Ordering::SeqCst);
                }
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
