-- 场景生命周期（setup / teardown）与数据驱动数据集。
--
-- 三列均为**增量列 + 严格默认值**：旧版本 SELECT 显式列名，不受影响；
-- 回滚方案：SQLite ≥ 3.35 直接 `ALTER TABLE scenarios DROP COLUMN <col>`；
-- 列内容为纯 JSON 文本，回滚前可导出（SELECT setup_json ...）留档。
ALTER TABLE scenarios ADD COLUMN setup_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE scenarios ADD COLUMN teardown_json TEXT NOT NULL DEFAULT '{"enabled":true,"steps":[]}';
ALTER TABLE scenarios ADD COLUMN data_sets_json TEXT NOT NULL DEFAULT '[]';
