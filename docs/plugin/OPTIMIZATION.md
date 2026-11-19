# 插件系统深度审查与优化报告（2026-11-20）

> 基线：插件系统 2.0（见 [AUDIT.md](./AUDIT.md)）之上的增量审查。
> 原则：先梳理实证、不做无意义重写、渐进式加固、保持 Java 插件与既有包格式兼容。
> 本次改动 ±313 行，全部为定向修复 + 测试，未重写任何模块。

## 一、审查范围（源码级，逐文件核到调用链）

| 层 | 文件 | 结论 |
|---|---|---|
| 领域层 | `crates/miyo-plugin`（manifest / package / permission / compat / deps / builtin） | ✅ 完善保留：清单 `deny_unknown_fields` + 路径/版本白名单；tar.gz 解包拒穿越/符号链接/非常规条目 + 体积三重上限；files 全文件清单三环节复核；Ed25519 keyring 签名 + 跨语言 golden vector；依赖 DFS（循环/缺失/版本/停用/不兼容） |
| 编排层 | `crates/miyo-tauri/commands/plugins.rs`、`sidecar.rs`、`java_gate.rs`、`sessions.rs` | ✅ 保留：安装/更新/回滚原子管道、权限快照闭环、会话凭证、28 java 命令 + sidecar 命令门禁。⚠️ 发现 4 处缺口（见三） |
| 进程层 | `crates/miyo-sidecar`（supervisor / protocol / limits） | ✅ 保留：stdin 握手 token、心跳判死、指数退避防循环、进程组/Job Object 清理。⚠️ 发现 1 处缺口 |
| 前端 | `frontend/src/plugins`（manager / sdk / bridge / protocol / iframe host / permissions / events）、`domains/plugins` | ✅ 保留：懒加载零字节、opaque-origin iframe 沙箱、RPC 去重/并发/熔断/深度/帧限、dispose 全量回收。⚠️ 发现 1 处缺口 |
| DX | `docs/plugin/*`（9 篇）、`examples/plugins/*`、`scripts/pack_plugin.sh`、`scripts/sign_plugin.mjs` | ✅ 保留：模板/脚手架（两文件最小工程 + sidecar 参考实现）/签名工具/调试目录直装通道齐备 |

## 二、架构总览（现状 + 本次加固点 ⭐）

```text
┌─ 插件生命周期（全链路均在 Rust 后端强制校验） ─────────────────────────────┐
│                                                                           │
│  发现(市场/本地/内置) → 下载 ⭐https-only重定向 → 检视(签名/完整性/files流式预验)
│    → 安装(staging解包+原子rename+权限快照) → 初始化(前端activate+会话签发)
│    → 启用(兼容性+依赖+全文件复核) → 运行(RPC/门禁逐调用)
│    → 禁用 ⭐即停sidecar+吊销会话 → 更新(快照/备份/原子换入 ⭐先停sidecar)
│    → 回滚(备份原样换回 ⭐先停sidecar) → 卸载 ⭐先停sidecar→目录→DB级联
│                                                                           │
│  崩溃恢复：sidecar 退出 → 监督任务指数退避 → ⭐重启门禁(复查插件启用态,     │
│           DB 失败 fail-closed) → 停用/卸载后的重启链就此终止               │
└───────────────────────────────────────────────────────────────────────────┘

┌─ 三层插件形态与安全边界（诚实边界，iframe ≠ OS 沙箱） ────────────────────┐
│ UI 插件      sandbox="allow-scripts"（opaque origin，无 DOM/存储/父窗口） │
│             └ 唯一通道 postMessage：源窗口===宿主iframe + 方法白名单       │
│               + 参数形状校验 + 帧限/去重(防重放)/并发上限/熔断 + nonce(v2) │
│ 后端插件     SDK context：命令→权限映射白名单（未登记=拒绝）               │
│             └ granted 快照（后端插件门禁逐命令二次校验，前端只是第一道闸）  │
│ Sidecar 插件 独立进程：stdin 握手一次性 token（ps 不可见）+ 心跳判死        │
│             └ 内存限额/进程组 kill/Job Object + 总实例上限 + ⭐启用态重启门禁│
└───────────────────────────────────────────────────────────────────────────┘

┌─ 权限模型（最小权限 + 后端强制） ─────────────────────────────────────────┐
│ 封闭目录 18 项（miyo-plugin::permission 唯一事实源；未知名安装即拒）        │
│ granted ⊆ manifest.permissions（启用收敛，收权即吊销全部会话+审计落库）     │
│ java.debug 聚合键向后兼容展开（0033 存量行）                                │
│ High 风险 ⭐安装/更新确认页单独勾选（补齐 PERMISSIONS.md 承诺）              │
│ 恒拒权限（database.*/std.network/std.files.*）登记显式化风险                │
└───────────────────────────────────────────────────────────────────────────┘
```

## 三、发现的问题与修复情况

| # | 级别 | 问题 | 根因（核到代码） | 修复 | 测试 |
|---|---|---|---|---|---|
| F1 | 高·生命周期 | 停用/卸载不停止 sidecar 进程——进程与心跳任务继续占用资源，验收「禁用后不占资源」对 ui-sidecar 形态不成立 | `plugin_set_enabled(false)` / `plugin_uninstall` 只改状态与落库，从未调 `Supervisor::stop`；前端 `manager.disable/uninstall` 也只回收 SDK 会话 | 后端 `stop_sidecar_quiet`（状态检查+静默停止）接入 disable / uninstall；停用前停进程（卸载先停再删目录——删目录后重启连二进制都找不到） | e2e `stop_kills_process_group_without_zombie`（既有）+ 门禁用例联动 |
| F2 | 高·隔离 | 崩溃自动重启不复查插件启用态——停用插件在崩溃窗口内被监督任务「复活」 | `spawn_supervision` 退避后直接 `spawn_session(spec)`，无任何生命周期查询 | `Supervisor` 新增 `RestartGate`（OnceLock 注入）；`build_supervisor` 接线 `repo::is_plugin_enabled`（DB 失败按禁用 fail-closed）；退避后、换代前复查，false 即终止重启链并移除会话 | e2e 新增 `restart_gate_blocks_respawn_of_disabled_plugin`（翻转开关：true 重启→false 崩溃→会话移除+拦截日志+无新进程） |
| F3 | 中·下载源 | https 入口可经重定向降级 http / 无限跳转下载（reqwest 默认跟随任意 scheme，≤10 跳） | `download_package` 用默认 redirect policy，`validate_download_url` 只管首跳 | `https_redirect_policy`：仅跟随 https 目标 + ≤5 跳；`redirect_decision` 独立成纯函数单测 | 新增 `redirect_policy_https_only_with_hop_cap` |
| F4 | 中·权限确认 | 文档承诺「High 风险单独确认」（PERMISSIONS.md / permission.rs docstring）但安装确认页只做风险着色展示，未单独勾选 | 实现与文档漂移 | 确认弹窗新增 High 勾选行（声明 High 即必选，安装/更新/远程三路共用，每次检视重置）；`highRiskPermissions` 目录派生；permission.rs docstring 同步为「安装/更新确认页」 | 前端新增 `permissions.spec.ts`（5 用例）；i18n zh/en 1:1 经 locale.spec 验证 |
| F5 | 低·健壮性 | `plugin_report_error` 消息无长度上限——异常插件可写 MB 级 error_json 入库 | 命令层直传，存储层 set_plugin_state 不限长 | 命令层截断 2000 字符（与插件日志口径一致） | 既有日志截断链路回归 |
| F6 | 低·竞态 | 更新收尾 `cleanup_stale_staging` 删全部 `.staging-*`——并发安装的在途 staging 被误删（安装报「目录不存在」） | 收尾清理无年龄判定 | 仅删 mtime ≥ 1h 的孤儿；元数据不可读保守跳过；核心函数注入 max_age 可测 | 新增 `cleanup_stale_staging_respects_age` |
| F7 | 低·API 卫生 | `plugin_logs` limit 未封顶——任意 i64 直达 SQL LIMIT | 命令层透传 | `clamp(1, 1000)`（存储滚动上限 200，放大无意义） | 编译期 + 既有日志链路回归 |

**审查中确认已完善、未重复实现的能力**（防 regress 的回归锚点）：tar 穿越条目拒绝、符号链接三处拒绝（条目/清单复核/资产读取）、重复路径与清单外文件拒绝、签名 payload 规范化 + keyring 轮换、未签名显式信任、staging 原子换入 + `.rollback-*` 备份回位、更新后 granted 收敛 + 会话吊销、RPC 重放去重/熔断/深度限制、token 回显握手、心跳判死、退出清理（main.rs ExitRequested → shutdown_all）、插件存储行级隔离 + 键白名单 + 64KiB 单值上限、日志 8KiB + 滚动 200、市场两步制安装（检视缓存防 TOCTOU）。

## 四、修改文件清单（本次）

**后端**
- `crates/miyo-sidecar/src/supervisor.rs`：+30 —— `RestartGate` 类型 / `set_restart_gate` / 监督任务换代前门禁复查
- `crates/miyo-sidecar/src/lib.rs`：+2 —— 导出 `RestartGate`
- `crates/miyo-tauri/src/commands/sidecar.rs`：+13/-1 —— `build_supervisor` 接线启用态门禁（fail-closed）
- `crates/miyo-tauri/src/commands/plugins.rs`：+140 —— disable/uninstall 停 sidecar、`stop_sidecar_quiet`、update/rollback 停 sidecar、https-only 重定向、report_error 截断、logs clamp、staging 按龄清理 + 3 个单测
- `crates/miyo-plugin/src/permission.rs`：docstring 措辞对齐实现

**前端**
- `frontend/src/plugins/permissions.ts`：+8 —— `highRiskPermissions`
- `frontend/src/plugins/permissions.spec.ts`：新增（5 用例）
- `frontend/src/domains/plugins/views/PluginsView.vue`：+28 —— High 单独确认（状态/守卫/模板/样式）
- `frontend/src/i18n/zh.ts`、`en.ts`：+2 —— `plugins.confirmHighRisk`（含 `{count}` 插值）

**测试**
- `crates/miyo-sidecar/tests/supervisor_e2e.rs`：+86 —— 崩溃重启门禁用例

（工作区另有他人 WIP：`integration/*`、`EndpointEditor.vue`、i18n integration.* 键——非本次改动。）

## 五、验收标准对照

| 验收项 | 结果 |
|---|---|
| 1. 架构图 | 本文第二节 |
| 2. 修改文件清单 | 本文第四节 |
| 3. 问题及修复 | 本文第三节（7 项全修复） |
| 4. 插件安全测试 | 既有 46 项（miyo-plugin：穿越/符号链接/篡改/签名轮换/golden vector…）+ 新增重定向策略/staging 年龄单测 |
| 5. 安装、更新、回滚测试 | 既有 E2E `install_update_rollback_tamper_full_cycle` 全绿（含篡改包拒绝 + 快照消费 + 备份换回逐字节比对），本次签名变更后回归通过 |
| 6. 插件崩溃隔离测试 | e2e 7/7：崩溃恢复/心跳判死/防僵尸/实例上限 + **新增停用门禁隔离用例** |
| 7. Java 插件不受影响 | `miyo-plugin` 46/46（含内置清单自洽 + java.* 子权限拆分断言）、miyo-tauri 全量 58+（java 门禁/Attach 链路）、前端 `javaPlugin.spec.ts` 全绿；manifest/包格式/权限目录零破坏性变更 |
| 8. 禁用后不占资源 | F1/F2 修复：停用即停进程 + 会话吊销 + 重启链终止（e2e 断言「无新进程复活」） |

## 六、验证记录

- `cargo fmt --all --check` ✅；`cargo clippy --all-targets -D warnings`（根工作区 + miyo-tauri）✅
- `cargo test --workspace`（根）✅ 全绿；`cargo test`（miyo-tauri）✅ 58+3 全绿
- 前端 `eslint` ✅；`vitest run` 1241/1242（唯一失败为 EndpointEditor.splitterStyle.spec，系他人 WIP 改动 `EndpointEditor.vue` 所致，与本系统无关）；`npm run build` ✅
- 本机 vitest 需 Node ≥22.23（jsdom 依赖的 undici 8 在 22.14 fork 崩溃，CI 的 node 22 挡位不受影响）

## 七、后续路线图（不在本次范围）

- RPC nonce 升级为 v2 SDK 强制回传（协议已就绪：桥端校验逻辑 + 文档均已落地）
- 事件总线 sidecar `evt` 帧经 tauri emit 转发前端（sink 已留桩）
- 市场索引签名（index.json 级信任链；当前逐包签名已覆盖包体）
- `std.network` 代理通道开放（权限已登记恒拒，能力待实现）
