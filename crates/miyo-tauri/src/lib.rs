//! # miyo-tauri：miyo-core / miyo-storage 的 Tauri 2 插件封装
//!
//! 将数据库访问与请求执行封装为 Tauri Command，供 Vue 3 前端通过
//! `@tauri-apps/api/core` 的 `invoke()` 调用。
//!
//! ## 接入方式（Tauri 应用）
//!
//! ```rust,ignore
//! fn main() {
//!     tauri::Builder::default()
//!         .plugin(miyo_tauri::plugin::init())
//!         .run(tauri::generate_context!())
//!         .expect("error while running tauri application");
//! }
//! ```
//!
//! 插件初始化流程：
//! 1. 打开 / 创建 `{data_dir}/Miyo/miyo.db` 并执行迁移（`miyo_storage::db::init_db`）；
//! 2. `app.manage(AppState)` 托管全局状态（连接池 + `tokio::sync::RwLock` 激活上下文）；
//! 3. 注册全部 Command（见下方 [`commands`]）。
//!
//! ## 错误约定
//!
//! 所有 Command 返回 `Result<T, CommandError>`；失败时前端 `invoke()` reject
//! 一个 `{ code: string, message: string }` 对象（`code` 如 `VALIDATION`/`NOT_FOUND`）。
//!
//! ## TypeScript 类型同步（.d.ts 生成方案）
//!
//! 方案 A（推荐）：`tauri-specta` — 在插件里对 Command 声明做 `collect_commands!`，
//! 构建期导出 `bindings.ts`（命令签名 + 实体类型），前端类型与 Rust 严格一致：
//!
//! ```rust,ignore
//! #[cfg(feature = "specta")]
//! tauri_specta::Builder::<tauri::Wry>::new()
//!     .commands(tauri_specta::collect_commands![
//!         get_projects, save_project, delete_project, set_active_project,
//!         list_endpoints, get_endpoint, save_endpoint, delete_endpoint, duplicate_endpoint,
//!         list_environments, save_environment, set_active_environment,
//!         execute_request,
//!     ])
//!     .export(specta_typescript::Typescript::default(), "bindings.ts")
//!     .expect("failed to export specta bindings");
//! ```
//!
//! 注意：模型类型（`miyo_core::model::*`）需要 `specta::Type` 派生，建议在
//! `miyo-core` 增加可选 `specta` feature 后在模型上 `#[cfg_attr(feature = "specta", derive(specta::Type))]`。
//!
//! 方案 B（零依赖）：手工维护 `frontend/src/types/miyoApi.d.ts`（本仓已提供一份镜像），
//! 并在 `useMiyoApi.ts` 中统一入口，保证单点修改。

pub mod commands;
pub mod error;
pub mod state;

use tauri::Manager;

pub use error::{CommandError, CommandResult};
pub use state::AppState;

/// 初始化日志：stdout + `{log_dir}/miyo.log` 按天滚动。
///
/// `RUST_LOG` 环境变量可覆盖级别（默认 `info`）。guard 有意泄漏——
/// 与进程同生命周期，否则后台写线程退出后日志静默丢失。
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let dir = miyo_storage::db::log_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        // 目录不可用时降级为仅 stdout，不阻断启动
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .try_init();
        return;
    }
    let (writer, guard) =
        tracing_appender::non_blocking(tracing_appender::rolling::daily(&dir, "miyo.log"));
    std::mem::forget(guard);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .try_init();
}

/// 插件命名空间。`init()` 注册状态与全部 Command。
///
/// 非泛型 `Wry` 实现:便于 Command 直接取 `tauri::AppHandle` 推送事件
/// (如 `load_test` 的 `miyo:load-progress` 进度)。
pub mod plugin {
    use super::*;

    /// 注册 Miyo 核心插件。
    pub fn init() -> tauri::plugin::TauriPlugin<tauri::Wry> {
        tauri::plugin::Builder::new("miyo")
            .setup(
                |app: &tauri::AppHandle, _api: tauri::plugin::PluginApi<tauri::Wry, ()>| {
                    // 品牌更名迁移必须最先执行：日志目录 / bootstrap 覆盖文件 /
                    // 数据库路径都依赖默认数据目录，整目录改名后其余逻辑无需感知。
                    miyo_core::paths::migrate_legacy_default_dirs();
                    init_tracing();
                    // 开发构建（tauri dev / debug）：先删库文件，迁移重建后写入种子
                    // 测试数据，保证每次重启都是一致的测试数据集；release 为空操作。
                    miyo_storage::db::reset_dev_database();
                    // 初始化数据库（建目录 + 迁移）。阻塞主线程代价低（本地 SQLite）。
                    let db = match tauri::async_runtime::block_on(miyo_storage::db::init_db(
                        &miyo_storage::db::database_path(),
                    )) {
                        Ok(db) => db,
                        Err(e) => {
                            // 无提示退出对用户表现为「闪退」：先弹原生错误框给出原因与数据目录
                            let msg = format!(
                                "数据库初始化失败，应用无法启动。\n\n原因：{}\n\n数据目录：{}",
                                e.user_message(),
                                miyo_storage::db::data_dir().display()
                            );
                            rfd::MessageDialog::new()
                                .set_level(rfd::MessageLevel::Error)
                                .set_title("Miyo 启动失败")
                                .set_description(&msg)
                                .show();
                            return Err(CommandError::from(e).into());
                        }
                    };
                    // 开发构建：写入种子数据（需在 restore_active 之前，激活项指向种子数据）
                    #[cfg(debug_assertions)]
                    if let Err(e) =
                        tauri::async_runtime::block_on(miyo_storage::seed::seed_dev_data(&db))
                    {
                        tracing::warn!("开发种子数据写入失败（不影响应用使用）：{e}");
                    }
                    // 四项互不依赖（各只读 settings、各自恢复自己的全局态）：
                    // 代理设置（失败静默保持直连）/ 自增序列 `{{$seq:key}}`
                    // （失败静默从 1 起）/ MCP 开关（控制面是否随应用自启）/
                    // TLS 客户端证书与自签放行（失败静默保持默认），
                    // 合并进一次 block_on 并发执行，替代原来多次串行 block_on。
                    // 其后的激活态恢复依赖 db 移交 AppState，必须排在后面。
                    let mcp_enabled = tauri::async_runtime::block_on(async {
                        let ((), (), mcp_enabled, ()) = tokio::join!(
                            commands::settings::apply_saved_proxy(&db),
                            commands::seq::apply_saved_seq_counters(&db),
                            commands::agent::read_mcp_enabled(&db),
                            commands::settings::apply_saved_tls(&db),
                        );
                        mcp_enabled
                    });
                    // 按保留天数清理过期日志（失败仅记日志）
                    commands::log::cleanup_logs_on_startup(&db);
                    // 恢复持久化的激活项目 / 环境（settings 表，含归属校验）
                    let state = AppState::new(db);
                    let _ = tauri::async_runtime::block_on(state.restore_active());
                    app.manage(state);
                    // Agent 控制面随应用自动拉起（幂等；失败仅记日志不阻断启动）；
                    // 用户在设置中关闭 MCP 后跳过自动拉起
                    if mcp_enabled {
                        if let Err(e) =
                            tauri::async_runtime::block_on(commands::agent::ensure_started(app))
                        {
                            tracing::warn!("Agent 控制面启动失败（不影响应用使用）：{e}");
                        }
                    }
                    Ok(())
                },
            )
            .invoke_handler(tauri::generate_handler![
                commands::get_projects,
                commands::update_projects_order,
                commands::list_project_stats,
                commands::save_project,
                commands::delete_project,
                commands::set_active_project,
                commands::get_active_project,
                commands::list_endpoints,
                commands::get_endpoint,
                commands::save_endpoint,
                commands::delete_endpoint,
                commands::duplicate_endpoint,
                commands::list_folders,
                commands::save_folder,
                commands::delete_folder,
                commands::parse_curl_command,
                commands::list_environments,
                commands::save_environment,
                commands::set_active_environment,
                commands::get_active_environment,
                commands::delete_environment,
                commands::export_environment,
                commands::import_environment,
                commands::get_global_variables,
                commands::save_global_variables,
                commands::get_global_params,
                commands::save_global_params,
                commands::execute_request,
                commands::cancel_request,
                commands::get_response_body,
                commands::list_examples,
                commands::save_example,
                commands::delete_example,
                commands::list_request_examples,
                commands::save_request_example,
                commands::delete_request_example,
                commands::oauth_authorize,
                commands::oauth_access_token,
                commands::oauth_fetch_token,
                commands::codegen_render,
                commands::cookie_list,
                commands::cookie_clear,
                commands::clipboard_write_text,
                commands::list_request_histories,
                commands::clear_request_histories,
                commands::mock_start,
                commands::mock_stop,
                commands::mock_status,
                commands::mock_reload,
                commands::agent_start,
                commands::agent_stop,
                commands::agent_status,
                commands::get_mcp_enabled,
                commands::set_mcp_enabled,
                commands::get_mcp_port,
                commands::set_mcp_port,
                commands::backup_export,
                commands::backup_restore,
                commands::sync_remote_list,
                commands::sync_remote_create,
                commands::sync_remote_delete,
                commands::sync_push,
                commands::sync_pull_preview,
                commands::sync_pull_apply,
                commands::sync_remote_list_projects,
                commands::sync_import_project,
                commands::list_teams,
                commands::create_team,
                commands::update_team,
                commands::delete_team,
                commands::move_project_to_team,
                commands::import_document,
                commands::read_text_file,
                commands::export_openapi,
                commands::export_docs,
                commands::export_smoke_docs,
                commands::save_text_file,
                commands::test_endpoint,
                commands::load_test,
                commands::log_files,
                commands::log_tail,
                commands::log_dir_path,
                commands::get_log_retention_days,
                commands::set_log_retention_days,
                commands::cancel_load_test,
                commands::test_collection,
                commands::cancel_test_collection,
                commands::grpc_list_services,
                commands::grpc_invoke,
                commands::grpc_stream_close,
                commands::grpc_list_proto_files,
                commands::grpc_save_proto_files,
                commands::grpc_delete_proto_file,
                commands::ws_connect,
                commands::ws_send,
                commands::ws_disconnect,
                commands::sse_connect,
                commands::sse_disconnect,
                commands::list_mock_rules,
                commands::save_mock_rule,
                commands::delete_mock_rule,
                commands::get_http_proxy,
                commands::set_http_proxy,
                commands::get_http_timeout_ms,
                commands::set_http_timeout_ms,
                commands::get_tls_settings,
                commands::set_tls_settings,
                commands::get_data_dir,
                commands::get_default_data_dir,
                commands::set_data_dir,
                commands::reset_data_dir,
                commands::list_seq_counters,
                commands::set_seq_counter,
                commands::delete_seq_counter,
                commands::test_http_proxy,
                commands::list_test_cases,
                commands::save_test_case,
                commands::update_test_case_meta,
                commands::update_test_case_content,
                commands::update_test_case_status,
                commands::delete_test_case,
            ])
            .build()
    }
}
