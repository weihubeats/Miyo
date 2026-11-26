-- 集成测试业务目录（多级文件夹）+ 场景管理扩展字段（2026-11）。
--
-- scenario_folders：项目级多级目录树（parent_id 自引用）。删除父级时
--   用递归 CTE 显式收集子树（SQLite 的 ON DELETE CASCADE 复用连接参数，
--   存储层不依赖 PRAGMA），外键仍声明为 SET NULL 作为兜底。
-- scenarios.folder_id：所属目录（NULL = 未分类）；旧数据零迁移。
-- scenarios.tags_json：标签数组 JSON（空数组 = 无标签），JSON 透传。
-- scenarios.favorite：收藏标记（列表置顶）。
CREATE TABLE IF NOT EXISTS scenario_folders (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    parent_id TEXT REFERENCES scenario_folders(id) ON DELETE SET NULL,
    sort_order INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scenario_folders_parent
    ON scenario_folders(project_id, parent_id, sort_order);

-- 同级目录禁止重名（根目录 parent_id 为 NULL，COALESCE 统一为空串参与唯一约束；
-- 大小写不敏感，与应用层预检口径一致）。
CREATE UNIQUE INDEX IF NOT EXISTS idx_scenario_folders_unique_name
    ON scenario_folders(project_id, COALESCE(parent_id, ''), name COLLATE NOCASE);

ALTER TABLE scenarios ADD COLUMN folder_id TEXT REFERENCES scenario_folders(id) ON DELETE SET NULL;
ALTER TABLE scenarios ADD COLUMN tags_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE scenarios ADD COLUMN favorite INTEGER NOT NULL DEFAULT 0;
-- 全量执行计数（运行历史会按场景修剪到最近 N 条，计数不能从 runs 聚合；
-- 每次落历史时增量更新，旧数据从 0 起算）。
ALTER TABLE scenarios ADD COLUMN run_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scenarios ADD COLUMN ok_run_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scenarios ADD COLUMN fail_run_count INTEGER NOT NULL DEFAULT 0;

CREATE INDEX IF NOT EXISTS idx_scenarios_folder ON scenarios(folder_id, sort_order);
-- 收藏置顶 + 更新时间排序（场景库列表的常用排序键）。
CREATE INDEX IF NOT EXISTS idx_scenarios_favorite ON scenarios(project_id, favorite DESC, updated_at DESC);