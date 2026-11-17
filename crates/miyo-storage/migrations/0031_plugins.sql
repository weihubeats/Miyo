-- 插件系统（Core + Plugin 架构）：
-- plugins      插件注册与生命周期状态（install/enabled/error），manifest 原文落库；
-- plugin_data  插件私有 KV 存储（按 plugin_id 强制隔离，插件之间不可互访）；
-- plugin_logs  插件统一日志（含 audit 审计条目，按插件滚动清理）。
--
-- 生命周期状态机（state 列，迁移由 repository 层校验）：
--   installed（已安装未启用）→ enabled（启用，前端激活后可用）→ error（激活失败，可恢复）
-- 卸载 = 删除行（builtin 插件文件仍在应用内，但不再注册任何 UI / 命令门禁放行）。

CREATE TABLE IF NOT EXISTS plugins (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL DEFAULT 'ui',
    version TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'installed',
    manifest_json TEXT NOT NULL,
    source TEXT NOT NULL DEFAULT 'local',
    integrity TEXT,
    signed INTEGER NOT NULL DEFAULT 0,
    granted_permissions_json TEXT NOT NULL DEFAULT '[]',
    install_path TEXT,
    error_json TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS plugin_data (
    plugin_id TEXT NOT NULL REFERENCES plugins(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    value_json TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin_id, key)
);

CREATE TABLE IF NOT EXISTS plugin_logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_id TEXT NOT NULL,
    level TEXT NOT NULL,
    message TEXT NOT NULL,
    context_json TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_plugin_logs_plugin ON plugin_logs(plugin_id, id DESC);
