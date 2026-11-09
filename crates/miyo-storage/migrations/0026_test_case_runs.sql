-- 测试用例执行历史（每次运行一行，append-only；口径镜像 scenario_runs：
-- 64KB 正文截断 + 敏感头/变量脱敏在写入方完成，保留策略每用例 20 条）。
CREATE TABLE IF NOT EXISTS test_case_runs (
    id TEXT PRIMARY KEY NOT NULL,
    case_id TEXT NOT NULL REFERENCES test_cases(id) ON DELETE CASCADE,
    endpoint_id TEXT,
    environment_id TEXT,
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    duration_ms REAL,
    status TEXT NOT NULL DEFAULT 'Failed',
    status_code INTEGER,
    ok_count INTEGER NOT NULL DEFAULT 0,
    fail_count INTEGER NOT NULL DEFAULT 0,
    error TEXT,
    results_json TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_test_case_runs_case ON test_case_runs(case_id, started_at DESC);
