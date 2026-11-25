//! 核心裁剪构建（java feature 关闭）的 java_* 命令占位。
//!
//! 全部 java_* 命令在无 java 构建中仍注册（lib.rs 命令清单零改动），一律
//! 返回 `FEATURE_UNAVAILABLE`：前端拿到结构化错误码而不是「未知命令」，
//! 诊断 UI 与门禁测试路径保持可用。真实实现见 cfg(feature = "java") 的
//! java_attach / java_debug / java_components 三模块。
#![cfg(not(feature = "java"))]

use crate::error::{CommandError, CommandErrorCode, CommandResult};

macro_rules! java_stub_commands {
    ($($name:ident),* $(,)?) => {
        $(
            #[tauri::command(rename_all = "camelCase")]
            pub async fn $name() -> CommandResult<serde_json::Value> {
                Err(CommandError::with_code(
                    CommandErrorCode::FeatureUnavailable,
                    "此构建未包含 Java 调试能力（构建未启用 java feature）",
                ))
            }
        )*
    };
}

java_stub_commands!(
    java_discover_processes,
    java_probe_pid,
    java_attach_process,
    java_detach_process,
    java_unload_agent,
    java_attach_sessions,
    java_agent_status,
    java_agent_spring_status,
    java_agent_classes,
    java_agent_classes_stats,
    java_agent_classes_reindex,
    java_agent_class_detail,
    java_agent_methods,
    java_agent_beans,
    java_agent_invoke,
    java_agent_cancel,
    java_component_status,
    java_component_download,
    list_java_projects,
    save_java_project,
    delete_java_project,
    java_discover_local,
    java_probe_endpoint,
    java_test_connection,
    java_list_classes,
    java_list_methods,
    java_invoke,
    java_cancel_invocation,
    list_java_history,
    get_java_invocation,
    delete_java_invocation,
    clear_java_history,
);
