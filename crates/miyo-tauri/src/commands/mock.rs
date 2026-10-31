//! Mock 服务 Command:启动(规则+接口示例生成定义)/ 停止 / 查询状态。
//! 端口从 4010 起自动探测;启动逻辑与 Dioxus 版对齐:启用规则优先,接口兜底。

use tauri::State;
use uuid::Uuid;

use miyo_core::model::{EndpointStatus, MockRequestLog};
use miyo_mock::server::MockServer;
use miyo_storage::repository as repo;
use std::collections::HashMap;

use crate::error::{CommandError, CommandResult};
use crate::state::AppState;

/// 启动 Mock 服务（重复启动返回 409 语义错误码 MOCK_RUNNING）。
///
/// 并发安全：先做快速读检查，DB 查询 + 端口 bind 全程不持写锁；
/// bind 成功后写锁 CAS，竞输者关闭刚启动的服务再报错，避免句柄泄漏。
#[tauri::command(rename_all = "camelCase")]
pub async fn mock_start(state: State<'_, AppState>) -> CommandResult<String> {
    if state.mock.read().await.is_some() {
        return Err(CommandError::with_code(
            "MOCK_RUNNING",
            "Mock 服务已在运行，请先停止",
        ));
    }

    let project = state
        .active_project()
        .await?
        .ok_or_else(|| CommandError::validation("未选择项目，无法启动 Mock 服务"))?;

    // 1. 构建定义（规则 > 示例 > 默认），批量一次查询（去 N+1）。
    let defs = build_definitions(&state.db, project.id).await?;

    // 2. 启动并 CAS 托管句柄（临提交前才加写锁）。
    let store = miyo_mock::server::MockStore::new();
    store.set_definitions(defs);
    let server = miyo_mock::server::start(store).await?;
    let address = server.address();
    let mut guard = state.mock.write().await;
    if guard.is_some() {
        server.stop().await;
        return Err(CommandError::with_code(
            "MOCK_RUNNING",
            "Mock 服务已在运行，请先停止",
        ));
    }
    *guard = Some(server);
    Ok(address)
}

/// 从数据库构建 Mock 定义（规则 > 接口示例 > 默认；与 Dioxus 版一致）。
async fn build_definitions(
    db: &sqlx::SqlitePool,
    project_id: Uuid,
) -> CommandResult<Vec<miyo_mock::server::MockDefinition>> {
    let rules = repo::list_mock_rules(db, project_id).await?;
    let endpoints = repo::list_endpoints(db, project_id).await?;
    let active_ids: Vec<Uuid> = endpoints
        .iter()
        .filter(|e| e.status != EndpointStatus::Deprecated)
        .map(|e| e.id)
        .collect();
    let examples_by_ep: HashMap<Uuid, Vec<miyo_core::model::ResponseExample>> =
        repo::list_response_examples_by_endpoints(db, &active_ids)
            .await
            .unwrap_or_default();

    let mut defs: Vec<miyo_mock::server::MockDefinition> = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        defs.push(miyo_mock::server::MockDefinition::from_rule(rule));
    }
    for ep in endpoints
        .iter()
        .filter(|e| e.status != EndpointStatus::Deprecated)
    {
        let example = examples_by_ep.get(&ep.id).and_then(|l| l.first());
        defs.push(miyo_mock::server::MockDefinition::from_endpoint(
            ep.method.as_str(),
            &ep.path,
            example,
        ));
    }
    Ok(defs)
}

/// 热重载 Mock 定义：运行中直接替换路由与模板，无需重启服务。
///
/// 原来修改接口/规则后必须"关闭再启动"Mock 才生效；现在运行中调用即
/// 原子替换（读锁内整体 swap，在途请求不受影响），返回定义条数。
#[tauri::command(rename_all = "camelCase")]
pub async fn mock_reload(state: State<'_, AppState>) -> CommandResult<u64> {
    let project = state
        .active_project()
        .await?
        .ok_or_else(|| CommandError::validation("未选择项目，无法重载 Mock 定义"))?;
    let defs = build_definitions(&state.db, project.id).await?;
    let count = defs.len() as u64;
    let guard = state.mock.read().await;
    let Some(server) = guard.as_ref() else {
        return Err(CommandError::with_code(
            "MOCK_NOT_RUNNING",
            "Mock 服务未运行，请先启动",
        ));
    };
    server.store().set_definitions(defs);
    Ok(count)
}

/// 停止 Mock 服务（幂等）。
#[tauri::command(rename_all = "camelCase")]
pub async fn mock_stop(state: State<'_, AppState>) -> CommandResult<()> {
    let mut guard = state.mock.write().await;
    if let Some(server) = guard.take() {
        server.stop().await;
    }
    Ok(())
}

/// Mock 服务状态：未运行返回 `None`，运行中返回监听地址。
#[tauri::command(rename_all = "camelCase")]
pub async fn mock_status(state: State<'_, AppState>) -> CommandResult<Option<String>> {
    let running = state.mock.read().await;
    Ok(running.as_ref().map(MockServer::address))
}

/// 最近 Mock 请求日志（新→旧，环形缓冲最近 100 条；服务未运行返回空）。
/// 未匹配请求 `matched = false`——Mock 调试「为什么没命中」的诊断入口。
#[tauri::command(rename_all = "camelCase")]
pub async fn mock_request_logs(state: State<'_, AppState>) -> CommandResult<Vec<MockRequestLog>> {
    let guard = state.mock.read().await;
    Ok(guard
        .as_ref()
        .map(|server| server.store().recent_logs())
        .unwrap_or_default())
}

/// 清空 Mock 请求日志（服务未运行为空操作）。
#[tauri::command(rename_all = "camelCase")]
pub async fn mock_clear_logs(state: State<'_, AppState>) -> CommandResult<()> {
    if let Some(server) = state.mock.read().await.as_ref() {
        server.store().clear_logs();
    }
    Ok(())
}
