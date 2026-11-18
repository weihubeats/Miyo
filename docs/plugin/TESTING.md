# 测试报告（性能 / 安全 / 包体积）

> 基线：2.0 全阶段落地后（2026-11-19）。所有数字来自本地实测（macOS arm64 /
> Apple Silicon / Node 22 / JDK21 launcher）。

## 测试规模

| 套件 | 用例数 | 说明 |
|---|---|---|
| miyo-plugin | 46 | manifest 恶意输入 / 签名 roundtrip / 篡改 / 密钥轮换 / 依赖解析 / 权限目录 |
| miyo-storage | 44 | 状态机 / KV 隔离级联 / 日志滚动 / 版本历史滚动 / 迁移守卫 |
| miyo-tauri | 53 | **更新-回滚真实包 E2E** / 门禁分型 / 会话凭证 / 契约三向同步 |
| miyo-java-attach | 18+2 | 协议 mock / 真实 JVM e2e（发现→attach→调用→取消→重连→卸载） |
| miyo-sidecar | 9 | 帧编解码 / **6 真实进程 e2e**（握手 / 崩溃恢复 / 心跳判死 / 僵尸 / 上限） |
| frontend (vitest) | 1210 | RPC 加固 / SDK 会话 / 门禁 UI / i18n 键集 / 主题回归 |
| Java (mvn) | launcher 4 + agent 内嵌 | 参数解析 / Agent 协议冒烟 28 项（脚本） |

CI 覆盖：根 workspace（ubuntu）+ miyo-tauri + 前端 + **插件 crate 三平台**
（ubuntu / macos / windows）+ **Java Attach job**（agent mvn test + 真实 JVM e2e）
+ 签名工具 golden vector 自测。

## 安全测试落位清单（任务书第十一节）

| 项 | 落位 |
|---|---|
| 插件生命周期测试 | tauri install/update/rollback E2E + manager.spec |
| 插件并发加载测试 | manager.spec（boot 自动激活 / 多标签） |
| 插件异常隔离测试 | coreNoPlugins.spec（卸载后核心零影响） |
| Sidecar 崩溃恢复测试 | supervisor_e2e::crash_recovery（真实进程 exit(1) → 退避重启） |
| Sidecar 进程清理测试 | supervisor_e2e::stop_kills（kill(pid,0) 探活 + 无僵尸） |
| 插件签名测试 | package.rs roundtrip / 轮换 / foreign 降级 |
| 插件资源篡改测试 | 篡改包更新拒绝（tauri E2E）+ 检视流 sha256 失配 |
| Manifest 恶意输入测试 | manifest.rs（未知字段 / 穿越 / 非法权限 / 超大 / 坏版本） |
| 路径穿越测试 | 解包 ustar 裸头穿越 / plugin_read_asset / nav/entry |
| 权限绕过测试 | java_gate 分型（未安装/停用/缺权限）+ sessions 跨插件盗用拒绝 |
| RPC 重放测试 | bridge.spec 同 id 重放丢弃 |
| RPC 消息伪造测试 | 帧形状 / 源窗口 / nonce 不匹配 / 未知方法静默丢 |
| 插件文件读取越权测试 | plugin_read_asset 穿越拒绝 / symlink 拒绝 / 扩展名白名单 |
| 插件更新回滚测试 | tauri E2E（备份换回 + 快照消费制） |
| 依赖循环测试 | deps.rs cycle_detected |
| 插件禁用后权限撤销测试 | sessions revoke + gate PLUGIN_DISABLED |
| 插件卸载数据清理测试 | storage 级联 + versions 删除 + 会话吊销 |
| 多平台安装测试 | CI plugin-crates 三平台 job（包解析 / 解包 / 签名逻辑平台无关验证） |
| 真实第三方插件完整测试 | hello-world 官方签名 → 打包 → verify（SIGNING.md E2E）+ attach e2e 非 mock |

## 性能测试

| 指标 | 实测 | 口径 |
|---|---|---|
| 插件激活耗时 | 已有计量（详情页展示 activate ms） | 前端 manager 计时 |
| Sidecar 握手（echo-sidecar） | < 100ms（含进程 spawn） | supervisor e2e |
| Sidecar 崩溃恢复 | 200ms 退避 + ~150ms 重启握手（fast timing） | e2e |
| Java attach 全链路（真实 JVM） | 2.6s（发现→attach→调用→黑名单→取消→重连→卸载→kill） | attach_e2e |
| 文件清单校验（1 文件包） | 检视+解包 < 50ms | tauri E2E |
| 前端套件 | 149 文件 / ~40s | vitest |

未做：长稳（24h）/ 大包（64MiB 上限包）压测——列入后续。

## 包体积对比

| 项 | 数值 | 说明 |
|---|---|---|
| miyo-attach-launcher.jar | 19.3 KiB | Java Attach sidecar（一次性短进程） |
| miyo-java-agent.jar | 339.6 KiB | 注入目标 JVM 的 Agent（Java 8 + shaded ASM） |
| echo-sidecar（debug bin） | 1.3 MiB | Supervisor 测试目标 / 示例插件（release 更小，仅按需分发） |
| 前端 dist | ~3.0 MiB | 新增市场/详情 UI 后（2.0 前 2.9 MiB，增量 < 0.2 MiB） |
| 2.0 净增资源 | **< 0.6 MiB** | jar + 索引 + 文档不含安装包 |

结论：Java Attach 走 bundle.resources（未装插件不启动、不初始化）；Sidecar
Supervisor 为纯代码增量（无新二进制分发）；**安装包体积净增 < 1 MiB**。
真实 release 安装包对比需 release 构建产物（CI release.yml 可比），此处为
组件级实测。

## 已知限制（诚实清单）

1. macOS 无内存硬限（Darwin 不支持 RLIMIT_AS）——Sidecar 内存限额仅
   Linux / Windows 生效；
2. Windows attach / sidecar 未经真实环境实测（代码就绪 + CI 编译验证；
   windows-latest job 跑 miyo-sidecar e2e 可覆盖进程管理）；
3. `registerCommand` / `registerSettings` / 第三方动态路由 / 远端市场索引 /
   Landlock 收口：路线图；
4. miyo-tauri 的 `cfg(debug_assertions)` 依赖段对 cargo 无效（既有问题）——
   Java 能力始终编译进主二进制（本轮未裁剪，见计划审批记录）。
