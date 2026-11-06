-- 集成测试工作流：场景分类 + 运行历史元信息（2026-11）。
--
-- scenarios.category：场景库分类过滤标签（空 = 未分类）。
-- scenario_runs.meta_json：运行元信息 JSON 透传（started_at / 场景配置快照 /
--   初始变量（脱敏）/ 执行策略 / 未定义变量警告）——历史基于当时配置快照
--   回看与重跑；旧行 NULL 视为无快照。
ALTER TABLE scenarios ADD COLUMN category TEXT NOT NULL DEFAULT '';
ALTER TABLE scenario_runs ADD COLUMN meta_json TEXT;
