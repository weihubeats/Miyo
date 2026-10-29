-- 场景的自定义脚本：脚本视图可编辑（语法高亮 CodeMirror），编辑后持久化。
-- NULL / 空 = 未自定义，脚本视图实时由手动步骤生成；非空 = 以脚本为准
--（复制 / CI 导出走该版本，可一键重置回生成版本）。手动步骤始终是执行真源。
ALTER TABLE scenarios ADD COLUMN script TEXT NULL;
