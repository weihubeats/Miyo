-- 0034：查询索引补齐（架构审查 Phase 6）
--
-- 1. ai_messages 复合索引：list_ai_messages / list_ai_messages_after 按
--    (session_id, seq) 排序，单列 session_id 索引在长会话下退化为 filesort；
-- 2. test_runs 复合索引：list_test_runs 按 (project_id, started_at DESC)
--    取每项目最近 N 条——与滚动清理同口径。

CREATE INDEX IF NOT EXISTS idx_ai_messages_session_seq
    ON ai_messages(session_id, seq);

CREATE INDEX IF NOT EXISTS idx_test_runs_project_started
    ON test_runs(project_id, started_at DESC);
