# 插件生命周期规范（12 阶段对照表）

> 唯一事实源是代码（命令层 `crates/miyo-tauri/src/commands/plugins.rs`、
> 纯逻辑 `crates/miyo-plugin`、状态机 `crates/miyo-storage/src/repository/plugins.rs`）；
> 本文把散落在各文档里的阶段定义收敛为单篇对照表。改动生命周期须同步本表。

## 状态机

```
            install                enable                 error
 （无行） ─────────→ installed ─────────→ enabled ──┬──────→ error
                        ↑  │                ↑   │      │ report_error
            uninstall   │  │ disable        │   │      │
                        └──┴────────────────┘   └──────┘
                                     （error → enable / uninstall 亦可）
```

- 状态常量：`STATE_INSTALLED` / `STATE_ENABLED` / `STATE_ERROR`
  （repository/plugins.rs:15-17；**没有 disabled 态——停用即回 installed**）；
- 跃迁校验：`set_plugin_state`（同文件 176-214），非法跃迁与未知状态串拒绝；
- 卸载 = 删行（plugin_data 经 FK 级联；版本历史显式删除；审计与日志**有意保留**）。

## 12 阶段对照

| # | 阶段 | 入口 | 校验 / 动作 | 失败处理 | 审计 | 代码锚点 |
|---|---|---|---|---|---|---|
| 1 | Discover | 市场页签（编译期嵌入 `docs/market/index.json` + 内置目录合并） | — | 无数据不渲染 | — | `plugin_market_index` / `plugin_list_catalog` |
| 2 | Install（获取） | 本地 .miyo / 远程 URL / 内置 | https-only 重定向 ≤5 跳、≤64MiB、URL 脱敏 | 下载失败结构化错误 | `inspect remote url=…` | `plugin_inspect_from_url` / `download_package` |
| 3 | Validate（检视） | 安装确认弹窗前 | Manifest 严格解析（deny_unknown_fields）+ integrity + Ed25519 验签 + files 流式预验 + `ensure_compatible` + `ensure_sdk_supported` + 依赖检查 | 逐项拒绝（未签包需显式信任、High 权限单独勾选） | `install <source> v.. integrity=.. signed=..` | `inspect_package_bytes` / `check_dependencies` |
| 4 | Install（落盘） | 确认后 | staging 解包（拒穿越/symlink）→ 全文件复核 → 原子 rename → 落库 `STATE_INSTALLED`（granted = manifest 全集） | 失败清 staging | 同上 | `install_package_bytes` / `install_final` |
| 5 | Enable | 用户 / 依赖拓扑 | 兼容 + SDK + 依赖 DFS（循环/缺失/版本/停用/不兼容）+ 全文件完整性复核 | 拒绝并给出依赖链 | `enable v..` | `plugin_set_enabled` / `deps::check_enable` |
| 6 | Initialize（激活） | enable 后前端 | 会话凭证签发 + SDK ctx 构造 + nav/route 注册 + 插件 activate() | 激活失败 → unregisterAllUi + `STATE_ERROR` | `report_error`（截断 2000） | `manager.activate` / `plugin_session_create` |
| 7 | Activate / Execute | 运行期 | RPC 白名单 + 权限逐调用（java 族走 `java_gate`，std 族走会话凭证） | 熔断（30s 窗口 50 错→开 60s）/ 并发上限 / 超时 15s | 命令级 audit | `bridge.ts` / `java_gate.rs` |
| 8 | Deactivate | disable / uninstall 前置 | nav/route 注销 + 事件解绑 + store dispose | 注册表兜底清空 | — | `manager.deactivate` / `unregisterAllUi` |
| 9 | Disable | 用户 | **先停 sidecar** → `STATE_INSTALLED` → 会话吊销 | sidecar 停止失败不阻断（静默日志） | `disable` | `plugin_set_enabled` / `stop_sidecar_quiet` |
| 10 | Uninstall | 用户（确认弹窗，可勾选「保留插件数据」） | 停 sidecar → 删受管安装目录 → 删行（级联 plugin_data）→ 删版本历史 → 吊销会话；keepData=false 时深度清理（plugin-data 目录 / java 项目与历史 / attach 会话 / 组件 jar） | 目录越界拒绝（仅删 plugins_root 内） | `uninstall v..` | `plugin_uninstall` / `cleanup_plugin_data` |
| 11 | Upgrade | 用户 / 市场 | 检视 → 快照入 `plugin_versions`（滚动 10 条）→ 停 sidecar → 原子换入（旧目录 → `.rollback-<id>`）→ granted 收敛（旧批准 ∩ 新声明）→ 会话吊销 | DB 失败文件层回位；同版本拒绝 | `update …` | `update_impl` |
| 12 | Rollback | 用户 | 取最新快照 + `.rollback-<id>` 原子换回 → granted 收敛 → files 复核落库 | 快照消费即删（防重复回滚）；builtin 拒绝 | `rollback …` | `rollback_impl` |

## 崩溃恢复（sidecar 形态专属）

```
sidecar 退出 → 监督任务指数退避（1s→16s，1h 窗口 >5 次放弃）
  → RestartGate（换代前复查插件启用态，DB 失败按禁用 fail-closed）
  → 停用 / 卸载后的重启链就此终止
```
锚点：`crates/miyo-sidecar/src/supervisor.rs:482-584`。

## 数据隔离

| 面 | 位置 | 生命周期 |
|---|---|---|
| KV 配置 | DB `plugin_data`（FK ON DELETE CASCADE） | 卸载级联消失 |
| 文件数据 | `{data_dir}/plugin-data/<id>/{data,cache,tmp}`（`plugin_dir_get` 懒创建） | 卸载 keepData=false 删；true 保留 |
| 插件日志 | DB `plugin_logs`（滚动 200 条/插件） | 卸载**保留**（审计口径） |
| 临时 | `<id>/tmp` | 卸载恒删 |
| Java 专项 | java_projects / java_invocations 表、`data_dir/java` 组件、attach 会话 | 卸载 keepData=false 清理；true 保留 |
| 前端 localStorage | manifest `storage_keys` 声明的前缀 | 卸载 keepData=false 时按声明清除 |
