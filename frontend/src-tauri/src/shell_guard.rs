//! 渲染活性守卫（issue #192 复盘）。
//!
//! ## 为什么需要这一层
//!
//! v0.0.66 的兜底只看「前端有没有上报就绪」。实测在 Windows Server 2016 +
//! VMware 上出现的是另一种故障：
//!
//! ```text
//! page load finished  url=http://tauri.localhost/
//! frontend ready                    ← 准时到达，8 秒看门狗永不触发
//! ```
//!
//! 页面渲染完了、CPU 归零、窗口一个像素都不刷新（只有首帧的 fixed 层可见）。
//! 用户的角度就是「全黑，连设置都点不到」。
//!
//! 本模块补三件事：
//!
//! 1. **渲染会话监控**：挂 WebView2 的 `ProcessFailed`，把
//!    「渲染进程退出 / 无响应 / 浏览器进程退出」写进日志（以前完全无记录）；
//! 2. **心跳停滞检测**：前端在 `requestAnimationFrame` 里周期调用
//!    `render_heartbeat`；计数停滞超过阈值即判定「画面冻结」，
//!    按 [重绘 → 软件合成重载 → 原生提示] 三级降级；
//! 3. **诊断落盘**：判定冻结时写 `blank-screen-<时间>.txt`（日志目录），
//!    用户点不动 UI 也能把文件发出来。

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

/// 心跳停滞多久算「画面冻结」。跨渲染进程重启（重载页面）需重新起算，故取 15s。
const HEARTBEAT_STALL_SECS: u64 = 15;

/// 心跳轮询间隔。
const HEARTBEAT_POLL_MS: u64 = 1500;

/// 前端就绪后先给一点缓冲，避免页面刚挂载、rAF 尚未跑起来的假冻结。
const HEARTBEAT_WARMUP_SECS: u64 = 5;

/// 渲染进程无响应（不是退出）时，留给 Chromium 自己恢复的宽限期。
const UNRESPONSIVE_GRACE_SECS: u64 = 10;

/// 已经执行过的降级级别（避免同一进程里反复弹框）。
static RECOVERY_STAGE: AtomicU8 = AtomicU8::new(0);

/// 本轮渲染会话是否已上报过「无响应」（用于宽限计时）。
static UNRESPONSIVE_SINCE: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// 是否已经用软件合成重建过一次窗口（只允许一次，避免无限闪窗）。
static SOFTWARE_RETRY_DONE: AtomicBool = AtomicBool::new(false);

/// 是否已经写过一次诊断文件（一次进程只写一份，避免刷目录）。
static REPORTED: AtomicBool = AtomicBool::new(false);

/// 降级级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// 什么都没做。
    None,
    /// 已尝试强制重绘（settle 一次 invalidate）。
    Repainted,
    /// 已用软件合成参数重载过一次。
    ReloadedSoftware,
    /// 已提示用户并落盘诊断。
    Reported,
}

impl Stage {
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Stage::Repainted,
            2 => Stage::ReloadedSoftware,
            3 => Stage::Reported,
            _ => Stage::None,
        }
    }
}

fn stage() -> Stage {
    Stage::from_u8(RECOVERY_STAGE.load(Ordering::SeqCst))
}

/// 推进到下一级（CAS，多线程下只生效一次）。返回是否由本线程完成推进。
fn advance_to(next: Stage) -> bool {
    let mut current = RECOVERY_STAGE.load(Ordering::SeqCst);
    loop {
        let cur_stage = Stage::from_u8(current);
        if cur_stage as u8 >= next as u8 {
            return false;
        }
        match RECOVERY_STAGE.compare_exchange(
            current,
            next as u8,
            Ordering::SeqCst,
            Ordering::SeqCst,
        ) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

/// 启动渲染守卫：渲染会话监控 + 心跳停滞看门狗。
///
/// 在 `setup` 里主窗构建完成后调用一次。非 Windows 平台也跑——绝大多数用户
/// 遇不到，但一旦遇到，日志里至少要有一行可依。
pub fn install(app: &AppHandle) {
    install_render_session_monitor(app);
    spawn_heartbeat_watchdog(app.clone());
}

/// 渲染会话监控：把 Chromium 上报的进程失败事件写进日志。
///
/// `WebviewWindow::with_webview` 的闭包只在 WebView2 环境里拿到
/// `ICoreWebView2_2`（`ProcessFailed` 注册在 2 及以上版本的接口上）。
fn install_render_session_monitor(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        tracing::warn!("render guard: main window not found, monitor skipped");
        return;
    };
    let handle = app.clone();
    // with_webview 在非 WebView2 后端也可能返回 Err：仅记日志，不阻断启动。
    if let Err(err) = window.with_webview(move |webview| {
        // 非 Windows / 取不到 CoreWebView2 时静默跳过：心跳看门狗仍然生效。
        #[cfg(target_os = "windows")]
        {
            use webview2_com::Microsoft::Web::WebView2::Win32::{
                ICoreWebView2ProcessFailedEventArgs2, ICoreWebView2_2,
                COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED,
                COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE,
                COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED,
            };
            use webview2_com::ProcessFailedEventHandler;
            use windows_core::Interface;

            /// 把 WebView2 返回的 `PWSTR` 转成 Rust 字符串并释放 COM 内存。
            ///
            /// SAFETY: 只接受由 WebView2/COM 分配的 `PWSTR`（可用 CoTaskMemFree 释放）；
            /// 空指针返回空串。
            unsafe fn pwstr_to_string(raw: windows_core::PWSTR) -> String {
                if raw.is_null() {
                    return String::new();
                }
                let text = raw.to_string().unwrap_or_default();
                windows_sys::Win32::System::Com::CoTaskMemFree(raw.0 as *const core::ffi::c_void);
                text
            }

            // SAFETY: 回调只在 WebView2 消息循环内触发，闭包捕获的均为 Send 值。
            let core = match unsafe {
                webview
                    .controller()
                    .CoreWebView2()
                    .and_then(|core| core.cast::<ICoreWebView2_2>())
            } {
                Ok(core) => core,
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "render guard: CoreWebView2_2 unavailable, process monitor skipped"
                    );
                    return;
                }
            };

            let app_for_event = handle.clone();
            let mut token = 0i64;
            let handler = ProcessFailedEventHandler::create(Box::new(move |_sender, args| {
                // 只关心「退出」与「无响应」两类；其余（如 GPU 进程退出）交给 Chromium 自恢复。
                let Some(args) = args else { return Ok(()) };
                // 这些接口都是 `&mut out` 出参形态（webview2-com 0.39），
                // 不是返回值——按返回值调用会在 Windows 上直接编译失败。
                let kind = {
                    let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED;
                    // SAFETY: kind 是栈上的可写出参，生命周期覆盖调用。
                    if unsafe { args.ProcessFailedKind(&mut kind) }.is_err() {
                        kind = COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED;
                    }
                    kind
                };
                let detail = {
                    // SAFETY: 出参均为栈上可写值；PWSTR 由 CoTaskMemFree 释放。
                    unsafe {
                        args.cast::<ICoreWebView2ProcessFailedEventArgs2>()
                            .map(|args2| {
                                let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON_UNEXPECTED;
                                let mut exit = -1i32;
                                let mut desc = windows_core::PWSTR::null();
                                let _ = args2.Reason(&mut reason);
                                let _ = args2.ExitCode(&mut exit);
                                let _ = args2.ProcessDescription(&mut desc);
                                let desc = pwstr_to_string(desc);
                                format!("reason={reason:?} exit={exit} desc={desc}")
                            })
                            .unwrap_or_default()
                    }
                };
                tracing::error!(
                    kind = kind.0,
                    detail = %detail,
                    "webview render process failed"
                );
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_RENDER_PROCESS_UNRESPONSIVE {
                    *UNRESPONSIVE_SINCE.lock().unwrap() = Some(Instant::now());
                }
                if kind == COREWEBVIEW2_PROCESS_FAILED_KIND_BROWSER_PROCESS_EXITED {
                    // 浏览器进程没了，窗口里已没有任何能跑的画布：直接提示。
                    report_and_notify(
                        &app_for_event,
                        "WebView2 浏览器进程异常退出（BrowserProcessExited）",
                    );
                }
                Ok(())
            }));
            // SAFETY: core 来自已创建的 WebView2 控制器，token 生命周期跟随控制器。
            if let Err(err) = unsafe { core.add_ProcessFailed(&handler, &mut token) } {
                tracing::warn!(error = %err, "render guard: add_ProcessFailed failed");
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = &webview;
            let _ = &handle;
        }
    }) {
        tracing::warn!(error = %err, "render guard: with_webview failed, monitor skipped");
    }
}

/// 心跳看门狗：前端就绪后周期性比较 `render_heartbeat` 计数。
///
/// 计数长期不变（且进程仍活着）即说明 `requestAnimationFrame` 不再被驱动——
/// 合成器输出没有呈现到窗口上。按三级降级处理。
fn spawn_heartbeat_watchdog(app: AppHandle) {
    std::thread::spawn(move || {
        // 等前端就绪（最多等 60s），就绪后再给 warmup 缓冲。
        let ready_deadline = Instant::now() + Duration::from_secs(60);
        while !crate::FRONTEND_READY.load(Ordering::SeqCst) {
            if Instant::now() > ready_deadline {
                // 前端压根没上报：8 秒看门狗报文负责，这里不重复。
                return;
            }
            std::thread::sleep(Duration::from_millis(HEARTBEAT_POLL_MS));
        }
        std::thread::sleep(Duration::from_secs(HEARTBEAT_WARMUP_SECS));

        let mut last_count = miyo_tauri::commands::render_heartbeat_count();
        let mut last_change = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(HEARTBEAT_POLL_MS));
            let count = miyo_tauri::commands::render_heartbeat_count();
            if count != last_count {
                last_count = count;
                last_change = Instant::now();
                continue;
            }

            // 渲染进程「无响应」但心跳停了：先等宽限期，别急着弹框。
            if let Some(since) = *UNRESPONSIVE_SINCE.lock().unwrap() {
                if since.elapsed() < Duration::from_secs(UNRESPONSIVE_GRACE_SECS) {
                    continue;
                }
            }

            if last_change.elapsed() < Duration::from_secs(HEARTBEAT_STALL_SECS) {
                continue;
            }

            tracing::error!(
                stalled_secs = last_change.elapsed().as_secs(),
                heartbeat = count,
                stage = ?stage(),
                "frontend animation frames stalled (window is probably blank)"
            );

            match stage() {
                Stage::None => {
                    advance_to(Stage::Repainted);
                    repaint(&app);
                }
                Stage::Repainted => {
                    advance_to(Stage::ReloadedSoftware);
                    reload_with_software_compositing(&app);
                    // 重载后重新起算，给新会话一个完整的宽限窗口。
                    last_change = Instant::now();
                }
                Stage::ReloadedSoftware | Stage::Reported => {
                    advance_to(Stage::Reported);
                    report_and_notify(&app, "界面连续多个心跳周期没有刷新（渲染已冻结）");
                    // 只提示一次，之后静默轮询（不再刷日志、不再弹框）。
                    return;
                }
            }
        }
    });
}

/// 一级：强制重绘。
///
/// 合成挂死时，窗口尺寸抖动一次往往能让 DWM 重建呈现表面——用户上一轮反馈
/// 「最小化再还原画面有变化」正是这个机制。
fn repaint(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let size = win.inner_size().ok();
    let _ = win.set_always_on_top(true);
    let _ = win.set_always_on_top(false);
    if let Some(size) = size {
        // ±1 像素的尺寸往返：触发 WM_SIZE → 重算呈现表面，但用户几乎无感。
        let _ = win.set_size(tauri::PhysicalSize::new(size.width + 1, size.height));
        std::thread::sleep(Duration::from_millis(60));
        let _ = win.set_size(tauri::PhysicalSize::new(size.width, size.height));
    }
    let _ = win.emit("miyo://render-stalled", ());
    tracing::info!("render guard: forced repaint");
}

/// 二级：用软件合成参数**重建**主窗并重载页面。
///
/// 为什么必须重建窗口：`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 只在创建
/// WebView2 实例时读取，`location.reload()` 复用同一实例 → 参数不生效（实测设计缺陷）。
/// 同时不同浏览器参数的 WebView2 不能共用同一个 `data_directory`，
/// 因此这里换一个独立目录，避免新建实例直接失败。
///
/// 代价：丢失当前滚动位置与在途表单。此时画面已经冻结 15 秒，
/// 用户本来就什么都点不到，重建是唯一可能恢复的路。
fn reload_with_software_compositing(app: &AppHandle) {
    let Some(old) = app.get_webview_window("main") else {
        return;
    };
    if !SOFTWARE_RETRY_DONE.swap(true, Ordering::SeqCst) {
        tracing::warn!("render guard: rebuilding window with software compositing");
    } else {
        // 已经重建过一次仍然冻结：不再循环重建（会无限闪窗），直接进三级。
        tracing::error!("render guard: software rebuild did not help");
        report_and_notify(app, "软件合成重建后画面仍未刷新");
        return;
    }

    let Some(config) = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .cloned()
    else {
        return;
    };

    // 旧窗口的尺寸/位置要带过去，否则用户会觉得应用「跳回默认大小」。
    let size = old.inner_size().ok();
    let pos = old.outer_position().ok();
    let was_fullscreen = old.is_fullscreen().unwrap_or(false);

    // 先关旧窗：同一 label 不能并存两个窗口。
    if let Err(err) = old.close() {
        tracing::warn!(error = %err, "render guard: closing frozen window failed");
    }

    let result = (|| -> tauri::Result<()> {
        let mut builder = tauri::WebviewWindowBuilder::from_config(app, &config)?;
        builder = builder
            .additional_browser_args(
                "--disable-features=msWebOOUI,msPdfOOUI --disable-gpu --disable-gpu-compositing",
            )
            .shadow(false);
        if let Ok(local_dir) = app.path().app_local_data_dir() {
            builder = builder.data_directory(local_dir.join("webview2-software-recovery"));
        }
        let win = builder.build()?;
        if let Some(size) = size {
            let _ = win.set_size(size);
        }
        if let Some(pos) = pos {
            let _ = win.set_position(pos);
        }
        if was_fullscreen {
            let _ = win.set_fullscreen(true);
        }
        let _ = win.show();
        let _ = win.set_focus();
        Ok(())
    })();

    match result {
        Ok(()) => {
            // 新会话要重新给足宽限：重置心跳纪元并清掉无响应计时。
            crate::FRONTEND_READY.store(false, Ordering::SeqCst);
            *UNRESPONSIVE_SINCE.lock().unwrap() = None;
            tracing::info!("render guard: window rebuilt with software compositing");
        }
        Err(err) => {
            tracing::error!(error = %err, "render guard: software rebuild failed");
            report_and_notify(app, &format!("软件合成重建窗口失败：{err}"));
        }
    }
}

/// 三级：落盘诊断 + 原生提示框。
///
/// 此时 UI 已经确认点不动，提示里必须给**不依赖窗口**的出口：
/// 日志文件路径、`--miyo-export-db` 命令行、以及 Server 版「桌面体验」开关。
fn report_and_notify(app: &AppHandle, reason: &str) {
    let log_dir = miyo_storage::db::log_dir();
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let webview = tauri::webview_version().unwrap_or_else(|e| format!("unavailable: {e}"));
    let heartbeat = miyo_tauri::commands::render_heartbeat_count();

    let body = format!(
        "Miyo 渲染冻结诊断\n\
         ==================\n\
         时间: {time}\n\
         原因: {reason}\n\
         平台: {os}/{arch}\n\
         WebView2: {webview}\n\
         心跳计数: {heartbeat}\n\
         降级阶段: {stage:?}\n\
         日志目录: {log_dir}\n\n\
         可尝试:\n\
         1. Windows Server 开启「桌面体验」(DesktopExperience) 功能后重启：\n\
            dism /online /enable-feature /featurename:ServerMediaFoundation /all /norestart\n\
            dism /online /enable-feature /featurename:DesktopExperience /all /norestart\n\
         2. 启动前设置环境变量 MIYO_WEBVIEW_SOFTWARE=1（强制软件渲染）\n\
         3. 窗口点不动时导出数据（命令行）：\n\
            Miyo.exe --miyo-export-db=<导出路径.db>\n\
         4. 把整个日志目录打包反馈：{log_dir}\n",
        time = chrono::Local::now().to_rfc3339(),
        stage = stage(),
        log_dir = log_dir.display(),
    );

    let wrote = if REPORTED.swap(true, Ordering::SeqCst) {
        None
    } else {
        miyo_tauri::commands::write_blank_screen_report(&body)
    };
    if let Some(path) = &wrote {
        tracing::error!(path = %path.display(), "blank screen report written");
    }

    let report_hint = match &wrote {
        Some(path) => format!("诊断文件已写入：\n{}\n", path.display()),
        None => String::new(),
    };
    let msg = format!(
        "界面无法正常刷新（窗口可能一片空白/黑）。程序仍可运行，但画面可能不更新。\n\n\
         原因：{reason}\n\n\
         {report_hint}日志目录：\n{}\n\n\
         请依次尝试：\n\
         1) Windows Server 开启「桌面体验」功能后重启服务器（虚拟机 + Server 版最常见的原因）；\n\
         2) 设置环境变量 MIYO_WEBVIEW_SOFTWARE=1 后重新打开 Miyo；\n\
         3) 若需先导出数据（窗口点不动也能用）：\n\
            Miyo.exe --miyo-export-db=导出路径.db\n\n\
         把日志目录整体打包发给开发者，可以精确定位。",
        log_dir.display()
    );
    tracing::error!(reason, "render guard: notifying user about a frozen window");
    let _ = rfd::MessageDialog::new()
        .set_level(rfd::MessageLevel::Error)
        .set_title("Miyo 界面无法刷新")
        .set_description(&msg)
        .set_buttons(rfd::MessageButtons::Ok)
        .show();
    let _ = app.emit("miyo://render-stalled", ());
}
