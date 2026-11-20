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
pub mod sessions;
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

/// Java Attach 工具 jar 候选目录（按序取第一个就绪的）：
/// 1. Tauri 资源目录（打包后：resources/java/，tauri.conf.json bundle.resources）；
/// 2. 开发回退：仓库 java/dist/（scripts/build_java_debug.sh 产物）。
fn java_attach_jar_candidates(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    let mut candidates = Vec::new();
    // 0. 组件自动下载落地目录（市场远程获取的 jar 优先于随包资源——
    //    远程版本可能比安装包携带的更新）
    candidates.push(miyo_storage::db::data_dir().join("java"));
    if let Ok(res_dir) = app
        .path()
        .resolve("java", tauri::path::BaseDirectory::Resource)
    {
        candidates.push(res_dir);
    }
    // dev 回退：CARGO_MANIFEST_DIR 在编译期已知（miyo-tauri/src → 仓库根 java/dist）
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        candidates.push(
            std::path::PathBuf::from(manifest_dir)
                .join("..")
                .join("..")
                .join("java")
                .join("dist"),
        );
    }
    candidates
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
                            // 版本降级兜底：库比程序新（用户从新版回退旧版，如 issue #153）
                            // 时把数据目录改指向新的隔离目录，重启即能进；老库原样留存。
                            let db_path = miyo_storage::db::database_path();
                            let switched = tauri::async_runtime::block_on(
                                miyo_storage::db::redirect_newer_data_dir(&db_path),
                            );
                            if let Some(dir) = &switched {
                                // env 覆盖时 data_dir.txt 不生效（env 优先级更高），提示要改成环境变量。
                                let action = if miyo_core::paths::data_dir_from_env().is_some() {
                                    format!(
                                        "当前数据目录由环境变量 MIYO_DATA_DIR 指定，程序不会覆盖它。\n\
                                         请把该环境变量改指向新目录后重新启动：\n{}",
                                        dir.display()
                                    )
                                } else {
                                    format!(
                                        "已切换到一个新的数据目录（旧数据原样保留，安装最新版可继续使用）：\n{}\n\n\
                                         请点击「确定」后重新启动 Miyo。",
                                        dir.display()
                                    )
                                };
                                rfd::MessageDialog::new()
                                    .set_level(rfd::MessageLevel::Warning)
                                    .set_title("Miyo 数据版本过新")
                                    .set_description(format!(
                                        "检测到数据由更新版本的 Miyo 创建，当前版本无法打开。\n\n{action}"
                                    ))
                                    .show();
                                return Err(CommandError::from(e).into());
                            }
                            // 无提示退出对用户表现为「闪退」：先弹原生错误框给出原因与数据目录。
                            // 未经兜底却报版本问题（如来自覆盖目录）：给出可操作指引。
                            let origin = tauri::async_runtime::block_on(
                                miyo_storage::db::newer_schema_note(&db_path),
                            );
                            let reason = match origin {
                                Some(v) => format!(
                                    "数据由更新版本的 Miyo 创建（库版本 {v}），当前版本无法识别。\n\
                                     请在设置中改用其他数据目录，或安装最新版 Miyo。"
                                ),
                                None => e.user_message(),
                            };
                            let msg = format!(
                                "数据库初始化失败，应用无法启动。\n\n原因：{reason}\n\n数据目录：{}",
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
                        let ((), (), (), mcp_enabled, ()) = tokio::join!(
                            commands::settings::apply_saved_proxy(&db),
                            commands::settings::apply_saved_sensitive_patterns(&db),
                            commands::seq::apply_saved_seq_counters(&db),
                            commands::agent::read_mcp_enabled(&db),
                            commands::settings::apply_saved_tls(&db),
                        );
                        mcp_enabled
                    });
                    // 按保留天数清理过期日志（失败仅记日志）
                    commands::log::cleanup_logs_on_startup(&db);
                    // 恢复持久化的激活项目 / 环境（settings 表，含归属校验）
                    let state = AppState::new(db, java_attach_jar_candidates(app));
                    // Java Attach 会话心跳常驻（失联会话移除 + infoFile 过期
                    // 清扫；首个 tick 立即触发，兼作启动清扫）。
                    // setup 回调不在 tokio 上下文内——必须经 tauri 的运行时驱动。
                    // 句柄注册进 TaskManager（原句柄丢弃——退出时随 runtime 结束，
                    // 现在有明确的取消与可见性）。
                    tauri::async_runtime::block_on(async {
                        let heartbeat = state
                            .java_attach
                            .spawn_heartbeat(std::time::Duration::from_secs(30));
                        let _ = state.tasks.start(
                            "internal:java-attach-heartbeat".into(),
                            miyo_tasks::TaskKind::JavaAttach,
                            None,
                        );
                        let _ = heartbeat; // 常驻循环：退出时随 runtime 结束
                    });
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
            commands::get_ai_settings,
            commands::set_ai_settings,
            commands::ai_test_connection,
            commands::ai_list_sessions,
            commands::ai_get_session,
            commands::ai_delete_session,
            commands::ai_chat_send,
            commands::ai_chat_approve,
            commands::ai_chat_cancel,
            commands::ai_analyze_dependencies,
            commands::ai_analyze_coverage,
            commands::ai_generate_test_case_drafts,
            commands::ai_generate_scenario_draft,
            commands::ai_propose_fix,
            commands::ai_save_scenario_draft,
            commands::ai_list_scenario_drafts,
            commands::ai_apply_scenario_draft,
            commands::ai_revert_scenario_draft,
                commands::get_projects,
                commands::update_projects_order,
                commands::list_project_stats,
                commands::list_project_quality,
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
                commands::delete_request_history,
                commands::clear_request_histories,
                commands::list_recent_activities,
                commands::mock_start,
                commands::mock_stop,
                commands::mock_status,
                commands::mock_request_logs,
                commands::mock_clear_logs,
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
                // 插件系统（commands/plugins.rs）
                commands::plugin_list_installed,
                commands::plugin_list_catalog,
                commands::plugin_get,
                commands::plugin_inspect_package,
                commands::plugin_install_from_path,
                commands::plugin_install_builtin,
                commands::plugin_uninstall,
                commands::plugin_set_enabled,
                commands::plugin_update_builtin,
                commands::plugin_set_permissions,
                commands::plugin_report_error,
                commands::plugin_verify_integrity,
                commands::plugin_logs,
                commands::plugin_clear_logs,
                commands::plugin_record_log,
                commands::plugin_data_get,
                commands::plugin_data_set,
                commands::plugin_data_delete,
                commands::plugin_data_keys,
                commands::plugin_session_create,
                commands::plugin_read_asset,
                commands::plugin_sidecar_start,
                commands::plugin_sidecar_stop,
                commands::plugin_sidecar_status,
                commands::plugin_sidecar_request,
                commands::plugin_update_from_path,
                commands::plugin_rollback,
                commands::plugin_market_index,
                commands::plugin_versions,
                commands::plugin_inspect_from_url,
                commands::plugin_install_from_cached,
                // Java 方法实时调试（commands/java_debug.rs，插件门禁见该文件）
                // Java Debug 2.0 Attach（commands/java_attach.rs）
                commands::java_discover_processes,
                commands::java_probe_pid,
                commands::java_attach_process,
                commands::java_detach_process,
                commands::java_unload_agent,
                commands::java_attach_sessions,
                commands::java_agent_status,
                commands::java_agent_spring_status,
                commands::java_agent_classes,
                commands::java_agent_class_detail,
                commands::java_agent_methods,
                commands::java_agent_beans,
                commands::java_agent_invoke,
                commands::java_agent_cancel,
                commands::java_component_status,
                commands::java_component_download,
                commands::list_java_projects,
                commands::save_java_project,
                commands::delete_java_project,
                commands::java_discover_local,
                commands::java_probe_endpoint,
                commands::java_test_connection,
                commands::java_list_classes,
                commands::java_list_methods,
                commands::java_invoke,
                commands::java_cancel_invocation,
                commands::list_java_history,
                commands::get_java_invocation,
                commands::delete_java_invocation,
                commands::clear_java_history,
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
                commands::get_sensitive_patterns,
                commands::set_sensitive_patterns,
                commands::get_mcp_permissions,
                commands::set_mcp_permissions,
                commands::set_tls_settings,
                commands::get_mcp_tools,
                commands::set_mcp_tool_enabled,
                commands::list_scenarios,
                commands::save_scenario,
                commands::delete_scenario,
                commands::run_scenario,
                commands::run_scenario_matrix,
                commands::export_scenario_report,
                commands::cancel_scenario_run,
                commands::list_scenario_runs,
                commands::get_scenario_run,
                commands::list_recent_scenario_runs,
                commands::project_scenario_run_stats,
                commands::list_scenario_summaries,
                commands::delete_scenario_run,
                commands::clear_scenario_runs,
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
                commands::set_test_case_folder,
                commands::rename_test_case_folder,
                commands::update_test_case_content,
                commands::update_test_case_status,
                commands::delete_test_case,
                commands::get_test_case,
                commands::reorder_test_cases,
                commands::list_test_case_runs,
                commands::get_test_case_run,
                commands::delete_test_case_run,
                commands::cancel_test_endpoint,
            ])
            .build()
    }
}
