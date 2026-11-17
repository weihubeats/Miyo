-- Java Debug 2.0：本地 JVM Attach 模式（零源码侵入）。
--
-- 设计：attach 会话复用 java_projects 行——历史（java_invocations.project_id
-- NOT NULL）、审计、重放链路全部零改动。区分字段：
--   mode          'runtime'（SDK 嵌入端点，v1 行为，存量行默认值）| 'attach'（动态注入）
--   attach_pid    目标进程 pid（attach 模式；runtime 模式 NULL）
--   vm_start_ms   目标 JVM 启动时刻（epoch ms）——进程重启识别（pid 复用时
--                 旧 vm_start 不匹配 → 更新本行，历史保持连续）
--
-- 存量行为：ALTER ADD COLUMN 带默认值会把 'runtime' 回填到全部存量行
--（无「恰好等于旧默认残留」问题——v1 数据本就只有 runtime 形态）。
-- runtime_endpoint 在 attach 模式下存 'attach://<pid>'（展示用占位，
-- 不参与连接；连接信息是运行时状态，不落库）。

ALTER TABLE java_projects ADD COLUMN mode TEXT NOT NULL DEFAULT 'runtime';
ALTER TABLE java_projects ADD COLUMN attach_pid INTEGER;
ALTER TABLE java_projects ADD COLUMN vm_start_ms INTEGER;

CREATE INDEX IF NOT EXISTS idx_java_projects_attach
    ON java_projects(mode, attach_pid);
