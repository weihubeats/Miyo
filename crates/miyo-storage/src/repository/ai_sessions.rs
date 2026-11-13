//! AI 聊天会话（应用内 AI 测试中心的持久化对话）。
//!
//! 会话按项目归属；消息按 `seq` 有序追加（模型消息 / 工具调用 / 工具结果
//! / 产物快照统一走一张表，role 区分）。会话删除级联清消息（迁移 0028）。

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use uuid::Uuid;

use miyo_core::{AppError, Result};

/// ai_messages 行元组（sqlx query_as 解包用）。
type AiMessageTuple = (
    String,
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// 聊天会话。
#[derive(Debug, Clone, PartialEq)]
pub struct AiSession {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// 单条消息（role: system / user / assistant / tool / artifact）。
#[derive(Debug, Clone, PartialEq)]
pub struct AiMessageRow {
    pub id: Uuid,
    pub session_id: Uuid,
    pub seq: i64,
    pub role: String,
    pub content: Option<String>,
    /// assistant 工具调用（miyo-ai ToolCall 数组 JSON）。
    pub tool_calls_json: Option<String>,
    /// tool 消息对应的调用 id。
    pub tool_call_id: Option<String>,
    /// 结构化产物快照（场景草稿 / 用例草稿 / 修复 diff / 覆盖率报告）。
    pub artifact_json: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// 新建会话。
pub async fn create_ai_session(db: &SqlitePool, session: &AiSession) -> Result<()> {
    sqlx::query("INSERT INTO ai_sessions (id, project_id, title, created_at, updated_at) VALUES (?, ?, ?, ?, ?)")
        .bind(session.id.to_string())
        .bind(session.project_id.to_string())
        .bind(&session.title)
        .bind(session.created_at.to_rfc3339())
        .bind(session.updated_at.to_rfc3339())
        .execute(db)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// 项目会话列表（新→旧）。
pub async fn list_ai_sessions(db: &SqlitePool, project_id: Uuid) -> Result<Vec<AiSession>> {
    let rows: Vec<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id, project_id, title, created_at, updated_at
         FROM ai_sessions WHERE project_id = ? ORDER BY updated_at DESC",
    )
    .bind(project_id.to_string())
    .fetch_all(db)
    .await
    .map_err(AppError::from)?;
    rows.into_iter()
        .map(|(id, pid, title, created, updated)| {
            Ok(AiSession {
                id: Uuid::parse_str(&id).map_err(uuid_err)?,
                project_id: Uuid::parse_str(&pid).map_err(uuid_err)?,
                title,
                created_at: DateTime::parse_from_rfc3339(&created)
                    .map_err(ts_err)?
                    .with_timezone(&Utc),
                updated_at: DateTime::parse_from_rfc3339(&updated)
                    .map_err(ts_err)?
                    .with_timezone(&Utc),
            })
        })
        .collect()
}

/// 触碰会话（更新 updated_at；可选改标题——首条用户消息生成标题时用）。
pub async fn touch_ai_session(
    db: &SqlitePool,
    session_id: Uuid,
    title: Option<&str>,
) -> Result<()> {
    match title {
        Some(t) => sqlx::query("UPDATE ai_sessions SET updated_at = ?, title = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(t)
            .bind(session_id.to_string()),
        None => sqlx::query("UPDATE ai_sessions SET updated_at = ? WHERE id = ?")
            .bind(Utc::now().to_rfc3339())
            .bind(session_id.to_string()),
    }
    .execute(db)
    .await
    .map_err(AppError::from)?;
    Ok(())
}

/// 删除会话（消息级联）。
pub async fn delete_ai_session(db: &SqlitePool, session_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM ai_sessions WHERE id = ?")
        .bind(session_id.to_string())
        .execute(db)
        .await
        .map_err(AppError::from)?;
    Ok(())
}

/// 追加消息（seq = 当前最大 +1，调用方无需传）。
pub async fn append_ai_message(db: &SqlitePool, msg: &AiMessageRow) -> Result<i64> {
    let (max,): (Option<i64>,) =
        sqlx::query_as("SELECT MAX(seq) FROM ai_messages WHERE session_id = ?")
            .bind(msg.session_id.to_string())
            .fetch_one(db)
            .await
            .map_err(AppError::from)?;
    let seq = max.unwrap_or(0) + 1;
    sqlx::query(
        "INSERT INTO ai_messages (id, session_id, seq, role, content, tool_calls_json, tool_call_id, artifact_json, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(msg.id.to_string())
    .bind(msg.session_id.to_string())
    .bind(seq)
    .bind(&msg.role)
    .bind(&msg.content)
    .bind(&msg.tool_calls_json)
    .bind(&msg.tool_call_id)
    .bind(&msg.artifact_json)
    .bind(msg.created_at.to_rfc3339())
    .execute(db)
    .await
    .map_err(AppError::from)?;
    Ok(seq)
}

/// 会话消息（seq 升序）。
pub async fn list_ai_messages(db: &SqlitePool, session_id: Uuid) -> Result<Vec<AiMessageRow>> {
    let rows: Vec<AiMessageTuple> = sqlx::query_as(
        "SELECT id, seq, role, content, tool_calls_json, tool_call_id, artifact_json, created_at
             FROM ai_messages WHERE session_id = ? ORDER BY seq",
    )
    .bind(session_id.to_string())
    .fetch_all(db)
    .await
    .map_err(AppError::from)?;
    rows.into_iter()
        .map(
            |(id, seq, role, content, calls, call_id, artifact, created)| {
                Ok(AiMessageRow {
                    id: Uuid::parse_str(&id).map_err(uuid_err)?,
                    session_id,
                    seq,
                    role,
                    content,
                    tool_calls_json: calls,
                    tool_call_id: call_id,
                    artifact_json: artifact,
                    created_at: DateTime::parse_from_rfc3339(&created)
                        .map_err(ts_err)?
                        .with_timezone(&Utc),
                })
            },
        )
        .collect()
}

fn uuid_err(e: uuid::Error) -> AppError {
    AppError::Database(sqlx::Error::Protocol(format!("UUID 解析失败：{e}")))
}

fn ts_err(e: chrono::ParseError) -> AppError {
    AppError::Database(sqlx::Error::Protocol(format!("时间戳解析失败：{e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;

    async fn session(db: &SqlitePool) -> AiSession {
        let s = AiSession {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            title: "测试会话".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        create_ai_session(db, &s).await.unwrap();
        s
    }

    #[tokio::test]
    async fn session_and_messages_roundtrip() {
        let db = memory_pool().await.unwrap();
        let s = session(&db).await;
        let list = list_ai_sessions(&db, s.project_id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].title, "测试会话");

        let m1 = AiMessageRow {
            id: Uuid::new_v4(),
            session_id: s.id,
            seq: 0,
            role: "user".into(),
            content: Some("帮我测下单链路".into()),
            tool_calls_json: None,
            tool_call_id: None,
            artifact_json: None,
            created_at: Utc::now(),
        };
        let seq1 = append_ai_message(&db, &m1).await.unwrap();
        assert_eq!(seq1, 1);
        let m2 = AiMessageRow {
            id: Uuid::new_v4(),
            role: "tool".into(),
            tool_call_id: Some("call_1".into()),
            content: Some("{\"ok\":true}".into()),
            ..m1.clone()
        };
        let seq2 = append_ai_message(&db, &m2).await.unwrap();
        assert_eq!(seq2, 2);

        let msgs = list_ai_messages(&db, s.id).await.unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].content.as_deref(), Some("帮我测下单链路"));
        assert_eq!(msgs[1].tool_call_id.as_deref(), Some("call_1"));

        // 删除会话级联清消息。
        delete_ai_session(&db, s.id).await.unwrap();
        assert!(list_ai_messages(&db, s.id).await.unwrap().is_empty());
        assert!(list_ai_sessions(&db, s.project_id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn touch_updates_title() {
        let db = memory_pool().await.unwrap();
        let s = session(&db).await;
        touch_ai_session(&db, s.id, Some("新标题")).await.unwrap();
        let list = list_ai_sessions(&db, s.project_id).await.unwrap();
        assert_eq!(list[0].title, "新标题");
    }
}
