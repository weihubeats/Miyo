-- 测试用例文件夹（自由分组）：用例可归入命名文件夹（如「Dlocal 渠道」），
-- 空串 = 未分组。自由文本而非独立实体表——单层分组足够（渠道 → 场景），
-- 后续需要多级时可演进为路径（A/B）。旧数据经 DEFAULT '' 零迁移。
ALTER TABLE test_cases ADD COLUMN folder TEXT NOT NULL DEFAULT '';
