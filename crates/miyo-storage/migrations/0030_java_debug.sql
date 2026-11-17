-- Java 方法实时调试（MVP）：
-- java_projects   调试项目配置（Runtime 连接信息 + 安全策略），token 走 AES-GCM 加密列；
-- java_invocations 执行历史（审计 + 重放），大字段截断落库、按项目滚动清理。
--
-- 关联设计：project_id 可空——Java 调试项目可独立存在（不依赖 Miyo HTTP 项目），
-- 关联后复用该项目的环境变量与集成测试上下文。

CREATE TABLE IF NOT EXISTS java_projects (
    id TEXT PRIMARY KEY,
    project_id TEXT REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    runtime_endpoint TEXT NOT NULL,
    environment_id TEXT,
    auth_json TEXT NOT NULL DEFAULT '{}',
    timeout_ms INTEGER NOT NULL DEFAULT 30000,
    max_result_bytes INTEGER NOT NULL DEFAULT 1048576,
    max_concurrent INTEGER NOT NULL DEFAULT 4,
    description TEXT NOT NULL DEFAULT '',
    last_connected_at TEXT,
    last_info_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_java_projects_project ON java_projects(project_id, updated_at DESC);

CREATE TABLE IF NOT EXISTS java_invocations (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES java_projects(id) ON DELETE CASCADE,
    class_name TEXT NOT NULL,
    method_id TEXT NOT NULL,
    method_signature TEXT NOT NULL DEFAULT '',
    args_json TEXT NOT NULL DEFAULT '[]',
    status TEXT NOT NULL,
    result_json TEXT,
    result_truncated INTEGER NOT NULL DEFAULT 0,
    exception_json TEXT,
    duration_ms REAL,
    started_at TEXT,
    finished_at TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_java_invocations_project ON java_invocations(project_id, created_at DESC);
