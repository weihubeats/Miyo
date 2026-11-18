# Sidecar Runtime 设计

> 实现：`crates/miyo-sidecar`（Supervisor / protocol / limits）
> + `crates/miyo-tauri/src/commands/sidecar.rs`（门禁编排）。
> 参考实现与测试目标：`crates/miyo-sidecar/src/bin/echo-sidecar.rs`（真实进程）。

## 定位

- **常驻型**插件 sidecar（UI+Sidecar 插件的后端进程）；
- 短命令型 sidecar（如 Java attach launcher）**不经过**本模块——一次性进程
  设计（spawn → 单命令 → 退出）已由 miyo-java-attach 承担。

## 协议（sidecar.protocol = 1）

stdio 传输：**4 字节小端长度前缀 + UTF-8 JSON 帧**，单帧上限 8 MiB。

| 方向 | 帧 | 说明 |
|---|---|---|
| 宿主 → sidecar | `hello` | 握手：一次性会话 token + 协议版本（**stdin 下发，不经 argv**） |
| sidecar → 宿主 | `hello_ack` | token **回显**（不匹配即断链）+ 协议版本 + sidecar_version |
| 宿主 → sidecar | `ping` | 心跳 |
| sidecar → 宿主 | `pong` | 心跳应答 |
| 宿主 → sidecar | `req` | 请求（id 关联 / method / params） |
| sidecar → 宿主 | `res` | 响应（ok / result / error{code,message}） |
| sidecar → 宿主 | `evt` | 事件推送 |
| sidecar → 宿主 | `log` | 日志（level / message → 宿主 plugin_logs） |

## Supervisor 职责与参数

| 能力 | 实现 | 参数 |
|---|---|---|
| 握手鉴权 | token stdin 下发 + 回显校验 + 协议一致 | 握手超时 15s |
| 心跳 | 周期 ping；距最后 pong 超限判死 | 10s / 30s |
| 崩溃恢复 | on-crash 指数退避重启（换代续监） | 1s→2s→4s→8s→16s |
| 防启动循环 | 1 小时窗口重启计数超限 → 放弃 + error 日志 | 5 次/小时 |
| 实例限额 | 总量 / 单插件幂等替换（旧实例先停） | 总 8 / 插件 1 |
| 请求转发 | id 关联 oneshot + 超时清理 | 30s |
| 日志采集 | log 帧 + stderr 行 → plugin_logs | 消息截断 2000 |
| 退出清理 | 进程组 kill + kill_on_drop + waitpid 回收 | ExitRequested 钩子 |
| 防僵尸 | wait 任务归数 + terminate 全挂起请求清空 | – |

## 进程安全（manifest.sidecar → spawn）

- **token 不经 argv**：`ps` 不可见（Java Attach 收口经验复用）；
- **最小环境变量**：env_clear + 白名单（PATH / HOME / TMPDIR / LANG / SYSTEMROOT）；
- IPC 仅 stdio（无网络端口、无命名管道）；
- 可执行文件必须在包内且过 files 清单校验；unix 安装 / 启动前兜底 chmod +x。

## 隔离等级矩阵（诚实边界）

> **启动子进程 ≠ 安全沙箱。** 下表如实标注每平台实际能力。

| 平台 | 实现 | 明确不提供 |
|---|---|---|
| Linux | setsid 进程组 + `RLIMIT_AS` 内存硬限 + `RLIMIT_NOFILE`(256) + 最小 env | seccomp / cgroup v2（需特权）；Landlock 收口为路线图 |
| macOS | setsid + `RLIMIT_NOFILE` + 最小 env。**内存硬限不可用**：Darwin 未实现 `setrlimit(RLIMIT_AS)`（恒 EINVAL，实测） | seatbelt profile（sandbox-exec 已废弃） |
| Windows | Job Object：KILL_ON_JOB_CLOSE + 进程内存上限 + TerminateJobObject | AppContainer |

结论：本模块提供**进程级隔离 + 资源限额 + 最小权限环境**；不可信代码的
强隔离仍需 OS 级机制（各平台沙箱），在文档与 UI 中不夸大。

## 命令面（后端强制）

| 命令 | 权限 |
|---|---|
| plugin_sidecar_start | runtime.process.start |
| plugin_sidecar_stop | runtime.process.stop |
| plugin_sidecar_request | runtime.process.start（方法名白名单字符校验） |
| plugin_sidecar_status | 宿主视图（不经 SDK 暴露） |

SDK 侧：`ctx.commands.invoke('plugin_sidecar_start')` → RPC host → 后端门禁
（granted 快照逐调用查）→ Supervisor。
