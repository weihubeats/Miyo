//! 环境 Command：列表 / 保存 / 激活切换 / 导入导出。
//!
//! 环境为项目维度：哪个项目就编辑哪个项目的环境（`list_environments`
//! 按项目列出，`save_environment` 校验归属当前激活项目）。
//!
//! 导入导出对标 Postman/Bruno：原来环境只能随整项目备份 JSON 迁移，
//! 无单环境文件互通。本模块提供 Miyo 原生 JSON 与 Postman Environment
//! v1 双格式（自动识别导入）。注意：导出含变量明文（与备份 JSON 一致），
//! 请妥善保管导出文件。

use serde::{Deserialize, Serialize};
use tauri::State;
use uuid::Uuid;

use miyo_core::model::{Environment, EnvironmentVariable};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 列出项目的全部环境（项目维度）。
#[tauri::command(rename_all = "camelCase")]
pub async fn list_environments(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<Environment>> {
    repo::list_environments(&state.db, project_id)
        .await
        .map_err(Into::into)
}

/// 保存环境（upsert）。名称必填；所属项目必须存在。
/// 不要求等于激活项目：设置里可跨项目管理环境（首页无工作区上下文）；
/// 激活环境的切换另有归属校验（`set_active_environment`）。
/// 返回完整环境，激活环境被编辑时同步覆盖缓存。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_environment(
    state: State<'_, AppState>,
    environment: Environment,
) -> CommandResult<Environment> {
    if environment.name.trim().is_empty() {
        return Err(CommandError::validation("环境名称不能为空"));
    }
    repo::get_project(&state.db, environment.project_id)
        .await
        .map_err(|_| CommandError::validation("环境所属项目不存在"))?;
    let saved = repo::save_environment(&state.db, &environment).await?;
    // 激活环境被编辑：同步覆盖缓存，否则发请求仍用旧环境变量
    state.refresh_active_environment(saved.clone()).await;
    Ok(saved)
}

/// 切换激活环境（`null` 表示不使用环境变量）。返回切换后的环境缓存。
/// 环境必须属于当前激活项目（后端校验）。
#[tauri::command(rename_all = "camelCase")]
pub async fn set_active_environment(
    state: State<'_, AppState>,
    environment_id: Option<Uuid>,
) -> CommandResult<Option<Environment>> {
    state.set_active_environment(environment_id).await?;
    state.active_environment().await
}

/// 读取当前激活环境（当前激活项目记忆的激活环境）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_active_environment(
    state: State<'_, AppState>,
) -> CommandResult<Option<Environment>> {
    state.active_environment().await
}

/// 删除环境；若删除的是当前激活环境，则同时清空激活状态（含持久化键）。
#[tauri::command(rename_all = "camelCase")]
pub async fn delete_environment(
    state: State<'_, AppState>,
    environment_id: Uuid,
) -> CommandResult<()> {
    repo::delete_environment(&state.db, environment_id).await?;
    let (project_id, is_active) = {
        let active = state.active.read().await;
        (
            active.project_id,
            active.environment_id == Some(environment_id),
        )
    };
    if is_active {
        let mut active = state.active.write().await;
        active.environment_id = None;
        active.environment = None;
        drop(active);
        if let Some(pid) = project_id {
            repo::set_setting(
                &state.db,
                &crate::state::active_environment_key(pid),
                "null",
            )
            .await?;
        }
    }
    Ok(())
}

/// 环境交换格式（导出目标 / 导入自动识别）。
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvExchangeFormat {
    /// Miyo 原生 JSON（变量 + Base URL 全量）。
    /// `alias` 兼容品牌更名前导出文件里的 `rustfox_json` 值。
    #[serde(alias = "rustfox_json")]
    MiyoJson,
    /// Postman Environment v1（`{name, values:[{key,value,enabled}]}`；Base URL 丢弃）。
    PostmanJson,
}

/// 环境导出结果：内容 + 建议文件名（前端经目录选择框落盘）。
#[derive(Debug, Clone, Serialize)]
pub struct ExportedEnv {
    pub content: String,
    pub suggested_name: String,
}

/// 导出单个环境（变量以明文落盘，与备份 JSON 口径一致）。
#[tauri::command(rename_all = "camelCase")]
pub async fn export_environment(
    state: State<'_, AppState>,
    environment_id: Uuid,
    format: EnvExchangeFormat,
) -> CommandResult<ExportedEnv> {
    let env = repo::get_environment(&state.db, environment_id).await?;
    let safe_name: String = env
        .name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_whitespace() => '-',
            c => c,
        })
        .collect();
    let safe_name = if safe_name.trim_matches('-').is_empty() {
        "environment".to_string()
    } else {
        safe_name
    };
    let date = chrono::Local::now().format("%Y-%m-%d");
    match format {
        EnvExchangeFormat::MiyoJson => {
            let content = serde_json::to_string_pretty(&env)
                .map_err(|e| CommandError::with_code("EXPORT", e.to_string()))?;
            Ok(ExportedEnv {
                content,
                suggested_name: format!("miyo-env-{safe_name}-{date}.json"),
            })
        }
        EnvExchangeFormat::PostmanJson => {
            let values: Vec<serde_json::Value> = env
                .variables
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "key": v.key,
                        "value": v.effective_value(),
                        "enabled": v.enabled,
                        "type": "default",
                    })
                })
                .collect();
            let doc = serde_json::json!({
                "_postman_variable_scope": "environment",
                "name": env.name,
                "values": values,
            });
            let content = serde_json::to_string_pretty(&doc)
                .map_err(|e| CommandError::with_code("EXPORT", e.to_string()))?;
            Ok(ExportedEnv {
                content,
                suggested_name: format!("postman-env-{safe_name}-{date}.json"),
            })
        }
    }
}

/// 导入预览（不落库）：自动识别 Miyo / Postman 格式，返回可直接保存的环境草稿。
/// id 全部重新生成，名称冲突时由前端在保存前改名（此处仅后缀提示）。
#[derive(Debug, Clone, Serialize)]
pub struct ImportedEnv {
    pub format: &'static str,
    pub name: String,
    pub variables: Vec<EnvironmentVariable>,
    /// 环境 Base URL（Postman 格式无此信息，导入后为空）。
    pub base_url: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn import_environment(text: String, project_id: Uuid) -> CommandResult<ImportedEnv> {
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| CommandError::validation(format!("不是合法 JSON：{e}")))?;
    // Miyo 原生：含 variables 数组（兼容旧版多模块导出：modules → 首个基址）。
    if value.get("variables").is_some_and(|v| v.is_array()) {
        let mut env: Environment = serde_json::from_value(normalize_legacy_env(value))
            .map_err(|e| CommandError::validation(format!("Miyo 环境格式解析失败：{e}")))?;
        if env.name.trim().is_empty() {
            return Err(CommandError::validation("环境名称为空"));
        }
        env.id = Uuid::new_v4();
        env.project_id = project_id;
        let now = chrono::Utc::now();
        env.created_at = now;
        env.updated_at = now;
        return Ok(ImportedEnv {
            format: "miyo",
            name: env.name,
            variables: env.variables,
            base_url: env.base_url,
        });
    }
    // Postman：values 数组。
    if let Some(values) = value.get("values").and_then(|v| v.as_array()) {
        let name = value
            .get("name")
            .and_then(|n| n.as_str())
            .filter(|n| !n.trim().is_empty())
            .ok_or_else(|| CommandError::validation("Postman 环境缺少 name"))?
            .to_string();
        let mut variables = Vec::with_capacity(values.len());
        for v in values {
            let Some(key) = v.get("key").and_then(|k| k.as_str()) else {
                continue;
            };
            if key.trim().is_empty() {
                continue;
            }
            variables.push(EnvironmentVariable {
                key: key.to_string(),
                remote_value: v
                    .get("value")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string(),
                local_value: String::new(),
                enabled: v.get("enabled").and_then(|x| x.as_bool()).unwrap_or(true),
                description: None,
            });
        }
        return Ok(ImportedEnv {
            format: "postman",
            name,
            variables,
            base_url: String::new(),
        });
    }
    Err(CommandError::validation(
        "无法识别的环境格式（支持 Miyo 环境 JSON / Postman Environment）",
    ))
}

/// 旧版（Miyo 多模块）导出兼容：`modules` 数组取默认 / 首个模块的
/// base_url 提升为环境 Base URL，并补 `project_id` 占位（由调用方覆盖）。
fn normalize_legacy_env(mut value: serde_json::Value) -> serde_json::Value {
    let Some(object) = value.as_object_mut() else {
        return value;
    };
    if object.contains_key("base_url") && object.contains_key("project_id") {
        return value;
    }
    let base_url = object
        .get("modules")
        .and_then(|m| m.as_array())
        .and_then(|ms| {
            ms.iter()
                .find(|m| {
                    m.get("is_default")
                        .and_then(|d| d.as_bool())
                        .unwrap_or(false)
                })
                .or_else(|| ms.first())
        })
        .and_then(|m| m.get("base_url"))
        .and_then(|b| b.as_str())
        .unwrap_or("")
        .to_string();
    object.remove("modules");
    object.insert("base_url".into(), serde_json::json!(base_url));
    object.insert(
        "project_id".into(),
        serde_json::json!(Uuid::nil().to_string()),
    );
    value
}

/// 读取全局变量（跨项目共享，优先级最低的兜底变量表）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_global_variables(
    state: State<'_, AppState>,
) -> CommandResult<Vec<miyo_core::model::EnvironmentVariable>> {
    repo::get_global_variables(&state.db)
        .await
        .map_err(Into::into)
}

/// 保存全局变量（整体覆盖写）。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_global_variables(
    state: State<'_, AppState>,
    variables: Vec<miyo_core::model::EnvironmentVariable>,
) -> CommandResult<()> {
    repo::save_global_variables(&state.db, &variables).await?;
    state.invalidate_global_settings();
    Ok(())
}

/// 读取全局参数（每个请求自动注入的 query / header）。
#[tauri::command(rename_all = "camelCase")]
pub async fn get_global_params(
    state: State<'_, AppState>,
) -> CommandResult<Vec<miyo_core::model::GlobalParam>> {
    repo::get_global_params(&state.db).await.map_err(Into::into)
}

/// 保存全局参数（整体覆盖写）。
#[tauri::command(rename_all = "camelCase")]
pub async fn save_global_params(
    state: State<'_, AppState>,
    params: Vec<miyo_core::model::GlobalParam>,
) -> CommandResult<()> {
    repo::save_global_params(&state.db, &params).await?;
    state.invalidate_global_settings();
    Ok(())
}
