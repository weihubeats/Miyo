-- 场景运行历史：run_scenario 每次完成落一行，供结果面板回看与重开恢复。
--
-- results_json 为步骤结果数组（EndpointResult 序列化；response 正文已剥离
-- 控制体积——状态码 / 断言明细 / 提取值 / 实际 URL 均保留）。variables_json
-- 为最终变量池（链路产物，重开恢复运行时值用）。每场景仅保留最近 N 条，
-- 由命令层插入后修剪（见 repo::prune_scenario_runs）。
CREATE TABLE IF NOT EXISTS scenario_runs (
    id TEXT PRIMARY KEY,
    scenario_id TEXT NOT NULL REFERENCES scenarios(id) ON DELETE CASCADE,
    environment_id TEXT,
    finished_at TEXT NOT NULL,
    duration_ms REAL,
    total_steps INTEGER NOT NULL,
    ok_count INTEGER NOT NULL,
    fail_count INTEGER NOT NULL,
    skipped_count INTEGER NOT NULL,
    cancelled INTEGER NOT NULL DEFAULT 0,
    stopped_on_failure INTEGER NOT NULL DEFAULT 0,
    results_json TEXT NOT NULL,
    variables_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scenario_runs_scenario
    ON scenario_runs(scenario_id, finished_at DESC);
