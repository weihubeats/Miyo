# 插件架构审计报告（插件系统 2.0 前置）

> 审计基线：main @ 5755abf（2026-11-17）+ 本地 Java Attach 2.0 未提交改动。
> 结论已全部核到代码调用链（非文档推断）。2.0 升级按本表逐项落地。

## 16 项能力审计

| # | 能力 | 升级前状态 | 2.0 落地结果 |
|---|---|---|---|
| 1 | Manifest 校验 | ✅ 严格（deny_unknown_fields / 路径 / 版本 / SDK） | ➕ files 全文件清单 / sidecar / platforms / ui-sidecar 开放 |
| 2 | 生命周期 | ⚠️ 三态状态机完整；本地包无 update、无回滚 | ✅ plugin_update_from_path / plugin_rollback / 版本历史 |
| 3 | 插件依赖 | ⚠️ 仅格式校验，无解析 | ✅ deps.rs：循环 / 缺失 / 版本 / 停用 / 不兼容，拓扑序 |
| 4 | 动态路由 | ✅ 内置插件动态注册 + 冲突防护 | 保留（第三方仍走 manifest.nav） |
| 5 | 插件 SDK | ⚠️ 10 成员 + 权限门禁；缺 registerCommand/Settings | ➕ 会话凭证 / runtime.* / java.debug 聚合展开；registerCommand/Settings 仍为路线图 |
| 6 | iframe 沙箱 | ✅ allow-scripts 无 same-origin + Blob URL | ➕ 入口读取收口 plugin_read_asset（受管目录 / 拒 symlink / 扩展名白名单） |
| 7 | RPC 协议 | ⚠️ 白名单 / 超时；**无**去重 / 并发 / 体积 / 深度 / 熔断 | ✅ 去重 / 并发 20 / 帧 1MiB / 深度 16 / 订阅 32 / 熔断 / nonce（v1 兼容可选） |
| 8 | 插件权限 | ❌ granted 快照无后端消费者；java 门禁不查权限 | ✅ 28 java 命令 + sidecar 命令按族查 granted；std 族会话凭证逐调用校验；18 项目录 |
| 9 | 插件存储 | ✅ plugin_data 行级隔离 + 级联清理 | ➕ 会话凭证强制（token ↔ plugin_id） |
| 10 | 插件日志 | ✅ 滚动 200 / 5 级别 / audit | ➕ sidecar 日志（log 帧 + stderr）统一入流 |
| 11 | 安装 / 卸载 | ✅ 解包防护完整；⚠️ 目录直装不复制 / read_text_file 任意路径读 | ✅ 原子安装（staging→rename）+ 收口命令 |
| 12 | 插件更新 | ⚠️ 仅 builtin 有 update | ✅ 本地包更新（快照 / 备份 / 原子换入 / 失败回位） |
| 13 | 完整性 | ❌ 仅 manifest 单文件；enable 不复核；前端零调用 | ✅ files 清单三环节复核（检视流式 / 解包落盘 / 启用）+ UI 展示与手动校验 |
| 14 | 签名 | ❌ 占位零值公钥，signed 恒 false；仅签 manifest 字节 | ✅ Ed25519 keyring（真实公钥 / 轮换）/ 规范化 payload（manifest+files）/ 信任三级 / 签名工具 / golden vector 跨语言互验 |
| 15 | 异常恢复 | ⚠️ error 态 + 恢复按钮 | ➕ sidecar 崩溃恢复（指数退避 + 防循环）+ 心跳判死 + 会话吊销 |
| 16 | 性能 | ⚠️ 激活计时 + 懒加载 chunk | ➕ sidecar 按需启动 / 状态展示 / 完整性复核结果落库 |

## Java Attach 2.0（本地改动，已作为基线提交）

**形态**：真实的独立进程 sidecar——

- `miyo-attach-launcher.jar`（19 KiB）：短生命周期 sidecar，Miyo 按需 spawn
  （tokio + kill_on_drop），单命令即退；attach 异常与 Miyo 主进程隔离；
- `miyo-java-agent.jar`（340 KiB，Java 8 字节码 + shaded ASM）：loadAgent 注入
  目标 JVM，内部 127.0.0.1 随机端口 TCP IPC（协议 v2、token 常数时间比较、
  连接数上限 8）；
- 会话安全：一次性 256-bit token 仅内存流转、expectedPid + vm_start_ms 身份
  绑定、infoFile 0600 不含 token、Rust 预检 + Agent RiskPolicy 终检（黑名单 +
  可赋值性闸）+ new 实例 consent。

**基线缺口（Phase 1 收口）**：门禁 14/14 对称、会话上限 8 / 心跳 30s /
infoFile 清扫 / token 改 stdin（ps 不可见）/ 场景路径黑名单预检 / 退出清理。
均已落地。
