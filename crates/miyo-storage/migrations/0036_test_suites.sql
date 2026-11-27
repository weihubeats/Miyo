-- 测试套件与批量执行（Test Suite / Test Run Batch）。
--
-- 设计要点：
-- * 套件与场景分表：test_suite_items 只是「引用 + 排序 + 启用」，
--   同一场景可被多个套件引用，场景本身的步骤仍存 scenarios.steps_json；
-- * 批次与批次明细独立落库：应用异常退出后，启动时把 running/pending/paused
--   批次标记为 interrupted，明细里未完成的项同样标记 interrupted（可恢复 /
--   重新执行），不会把「中断」误报成「失败」；
-- * 明细保留场景名快照（scenario_name）：场景被改名 / 删除后历史报告仍可读，
--   因此 scenario_id 不设外键（不随场景删除级联清历史）；
-- * 套件删除不级联删除批次历史（suite_id 同样无外键）。
CREATE TABLE IF NOT EXISTS test_suites (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    -- 归属业务目录（复用 scenarios 的目录树；NULL = 未分类）。
    folder_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_test_suites_project
    ON test_suites(project_id, name);

CREATE TABLE IF NOT EXISTS test_suite_items (
    id TEXT PRIMARY KEY,
    suite_id TEXT NOT NULL REFERENCES test_suites(id) ON DELETE CASCADE,
    scenario_id TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    enabled INTEGER NOT NULL DEFAULT 1,
    UNIQUE(suite_id, scenario_id)
);

CREATE INDEX IF NOT EXISTS idx_test_suite_items_suite
    ON test_suite_items(suite_id, sort_order);
CREATE INDEX IF NOT EXISTS idx_test_suite_items_scenario
    ON test_suite_items(scenario_id);

CREATE TABLE IF NOT EXISTS test_run_batches (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL,
    -- 来源套件（None = 手动勾选 / 文件夹批量执行的临时批次）。
    suite_id TEXT,
    -- 重跑批次指向原批次（失败场景重试 / 整批重跑都新建批次，不覆盖历史）。
    parent_batch_id TEXT,
    environment_id TEXT,
    -- pending / running / paused / completed / failed / cancelled / interrupted
    status TEXT NOT NULL,
    -- serial / parallel
    mode TEXT NOT NULL DEFAULT 'serial',
    concurrency INTEGER NOT NULL DEFAULT 1,
    -- continue / stop / retry / skip_dependents
    failure_policy TEXT NOT NULL DEFAULT 'continue',
    retry_count INTEGER NOT NULL DEFAULT 0,
    skip_disabled INTEGER NOT NULL DEFAULT 1,
    total_count INTEGER NOT NULL DEFAULT 0,
    passed_count INTEGER NOT NULL DEFAULT 0,
    failed_count INTEGER NOT NULL DEFAULT 0,
    skipped_count INTEGER NOT NULL DEFAULT 0,
    cancelled_count INTEGER NOT NULL DEFAULT 0,
    running_count INTEGER NOT NULL DEFAULT 0,
    pending_count INTEGER NOT NULL DEFAULT 0,
    started_at TEXT,
    finished_at TEXT,
    duration_ms REAL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_test_run_batches_project
    ON test_run_batches(project_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_test_run_batches_suite
    ON test_run_batches(suite_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_test_run_batches_status
    ON test_run_batches(status);

CREATE TABLE IF NOT EXISTS test_run_batch_items (
    id TEXT PRIMARY KEY,
    batch_id TEXT NOT NULL REFERENCES test_run_batches(id) ON DELETE CASCADE,
    scenario_id TEXT NOT NULL,
    -- 场景名快照（场景改名 / 删除后报告仍可读）。
    scenario_name TEXT NOT NULL DEFAULT '',
    -- pending / running / passed / failed / skipped / cancelled / interrupted
    status TEXT NOT NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    -- 已执行次数（失败重试策略下 > 1）。
    attempt INTEGER NOT NULL DEFAULT 0,
    duration_ms REAL,
    error_message TEXT,
    -- 对应的 scenario_runs.id（单场景执行记录，点击明细可回看）。
    execution_record_id TEXT,
    started_at TEXT,
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_test_run_batch_items_batch
    ON test_run_batch_items(batch_id, sort_order);
