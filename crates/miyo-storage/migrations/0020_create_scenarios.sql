-- 集成测试场景（接口链路）：项目级实体，步骤内嵌 JSON。
--
-- 步骤为「引用式」：只存 endpoint_id + 步骤级提取/断言，URL / 头 / 体
-- 用接口当前配置（接口更新后场景自动跟随）；extract / assertions 与接口
-- tests 内对应字段同构（miyo-test ExtractSpec / AssertionSpec 数组），
-- JSON 透传、运行时解析，schema 演进无需迁移。
CREATE TABLE IF NOT EXISTS scenarios (
    id TEXT PRIMARY KEY,
    project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    sort_order INTEGER NOT NULL DEFAULT 0,
    steps_json TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_scenarios_project ON scenarios(project_id, sort_order);
