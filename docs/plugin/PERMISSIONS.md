# 权限模型 2.0

> 唯一事实源：`crates/miyo-plugin/src/permission.rs`（封闭集合，manifest 声明
> 未知名即拒绝）。前端 `plugins/permissions.ts` 仅做展示镜像（i18n 键）。

## 目录（18 项）

| 权限 | 风险 | 后端强制点 |
|---|---|---|
| std.storage | Low | plugin_data_*（**会话凭证**） |
| std.log | Low | plugin_record_log（**会话凭证**） |
| std.notify | Low | SDK 门禁（Toast 为前端能力） |
| std.events | Low | SDK 门禁（事件总线为前端能力） |
| std.clipboard.write | Low | SDK 门禁（clipboard_write_text） |
| std.env.read | Medium | SDK 门禁（变量合并表） |
| std.network | Medium | **登记恒拒**——能力未实现，显式化风险 |
| std.files.read / std.files.write | Medium | **登记恒拒**（同上） |
| runtime.process.start | High | plugin_sidecar_start / plugin_sidecar_request |
| runtime.process.stop | Medium | plugin_sidecar_stop |
| java.process.discover | Medium | java_discover_local / probe_endpoint / test_connection / discover_processes / probe_pid |
| java.runtime.attach | High | java_attach_process / detach / unload_agent / sessions / agent_status / cancel / cancel_invocation |
| java.class.inspect | High | java_list_classes / list_methods / agent_classes / class_detail / methods / beans / spring_status |
| java.method.invoke | High | java_invoke / java_agent_invoke / 场景引擎 java_hook |
| java.debug | High | **聚合兼容项**：等价全部 java.* 子权限（0033 存量行展开） |
| database.connect / database.execute | High | **登记恒拒**（能力未实现） |

## 后端强制闭环（不依赖前端自律）

1. **java 族（28 命令）**：`commands/java_gate.rs::ensure_java_permission`——
   启用态 + `granted_permissions_json` 快照逐命令查（`granted_effective`
   展开 java.debug 聚合键）。收权（plugin_set_permissions）即时生效；
2. **std 族（plugin_data_* / record_log）**：`plugin_session_create` 签发
   一次性 token（内存表，12h TTL，每插件上限 16，跨插件盗用拒绝）→
   SDK / 管理界面逐调用携带 → 收权 / 停用 / 卸载 / error 态即吊销全部会话；
3. **runtime 族**：同 java 族口径（快照直查）；
4. **审计**：授予 / 收回 diff 写 `plugin_permission_audit`；高危动作（attach /
   unload / invoke / sidecar start-stop）写 audit 级插件日志。

## 生命周期语义

- 安装即授予 manifest 声明全集（确认页明示 + 风险着色 + High 单独确认）；
- 更新收敛为「旧批准 ∩ 新声明」（新权限需重新批准）；
- builtin 更新同样交集收敛；
- 停用 / 卸载 / 激活失败（error 态）→ 会话全量吊销；
- java.debug 聚合键：仅存量行兼容（0033 迁移已展开），新清单不应再声明。

## 「登记恒拒」的诚实口径

`std.network` / `std.files.*` / `database.*` 当前**没有宿主能力**：声明合法
（风险显式化、市场可展示），但所有关联命令路径不存在 → 实际能力 = 拒绝。
文档与 UI 不冒充可用（AGENTS.md「不要用 Mock 冒充真实功能」）。
