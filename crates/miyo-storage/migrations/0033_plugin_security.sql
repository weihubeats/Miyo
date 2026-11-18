-- 0033：插件安全地基（manifest 2.0 / 权限细分 / 版本历史 / 权限审计）
--
-- 1. plugins 增列：全文件清单校验状态（files_verified / last_verified_at）——
--    manifest.files 声明即强制，安装 / 启用复核结果落库供详情页展示；
-- 2. 存量 granted_permissions_json 中的聚合权限 `java.debug` 替换为
--    四个 java.* 子权限（replace 幂等：无该键的行原样保留）；
-- 3. plugin_versions：版本历史（更新前快照，支持手动回滚）；
-- 4. plugin_permission_audit：权限授予 / 收回审计（不含 token / 会话）。

ALTER TABLE plugins ADD COLUMN files_verified INTEGER NOT NULL DEFAULT 0;
ALTER TABLE plugins ADD COLUMN last_verified_at TEXT;

UPDATE plugins
SET granted_permissions_json = replace(
    granted_permissions_json,
    '"java.debug"',
    '"java.process.discover","java.runtime.attach","java.class.inspect","java.method.invoke"'
);

CREATE TABLE plugin_versions (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id   TEXT NOT NULL,
    version     TEXT NOT NULL,
    manifest_json TEXT NOT NULL,
    integrity   TEXT,
    signed      INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_plugin_versions_plugin ON plugin_versions(plugin_id, id DESC);

CREATE TABLE plugin_permission_audit (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id   TEXT NOT NULL,
    action      TEXT NOT NULL,          -- grant / revoke
    permissions_json TEXT NOT NULL,     -- 本次动作涉及的权限列表
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_plugin_permission_audit_plugin
    ON plugin_permission_audit(plugin_id, id DESC);
