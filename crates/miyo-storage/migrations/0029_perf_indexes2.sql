-- 性能补索引（二）：
-- 1. list_recent_activities 跨项目 ORDER BY created_at DESC LIMIT 20（仪表板
--    每次打开都跑）：现有索引全部 project_id 前导，跨项目排序需全表排序。
-- 2. list_endpoint_stats 的每项目 latest 接口子查询
--    ORDER BY updated_at DESC LIMIT 1：仅 (project_id) 前导索引，取最新需排序。
-- 3. teams.rs 删团队前置检查 SELECT COUNT(*) FROM projects WHERE team_id = ?：
--    无索引，全表扫。
CREATE INDEX IF NOT EXISTS idx_histories_created
    ON request_histories (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_endpoints_project_updated
    ON endpoints (project_id, updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_projects_team
    ON projects (team_id);
