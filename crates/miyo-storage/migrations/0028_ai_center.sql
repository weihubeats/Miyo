-- AI 测试中心：聊天会话 / 消息 / 场景修复草稿。
--
-- ai_sessions + ai_messages：应用内 AI 对话的持久化（重启后可回看；
-- 消息按 seq 有序，工具调用与产物快照随行）。会话删除级联清消息。
-- scenario_drafts：AI 修复 / 场景生成的草稿留档（需求五「保留修改历史 +
-- 允许撤销」）——base_json 存应用前场景快照（撤销即回写），draft_json 存
-- 草稿本体；applied_at 标记是否已应用（未应用的草稿可直接应用）。
CREATE TABLE IF NOT EXISTS ai_sessions (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ai_sessions_project
    ON ai_sessions(project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS ai_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES ai_sessions(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    role TEXT NOT NULL,
    content TEXT,
    tool_calls_json TEXT,
    tool_call_id TEXT,
    artifact_json TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_ai_messages_session
    ON ai_messages(session_id, seq);

CREATE TABLE IF NOT EXISTS scenario_drafts (
    id TEXT PRIMARY KEY,
    scenario_id TEXT NOT NULL REFERENCES scenarios(id) ON DELETE CASCADE,
    base_json TEXT NOT NULL,
    draft_json TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    source TEXT NOT NULL DEFAULT 'ai_fix',
    created_at TEXT NOT NULL,
    applied_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_scenario_drafts_scenario
    ON scenario_drafts(scenario_id, created_at DESC);
