//! AI 测试中心 Command 层。
//!
//! 分四块：
//! 1. **AI 模型设置**（OpenAI 兼容；API Key 经 miyo-secret 加密落库）；
//! 2. **聊天会话**（持久化 + agent 循环 + 副作用确认卡：registry 元数据
//!    `side_effects=true` 的工具在执行前暂停，等用户在聊天内批准）；
//! 3. **直连分析**（不走 LLM：依赖图 / 覆盖率 / 用例草稿 / 场景草稿 / 修复草稿，
//!    供左侧面板与产物预览）；
//! 4. **场景草稿历史**（应用 / 撤销；base_json 回写即撤销）。
//!
//! 循环的工具执行走 miyo-agent 控制面（`ensure_started` + `ControlClient`），
//! 与外部 MCP Agent 共享统一 Registry / 五级权限 / 脱敏体系。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use miyo_agent::registry::{self, tool_meta};
use miyo_storage::repository as repo;

use crate::error::{CommandError, CommandErrorCode, CommandResult};
use crate::state::AppState;

/// 聊天事件通道（前端 `listen('miyo:ai-chat')`）。
const CHAT_EVENT: &str = "miyo:ai-chat";

// ---------- 1. AI 模型设置 ----------

/// AI 设置回显（apiKey 掩码：只回前 4 位 + 长度，明文不出仓）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsView {
    pub base_url: String,
    pub api_key_masked: String,
    pub api_key_set: bool,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_ai_settings(state: State<'_, AppState>) -> CommandResult<AiSettingsView> {
    let cfg = repo::get_ai_settings(&state.db).await?;
    Ok(match cfg {
        None => AiSettingsView {
            base_url: String::new(),
            api_key_masked: String::new(),
            api_key_set: false,
            model: String::new(),
            temperature: None,
            max_tokens: None,
        },
        Some(c) => {
            let masked = if c.api_key.len() > 8 {
                format!("{}…（{} 位）", &c.api_key[..4], c.api_key.len())
            } else if !c.api_key.is_empty() {
                "••••".to_string()
            } else {
                String::new()
            };
            AiSettingsView {
                base_url: c.base_url,
                api_key_masked: masked,
                api_key_set: !c.api_key.is_empty(),
                model: c.model,
                temperature: c.temperature,
                max_tokens: c.max_tokens,
            }
        }
    })
}

/// 保存 AI 设置请求体。
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetAiSettingsArgs {
    pub base_url: String,
    /// None = 保持原 Key 不变（前端掩码回显时不回传明文）。
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn set_ai_settings(
    state: State<'_, AppState>,
    args: SetAiSettingsArgs,
) -> CommandResult<()> {
    let base_url = args.base_url.trim().to_string();
    if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
        return Err(CommandError::validation(
            "Base URL 须以 http:// 或 https:// 开头",
        ));
    }
    if args.model.trim().is_empty() {
        return Err(CommandError::validation("模型名不能为空"));
    }
    let api_key = match args.api_key {
        Some(k) => k.trim().to_string(),
        None => repo::get_ai_settings(&state.db)
            .await?
            .map(|c| c.api_key)
            .unwrap_or_default(),
    };
    repo::save_ai_settings(
        &state.db,
        &repo::AiProviderSettings {
            base_url,
            api_key,
            model: args.model.trim().to_string(),
            temperature: args.temperature,
            max_tokens: args.max_tokens,
        },
    )
    .await?;
    Ok(())
}

/// 连通性验活（最小 completions 请求）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_test_connection(state: State<'_, AppState>) -> CommandResult<()> {
    let cfg = require_ai_config(&state.db).await?;
    miyo_ai::LlmClient::new(cfg)
        .map_err(CommandError::from)?
        .test_connection()
        .await
        .map_err(CommandError::from)
}

async fn require_ai_config(db: &sqlx::SqlitePool) -> CommandResult<miyo_ai::AiProviderConfig> {
    let cfg = repo::get_ai_settings(db).await?.ok_or_else(|| {
        CommandError::with_code(
            CommandErrorCode::AiNotConfigured,
            "尚未配置 AI 模型：请到 设置 → AI 模型 填写 Base URL / API Key / 模型名",
        )
    })?;
    to_ai_config(cfg)
}

fn to_ai_config(cfg: repo::AiProviderSettings) -> CommandResult<miyo_ai::AiProviderConfig> {
    let c = miyo_ai::AiProviderConfig {
        base_url: cfg.base_url,
        api_key: cfg.api_key,
        model: cfg.model,
        temperature: cfg.temperature,
        max_tokens: cfg.max_tokens,
    };
    if !c.is_complete() {
        return Err(CommandError::with_code(
            CommandErrorCode::AiNotConfigured,
            "AI 配置不完整：Base URL 与模型名必填",
        ));
    }
    Ok(c)
}

impl From<miyo_ai::AiError> for CommandError {
    fn from(e: miyo_ai::AiError) -> Self {
        CommandError::with_code(CommandErrorCode::AiProvider, e.to_string())
    }
}

// ---------- 2. 聊天会话 ----------

/// 会话运行注册表：取消令牌 + 待确认审批（循环任务与 approve/cancel 命令共享）。
#[derive(Default)]
pub struct AiChatRegistry {
    pub cancels: HashMap<String, CancellationToken>,
    /// key = `{sessionId}:{callId}`。
    pub approvals: HashMap<String, tokio::sync::oneshot::Sender<bool>>,
}

/// 会话摘要（列表项）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSessionView {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 消息视图（JSON 透传形状，role: system 不持久化）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiMessageView {
    pub id: String,
    pub seq: i64,
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Option<serde_json::Value>,
    pub tool_call_id: Option<String>,
    pub artifact: Option<serde_json::Value>,
    pub created_at: String,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn ai_list_sessions(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<Vec<AiSessionView>> {
    let sessions = repo::list_ai_sessions(&state.db, project_id).await?;
    Ok(sessions
        .into_iter()
        .map(|s| AiSessionView {
            id: s.id.to_string(),
            project_id: s.project_id.to_string(),
            title: s.title,
            created_at: s.created_at.to_rfc3339(),
            updated_at: s.updated_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn ai_get_session(
    state: State<'_, AppState>,
    session_id: String,
) -> CommandResult<Vec<AiMessageView>> {
    let sid = Uuid::parse_str(&session_id)
        .map_err(|_| CommandError::validation("sessionId 不是合法 UUID"))?;
    let msgs = repo::list_ai_messages(&state.db, sid).await?;
    Ok(msgs
        .into_iter()
        .map(|m| AiMessageView {
            id: m.id.to_string(),
            seq: m.seq,
            role: m.role,
            content: m.content,
            tool_calls: m
                .tool_calls_json
                .and_then(|s| serde_json::from_str(&s).ok()),
            tool_call_id: m.tool_call_id,
            artifact: m.artifact_json.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: m.created_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn ai_delete_session(
    state: State<'_, AppState>,
    session_id: String,
) -> CommandResult<()> {
    let sid = Uuid::parse_str(&session_id)
        .map_err(|_| CommandError::validation("sessionId 不是合法 UUID"))?;
    // 在途循环先取消。
    if let Some(token) = state
        .ai_chats
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .cancels
        .remove(&session_id)
    {
        token.cancel();
    }
    repo::delete_ai_session(&state.db, sid).await?;
    Ok(())
}

/// 发送消息并启动 agent 循环（异步返回；流式事件经 `miyo:ai-chat` 推送）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_chat_send(
    app: AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    message: String,
    project_id: Option<Uuid>,
    locale: Option<String>,
) -> CommandResult<()> {
    if message.trim().is_empty() {
        return Err(CommandError::validation("消息不能为空"));
    }
    let llm = miyo_ai::LlmClient::new(require_ai_config(&state.db).await?)?;

    // 会话确保存在 + 标题。
    let sid = Uuid::parse_str(&session_id)
        .map_err(|_| CommandError::validation("sessionId 不是合法 UUID"))?;
    let pid = match project_id {
        Some(p) => p,
        None => state
            .active
            .read()
            .await
            .project_id
            .ok_or_else(|| CommandError::validation("请先选择项目"))?,
    };
    ensure_session(&state.db, sid, pid).await?;
    let is_first = repo::list_ai_messages(&state.db, sid).await?.is_empty();
    if is_first {
        let title: String = message.trim().chars().take(24).collect();
        repo::touch_ai_session(&state.db, sid, Some(&title)).await?;
    }

    // 控制面（工具执行通道；与外部 MCP Agent 同一权限 / 脱敏体系）。
    crate::commands::agent::ensure_started(&app).await?;
    let control = miyo_agent::ControlClient::discover_default().await?;

    // 会话历史 → LLM 消息（system 每轮重建，不持久化）。
    let mut messages = load_conversation(&state.db, sid).await?;
    let system = build_system_prompt(&state.db, pid, locale.as_deref()).await?;
    messages.insert(0, miyo_ai::ChatMessage::system(system));

    // 用户消息：持久化 + 入列。
    persist_message(
        &state.db,
        sid,
        "user",
        Some(message.trim()),
        None,
        None,
        None,
    )
    .await?;
    messages.push(miyo_ai::ChatMessage::user(message.trim().to_string()));

    // 工具清单（统一 Registry：按权限设置 + 停用列表过滤，与 tools/list 同规则）。
    let tools = build_tool_specs(&state.db).await?;

    // 运行注册：取消令牌。
    let token = CancellationToken::new();
    state
        .ai_chats
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .cancels
        .insert(session_id.clone(), token.clone());

    // spawn 循环任务。
    let db = state.db.clone();
    let registry = state.ai_chats.clone();
    let emit_app = app.clone();
    let event_session = session_id.clone();
    let persist_from = messages.len(); // system+user 之后的新消息需落库
    tauri::async_runtime::spawn(async move {
        let executor = ControlPlaneExecutor { client: control };
        let artifacts: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(Vec::new()));
        let observer = ChatObserver {
            app: emit_app.clone(),
            session_id: event_session.clone(),
            registry: registry.clone(),
            cancel: token.clone(),
            artifacts: artifacts.clone(),
        };
        let result = miyo_ai::run_agent_loop(
            &mut messages,
            miyo_ai::AgentLoopParams {
                client: &llm,
                tools: &tools,
                needs_approval: &needs_approval,
                executor: &executor,
                observer: &observer,
                cancel: token,
                max_iterations: MAX_LOOP_ITERATIONS,
            },
        )
        .await;

        // 落库本轮新增消息（assistant / tool）。
        for m in messages.iter().skip(persist_from) {
            let row = match m.role {
                miyo_ai::Role::Assistant => repo::AiMessageRow {
                    id: Uuid::new_v4(),
                    session_id: sid,
                    seq: 0,
                    role: "assistant".into(),
                    content: m.content.clone(),
                    tool_calls_json: m
                        .tool_calls
                        .as_ref()
                        .map(|c| serde_json::to_string(c).unwrap_or_default()),
                    tool_call_id: None,
                    artifact_json: None,
                    created_at: chrono::Utc::now(),
                },
                miyo_ai::Role::Tool => repo::AiMessageRow {
                    id: Uuid::new_v4(),
                    session_id: sid,
                    seq: 0,
                    role: "tool".into(),
                    content: m.content.clone(),
                    tool_calls_json: None,
                    tool_call_id: m.tool_call_id.clone(),
                    artifact_json: None,
                    created_at: chrono::Utc::now(),
                },
                _ => continue,
            };
            let _ = repo::append_ai_message(&db, &row).await;
        }
        // 产物消息落库（generate_* / analyze_* 的结构化产物）。
        let drained: Vec<serde_json::Value> = artifacts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
            .collect();
        for artifact in drained {
            let row = repo::AiMessageRow {
                id: Uuid::new_v4(),
                session_id: sid,
                seq: 0,
                role: "artifact".into(),
                content: None,
                tool_calls_json: None,
                tool_call_id: None,
                artifact_json: Some(serde_json::to_string(&artifact).unwrap_or_default()),
                created_at: chrono::Utc::now(),
            };
            let _ = repo::append_ai_message(&db, &row).await;
        }
        // 收尾事件 + 注册表清理（guard 在块内释放，不跨 await）。
        {
            registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .cancels
                .remove(&event_session);
        }
        match result {
            Ok(miyo_ai::LoopStop::Completed) => {
                emit_chat(
                    &emit_app,
                    &event_session,
                    "done",
                    serde_json::json!({"stopReason": "completed"}),
                );
            }
            Ok(miyo_ai::LoopStop::MaxIterations(n)) => {
                emit_chat(
                    &emit_app,
                    &event_session,
                    "done",
                    serde_json::json!({"stopReason": "max_iterations", "iterations": n}),
                );
            }
            Ok(miyo_ai::LoopStop::Cancelled) => {
                emit_chat(
                    &emit_app,
                    &event_session,
                    "cancelled",
                    serde_json::json!({}),
                );
            }
            Err(e) => {
                emit_chat(
                    &emit_app,
                    &event_session,
                    "error",
                    serde_json::json!({"message": e.to_string()}),
                );
                // 错误也留痕（assistant 角色），重载会话可见。
                let row = repo::AiMessageRow {
                    id: Uuid::new_v4(),
                    session_id: sid,
                    seq: 0,
                    role: "assistant".into(),
                    content: Some(format!("⚠ 循环异常终止：{e}")),
                    tool_calls_json: None,
                    tool_call_id: None,
                    artifact_json: None,
                    created_at: chrono::Utc::now(),
                };
                let _ = repo::append_ai_message(&db, &row).await;
            }
        }
    });
    Ok(())
}

/// 用户在确认卡上批准 / 拒绝。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_chat_approve(
    state: State<'_, AppState>,
    session_id: String,
    call_id: String,
    approved: bool,
) -> CommandResult<()> {
    let key = format!("{session_id}:{call_id}");
    let sender = state
        .ai_chats
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .approvals
        .remove(&key)
        .ok_or_else(|| {
            CommandError::with_code(
                CommandErrorCode::AiNoPending,
                "该调用不在待确认状态（可能已处理）",
            )
        })?;
    let _ = sender.send(approved);
    Ok(())
}

/// 取消会话在途循环。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_chat_cancel(state: State<'_, AppState>, session_id: String) -> CommandResult<()> {
    let token = state
        .ai_chats
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .cancels
        .get(&session_id)
        .cloned();
    if let Some(t) = token {
        t.cancel();
    }
    Ok(())
}

/// 循环迭代上限（防失控；达到后 done 事件给 max_iterations）。
const MAX_LOOP_ITERATIONS: usize = 15;

/// 副作用工具需要确认（registry 元数据单源；未知工具默认要确认）。
fn needs_approval(name: &str) -> bool {
    match tool_meta(name) {
        Some(meta) => meta.side_effects || meta.destructive,
        None => true,
    }
}

async fn ensure_session(db: &sqlx::SqlitePool, sid: Uuid, project_id: Uuid) -> CommandResult<()> {
    if repo::list_ai_sessions(db, project_id)
        .await?
        .iter()
        .any(|s| s.id == sid)
    {
        return Ok(());
    }
    // 不存在则创建（前端可能在首条消息前预创建）。
    repo::create_ai_session(
        db,
        &repo::AiSession {
            id: sid,
            project_id,
            title: String::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        },
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn persist_message(
    db: &sqlx::SqlitePool,
    session_id: Uuid,
    role: &str,
    content: Option<&str>,
    tool_calls_json: Option<String>,
    tool_call_id: Option<String>,
    artifact_json: Option<String>,
) -> CommandResult<()> {
    repo::append_ai_message(
        db,
        &repo::AiMessageRow {
            id: Uuid::new_v4(),
            session_id,
            seq: 0,
            role: role.to_string(),
            content: content.map(str::to_string),
            tool_calls_json,
            tool_call_id,
            artifact_json,
            created_at: chrono::Utc::now(),
        },
    )
    .await?;
    repo::touch_ai_session(db, session_id, None).await?;
    Ok(())
}

/// 会话历史 → LLM 消息（不含 system）。
async fn load_conversation(
    db: &sqlx::SqlitePool,
    session_id: Uuid,
) -> CommandResult<Vec<miyo_ai::ChatMessage>> {
    let rows = repo::list_ai_messages(db, session_id).await?;
    let mut out = Vec::new();
    for m in rows {
        match m.role.as_str() {
            "user" => out.push(miyo_ai::ChatMessage::user(m.content.unwrap_or_default())),
            "assistant" => {
                let tool_calls = m
                    .tool_calls_json
                    .and_then(|s| serde_json::from_str(&s).ok());
                out.push(miyo_ai::ChatMessage::assistant_with_tool_calls(
                    m.content,
                    tool_calls.unwrap_or_default(),
                ));
            }
            "tool" => out.push(miyo_ai::ChatMessage::tool_result(
                m.tool_call_id.unwrap_or_default(),
                m.content.unwrap_or_default(),
            )),
            // system 不持久化；artifact 不进 LLM 上下文（纯展示）。
            _ => {}
        }
    }
    Ok(out)
}

/// 统一 Registry → LLM 工具清单（权限级别 + 停用列表过滤）。
async fn build_tool_specs(db: &sqlx::SqlitePool) -> CommandResult<Vec<miyo_ai::ToolSpec>> {
    let levels_raw = repo::get_setting(db, miyo_agent::server::MCP_PERMISSIONS_KEY).await?;
    let enabled = registry::parse_enabled_levels(levels_raw.as_deref());
    let disabled: Vec<String> = repo::get_setting(db, miyo_agent::server::MCP_DISABLED_TOOLS_KEY)
        .await?
        .and_then(|v| serde_json::from_str(&v).ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for def in registry::tool_definitions_json() {
        let Some(name) = def["name"].as_str() else {
            continue;
        };
        let Some(meta) = tool_meta(name) else {
            continue;
        };
        if !enabled.contains(&meta.permission) || disabled.iter().any(|d| d == name) {
            continue;
        }
        out.push(miyo_ai::ToolSpec::new(
            name,
            def["description"].as_str().unwrap_or(""),
            def["inputSchema"].clone(),
        ));
    }
    Ok(out)
}

/// 系统提示词（注入项目现场 + 工具守则；随 UI 语言）。
async fn build_system_prompt(
    db: &sqlx::SqlitePool,
    project_id: Uuid,
    locale: Option<&str>,
) -> CommandResult<String> {
    let en = locale == Some("en");
    let project = repo::get_project(db, project_id).await.map_err(|e| {
        CommandError::with_code(CommandErrorCode::NotFound, format!("项目不存在：{e}"))
    })?;
    let endpoints = repo::list_endpoints(db, project_id).await?;
    let scenarios = repo::list_scenarios(db, project_id).await?;
    let environments = repo::list_environments(db, project_id).await?;

    let mut ep_lines = String::new();
    for ep in endpoints.iter().take(100) {
        ep_lines.push_str(&format!(
            "- [{}] {} {} （id: {}）{}\n",
            ep.id,
            ep.method.as_str(),
            ep.path,
            ep.name,
            if ep.description.is_empty() {
                String::new()
            } else {
                format!("—— {}", ep.description)
            }
        ));
    }
    let mut sc_lines = String::new();
    for sc in scenarios.iter().take(30) {
        sc_lines.push_str(&format!(
            "- {}（id: {}，{} 步）\n",
            sc.name,
            sc.id,
            sc.steps.len()
        ));
    }
    let mut env_lines = String::new();
    for env in &environments {
        env_lines.push_str(&format!("- {}（base_url: {}）\n", env.name, env.base_url));
    }

    if en {
        Ok(format!(
            "You are the built-in AI testing assistant of Miyo (an API debugging tool).\n\
             Current project: \"{name}\" ({pid}).\n\n\
             Project endpoints:\n{eps}\nScenarios:\n{scs}\nEnvironments:\n{envs}\n\
             Rules:\n\
             1. Analyze before acting: call analyze_api_dependencies before designing a scenario to confirm step order and extraction chains.\n\
             2. Produce drafts first (generate_integration_scenario / generate_test_cases), show the preview and wait for user confirmation before calling create_scenario / create_test_case to persist.\n\
             3. Side-effect operations (run_scenario / send_endpoint / writes) trigger a confirmation card; if denied, do NOT retry — explain instead.\n\
             4. Reference variables as {{{{varName}}}}; earlier steps extract, later steps consume.\n\
             5. Answer in English; back conclusions with evidence (step, field, status code).",
            name = project.name,
            pid = project.id,
            eps = ep_lines,
            scs = sc_lines,
            envs = env_lines,
        ))
    } else {
        Ok(format!(
            "你是 Miyo（API 调试工具）内置的 AI 测试助手。\n\
             当前项目：「{name}」（{pid}）。\n\n\
             项目接口清单：\n{eps}\n已有场景：\n{scs}\n环境：\n{envs}\n\
             工作守则：\n\
             1. 先分析再动手：设计场景前先调 analyze_api_dependencies，确认步骤顺序与提取链。\n\
             2. 先出草稿（generate_integration_scenario / generate_test_cases），向用户展示预览并等待确认，再调 create_scenario / create_test_case 保存。\n\
             3. 有副作用的操作（run_scenario / send_endpoint / 写库）系统会弹确认卡；被拒绝时不要重试，向用户说明影响。\n\
             4. 引用变量一律写 {{{{变量名}}}}，前步提取、后步引用；缺失的提取规则用工具自动补齐能力或手动补。\n\
             5. 用中文回答；结论必须给证据（步骤 / 字段 / 状态码）。",
            name = project.name,
            pid = project.id,
            eps = ep_lines,
            scs = sc_lines,
            envs = env_lines,
        ))
    }
}

/// 控制面工具执行器（miyo-ai ToolExecutor → dispatch → 控制面 HTTP）。
struct ControlPlaneExecutor {
    client: miyo_agent::ControlClient,
}

#[async_trait::async_trait]
impl miyo_ai::ToolExecutor for ControlPlaneExecutor {
    async fn execute(&self, name: &str, arguments_json: &str) -> Result<serde_json::Value, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments_json).unwrap_or(serde_json::json!({}));
        miyo_agent::dispatch::dispatch_tool(&self.client, name, &args)
            .await
            .map_err(|e| e.to_string())
    }
}

/// 产物型工具：结果进右栏预览 + 落 artifact 消息。
fn artifact_type(name: &str) -> Option<&'static str> {
    match name {
        "generate_integration_scenario" => Some("scenario_draft"),
        "generate_test_cases" => Some("test_case_drafts"),
        "propose_scenario_fix" => Some("fix_proposal"),
        "analyze_project_test_coverage" => Some("coverage_report"),
        "analyze_api_dependencies" => Some("dependency_report"),
        "analyze_run_failure" => Some("failure_analysis"),
        _ => None,
    }
}

/// 聊天循环观察者：流式事件 + 审批暂停 + 产物收集。
struct ChatObserver {
    app: AppHandle,
    session_id: String,
    registry: Arc<Mutex<AiChatRegistry>>,
    cancel: CancellationToken,
    artifacts: Arc<Mutex<Vec<serde_json::Value>>>,
}

fn emit_chat(app: &AppHandle, session_id: &str, kind: &str, payload: serde_json::Value) {
    let mut event = serde_json::json!({ "sessionId": session_id, "type": kind });
    if let (Some(base), Some(obj)) = (event.as_object_mut(), payload.as_object()) {
        for (k, v) in obj {
            base.insert(k.clone(), v.clone());
        }
    }
    let _ = app.emit(CHAT_EVENT, event);
}

#[async_trait::async_trait]
impl miyo_ai::LoopObserver for ChatObserver {
    fn on_content_delta(&self, delta: &str) {
        emit_chat(
            &self.app,
            &self.session_id,
            "delta",
            serde_json::json!({ "delta": delta }),
        );
    }

    fn on_tool_start(&self, call: &miyo_ai::ToolCall) {
        emit_chat(
            &self.app,
            &self.session_id,
            "tool_start",
            serde_json::json!({
                "callId": call.id,
                "name": call.function.name,
                "args": serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                    .unwrap_or(serde_json::json!({})),
            }),
        );
    }

    fn on_tool_result(&self, call: &miyo_ai::ToolCall, ok: bool, result: &serde_json::Value) {
        emit_chat(
            &self.app,
            &self.session_id,
            "tool_result",
            serde_json::json!({
                "callId": call.id,
                "name": call.function.name,
                "ok": ok,
                "result": result,
            }),
        );
        if let Some(kind) = artifact_type(&call.function.name) {
            // 用例草稿包一层 endpointId（前端「保存所选」需要定位目标接口）。
            let data = if kind == "test_case_drafts" {
                let endpoint_id =
                    serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                        .ok()
                        .and_then(|a| a.get("endpointId").cloned())
                        .unwrap_or(serde_json::Value::Null);
                serde_json::json!({ "endpointId": endpoint_id, "drafts": result })
            } else {
                result.clone()
            };
            let artifact = serde_json::json!({ "artifactType": kind, "data": data });
            self.artifacts
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push(artifact.clone());
            emit_chat(&self.app, &self.session_id, "artifact", artifact);
        }
    }

    async fn approve_tool(&self, call: &miyo_ai::ToolCall) -> bool {
        let meta = tool_meta(&call.function.name);
        let risk = match meta {
            Some(m) if m.destructive => "high",
            Some(m) if m.open_world => "network",
            _ => "write",
        };
        let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
        let key = format!("{}:{}", self.session_id, call.id);
        self.registry
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .approvals
            .insert(key, tx);
        emit_chat(
            &self.app,
            &self.session_id,
            "pending",
            serde_json::json!({
                "callId": call.id,
                "name": call.function.name,
                "args": serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                    .unwrap_or(serde_json::json!({})),
                "risk": risk,
            }),
        );
        tokio::select! {
            _ = self.cancel.cancelled() => false,
            r = rx => r.unwrap_or(false),
        }
    }
}

// ---------- 3. 直连分析（不走 LLM） ----------

/// 项目接口依赖分析。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_analyze_dependencies(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<serde_json::Value> {
    let report =
        miyo_agent::analysis::dependencies::analyze_api_dependencies(&state.db, project_id).await?;
    serde_json::to_value(report)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string()))
}

/// 项目测试覆盖率（七维）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_analyze_coverage(
    state: State<'_, AppState>,
    project_id: Uuid,
) -> CommandResult<serde_json::Value> {
    let report =
        miyo_agent::analysis::coverage::analyze_project_test_coverage(&state.db, project_id)
            .await?;
    serde_json::to_value(report)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string()))
}

/// 测试用例草稿（不落库）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_generate_test_case_drafts(
    state: State<'_, AppState>,
    endpoint_id: Uuid,
) -> CommandResult<serde_json::Value> {
    let drafts =
        miyo_agent::analysis::testcase_gen::generate_test_case_drafts(&state.db, endpoint_id)
            .await?;
    serde_json::to_value(drafts)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string()))
}

/// 场景草稿物化（不落库）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_generate_scenario_draft(
    state: State<'_, AppState>,
    draft: miyo_agent::analysis::scenario_gen::ScenarioDraftRequest,
) -> CommandResult<serde_json::Value> {
    let result =
        miyo_agent::analysis::scenario_gen::materialize_scenario_draft(&state.db, &draft).await?;
    serde_json::to_value(result)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string()))
}

/// 修复草稿（不落库）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_propose_fix(
    state: State<'_, AppState>,
    run_id: Uuid,
    notes: Option<String>,
) -> CommandResult<serde_json::Value> {
    let proposal = miyo_agent::analysis::fix::propose_fix_for_run(&state.db, run_id, notes).await?;
    serde_json::to_value(proposal)
        .map_err(|e| CommandError::with_code(CommandErrorCode::Internal, e.to_string()))
}

// ---------- 4. 场景草稿历史 ----------

/// 草稿视图（JSON 解析后回传）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioDraftView {
    pub id: String,
    pub scenario_id: String,
    pub base: serde_json::Value,
    pub draft: serde_json::Value,
    pub summary: String,
    pub source: String,
    pub created_at: String,
    pub applied_at: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn ai_save_scenario_draft(
    state: State<'_, AppState>,
    scenario_id: Uuid,
    base: serde_json::Value,
    draft: serde_json::Value,
    summary: Option<String>,
    source: Option<String>,
) -> CommandResult<String> {
    // base / draft 都必须是完整 Scenario JSON（应用 / 撤销 = 整体回写）。
    for (label, v) in [("base", &base), ("draft", &draft)] {
        if serde_json::from_value::<miyo_core::model::Scenario>(v.clone()).is_err() {
            return Err(CommandError::validation(format!(
                "{label} 不是合法的场景 JSON（须为完整 Scenario）"
            )));
        }
    }
    let id = Uuid::new_v4();
    repo::save_scenario_draft(
        &state.db,
        &repo::ScenarioDraftRow {
            id,
            scenario_id,
            base_json: serde_json::to_string(&base).unwrap_or_default(),
            draft_json: serde_json::to_string(&draft).unwrap_or_default(),
            summary: summary.unwrap_or_default(),
            source: source.unwrap_or_else(|| "ai_fix".into()),
            created_at: chrono::Utc::now(),
            applied_at: None,
        },
    )
    .await?;
    Ok(id.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn ai_list_scenario_drafts(
    state: State<'_, AppState>,
    scenario_id: Uuid,
    limit: Option<i64>,
) -> CommandResult<Vec<ScenarioDraftView>> {
    let rows = repo::list_scenario_drafts(&state.db, scenario_id, limit.unwrap_or(20)).await?;
    Ok(rows
        .into_iter()
        .map(|r| ScenarioDraftView {
            id: r.id.to_string(),
            scenario_id: r.scenario_id.to_string(),
            base: serde_json::from_str(&r.base_json).unwrap_or(serde_json::Value::Null),
            draft: serde_json::from_str(&r.draft_json).unwrap_or(serde_json::Value::Null),
            summary: r.summary,
            source: r.source,
            created_at: r.created_at.to_rfc3339(),
            applied_at: r.applied_at.map(|a| a.to_rfc3339()),
        })
        .collect())
}

/// 应用草稿：把 draft_json 整体回写为当前场景（留痕后标记已应用）。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_apply_scenario_draft(
    state: State<'_, AppState>,
    draft_id: Uuid,
) -> CommandResult<()> {
    let row = repo::get_scenario_draft(&state.db, draft_id)
        .await?
        .ok_or_else(|| {
            CommandError::with_code(CommandErrorCode::NotFound, "草稿不存在或已被清理")
        })?;
    let scenario: miyo_core::model::Scenario = serde_json::from_str(&row.draft_json)
        .map_err(|e| CommandError::validation(format!("草稿 JSON 损坏：{e}")))?;
    repo::save_scenario(&state.db, &scenario).await?;
    repo::mark_scenario_draft_applied(&state.db, draft_id).await?;
    Ok(())
}

/// 撤销：把 base_json（应用前快照）回写为当前场景。
#[tauri::command(rename_all = "camelCase")]
pub async fn ai_revert_scenario_draft(
    state: State<'_, AppState>,
    draft_id: Uuid,
) -> CommandResult<()> {
    let row = repo::get_scenario_draft(&state.db, draft_id)
        .await?
        .ok_or_else(|| {
            CommandError::with_code(CommandErrorCode::NotFound, "草稿不存在或已被清理")
        })?;
    let scenario: miyo_core::model::Scenario = serde_json::from_str(&row.base_json)
        .map_err(|e| CommandError::validation(format!("草稿基线 JSON 损坏：{e}")))?;
    repo::save_scenario(&state.db, &scenario).await?;
    Ok(())
}
