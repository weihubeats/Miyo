-- 测试用例断言 / 提取 / 执行控制列（2026-11 测试用例管理与执行中心）。
-- 存量行默认值兜底：断言 / 提取空数组 = 维持「2xx-3xx 即通过」现状；
-- sort_order 默认 0，列表按 (sort_order, created_at) 排序自然回落旧序。
ALTER TABLE test_cases ADD COLUMN assertions TEXT NOT NULL DEFAULT '[]';
ALTER TABLE test_cases ADD COLUMN extractors TEXT NOT NULL DEFAULT '[]';
ALTER TABLE test_cases ADD COLUMN auth TEXT;
ALTER TABLE test_cases ADD COLUMN timeout_ms INTEGER;
ALTER TABLE test_cases ADD COLUMN disable_cookies INTEGER NOT NULL DEFAULT 0;
ALTER TABLE test_cases ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
ALTER TABLE test_cases ADD COLUMN last_run_at TEXT;
ALTER TABLE test_cases ADD COLUMN last_run_ok INTEGER;
ALTER TABLE test_cases ADD COLUMN last_run_fail INTEGER;
