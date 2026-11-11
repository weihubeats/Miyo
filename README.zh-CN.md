# Miyo

[![Release](https://img.shields.io/github/v/release/weihubeats/Miyo?include_prereleases&sort=semver)](https://github.com/weihubeats/Miyo/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/weihubeats/Miyo/ci.yml?branch=main)](https://github.com/weihubeats/Miyo/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey)](https://github.com/weihubeats/Miyo/releases)

> 一个 ~10MB 的本地优先 API 客户端——调试 HTTP / WebSocket / SSE / GraphQL / gRPC，内置 Mock 服务、GUI 测试运行器和 CI 用的 CLI。无需账号，Apache-2.0 开源。

**下载**：[Releases](https://github.com/weihubeats/Miyo/releases) · **官网与文档**：[weihubeats.github.io/Miyo](https://weihubeats.github.io/Miyo/) · **语言**：[English](README.md) · 简体中文

<!-- 待系统代理回环修复后重录：![Miyo 快速上手演示](docs/imags/demo-debug.gif) -->

## 一张表看懂 Miyo

| | Miyo | Postman | Bruno | Insomnia |
| --- | --- | --- | --- | --- |
| 安装包 | **~10 MB** | ~310 MB | ~433 MB | ~200 MB |
| 账号 | **不需要** | 引导登录 | 不需要 | 引导登录 |
| 内置 Mock | **有** | 云端限额 | 无 | 有 |
| GUI 测试 + CI CLI | **有（JUnit 报告）** | 需外挂 Newman | bruno-cli | 无 |
| 压测 | **内置** | 付费档 | 无 | 无 |
| 开源协议 | **Apache-2.0** | 专有 | MIT | 专有 |

完整实测数据（冷启动、内存、应用壳）见[官网](https://weihubeats.github.io/Miyo/)。

## 核心能力

- **调试** — HTTP、WebSocket、SSE、GraphQL、gRPC；环境变量 `{{插值}}` + AES-256-GCM 加密；cURL 粘贴导入；多项目多标签、草稿独立
- **Mock 服务** — 端口自动探测、规则匹配、模板变量、延迟与故障注入：前后端不用互相等
- **测试 → CI** — GUI 里写断言和变量链，`miyo-cli` 无头跑集合（JUnit 报告 + 退出码语义），接口回归流水线直接红
- **压测** — 并发、QPS、P50/P90/P99 分位耗时，内置
- **本地优先安全** — 数据全在本地 SQLite，敏感值 AES-256-GCM 加密，密钥不出本机
- **AI Agent 集成** — 内置 MCP Server，在 Claude Code / Cursor 里一句话把 cURL 存成接口
- **团队同步** — 项目快照推 git / S3 / 数据库远端，敏感值自动脱敏
- **导入** — OpenAPI 3 / Swagger 2 / Postman v2.1 / cURL；删除撤销、快捷键帮助、深浅主题

## 为什么选择 Miyo？

### 🪶 10 MB，零运行时依赖

同类工具把整个 Chromium/Node.js 运行时打包进安装包，Miyo 用 **Rust + Tauri 2 + 系统 WebView**——不内置浏览器内核、无 Node 沙箱、无 JRE，装完即用。

| 维度 | Miyo | Postman | Bruno | Insomnia |
| --- | --- | --- | --- | --- |
| 安装包体积 | **~10 MB** | ~310 MB (Electron) | ~433 MB (Electron) | ~200 MB (Electron) |
| 首屏启动 | **< 1 秒** | 2–4 秒 | 2–5 秒 | 2–4 秒 |
| 运行时内存 | **~40 MB** | ~500 MB+ | ~300 MB+ | ~200 MB+ |
| 应用壳 | 系统 WebView（无内置 Chromium） | Chromium + Node.js | Chromium + Node.js | Chromium + Node.js |

同等工作负载下，安装包体积缩小 **20–40 倍**、启动快 **2–5 倍**、占用内存减少一个数量级。

### ⚡ 快，但功能不缺

Rust LTO 优化 + 单一进程模型 + SQLite 零拷贝本地存储——秒开、秒搜、秒发；请求 / Mock / 测试 / 压测全部内置，不依赖云端服务。

### 🔒 本地优先，数据在自己手里

数据只存在本机 `miyo.db`，环境变量值 **AES-256-GCM 加密**存储；一键备份为 JSON，随时可恢复。

## 功能

### 请求与编辑

- 8 种 HTTP 方法、6 种请求体（JSON / Form / x-www-form-urlencoded / Multipart / GraphQL / Text）
- Params / Headers / Body / Auth / Tests / Docs 分页编辑，未保存草稿自动标记
- cURL 一键粘贴导入，自动识别方法 / URL / Header / Body / Basic Auth（地址栏直接粘贴 cURL 也会自动解析回填当前接口）
- 环境变量 `{{name}}` 任意位置自动解析（环境 > 项目 优先级）

### 认证与安全

- API Key / Basic / Bearer / OAuth2（Authorization Code / Client Credentials / Password / Implicit 四模式）
- 令牌自动附加请求头，无需手写

### 响应体验

- 格式化 JSON 树 / Raw / 响应头 / 状态码 / 耗时 / 大小
- 流式下载保存到本地；请求历史支持关键字 + 状态筛选搜索，可重发、可删除

### Mock Server

- 本地 axum Mock（端口 4010 起自动探测），无需联网
- **Mock 规则**（方法 + 路径 + Header + Body 匹配）优先，接口「响应示例」兜底
- 模板变量：`{{params.id}}` `{{headers.X-Token}}` `{{mock.uuid|email|name|word|int}}`

### 自动化测试与压测

- JSON 测试脚本：`pre_request` 注入变量、`extract` 提取传递、`assertions` 断言
- 测试用例：多用例管理，列表按名称 / 路径 / Body 搜索，Drawer 内请求 Body 支持实时查找高亮
- 单接口 / 文件夹 / 全项目一键运行，结果与历史留存
- 压测：并发 × 总请求数，输出 QPS、平均耗时、P50/P90/P99、错误样例，chart.js 图表

### 导入导出与协作

- OpenAPI 3.x / Swagger 2.0 / Postman Collection v2.1 导入导出
- 单接口 / 全项目导出 Markdown 文档
- 客户端代码生成：cURL / JavaScript / Java / Go / Rust / Python / PHP（自动含变量替换与认证头，带语法高亮）

### 更多

- GraphQL 调试视图
- 实时调试：WebSocket 收发 / SSE 订阅（可弹出独立窗口）
- 环境导入导出（Miyo 原生 / Postman）、OpenAPI 3.1 导入
- Cookie 管理（查看/按域清理/单请求禁用）、Mock 热重载与故障注入
- 删除撤销、目录树多选批量操作、快捷键帮助（Ctrl+/）、设置页日志查看
- 备份（JSON）与恢复（ID 全量重映射，绝不覆盖现有数据）
- 数据目录可配置（设置页更改或 `MIYO_DATA_DIR` 覆盖，重启生效）
- 无头测试运行器（`miyo-cli`）：在 CI 运行 API 集合并输出 JUnit 报告

## AI Agent 集成

Miyo 内置 **Agent 控制面**（应用启动时自动拉起）：本机回环地址上的带令牌 HTTP API，
让 AI Agent（Claude / Cursor / 任意能执行命令的工具）直接把 cURL 命令保存为接口，无需人工粘贴。

完整指南（获取 `miyo-mcp`、各客户端配置、HTTP API 手册、排障）：**[docs/AGENT.md](docs/AGENT.md)**

### MCP Server（推荐）

> **前置**：MCP 配置需要 `miyo-mcp` 二进制。v0.0.10+ 安装包不在 PATH，必须填安装包内绝对路径（macOS 安装包：`/Applications/Miyo.app/Contents/MacOS/miyo-mcp`）。
> **v0.0.10 起安装包已内置**，无需额外安装；更早版本请先按 [docs/AGENT.md](docs/AGENT.md) 构建。

Claude Code 等支持 MCP 的客户端，在项目 `.mcp.json` 中配置一次：

```json
{ "mcpServers": { "miyo": { "command": "/Applications/Miyo.app/Contents/MacOS/miyo-mcp" } } }
```

> 仅当二进制已加入 PATH 时才可写裸 `miyo-mcp`（如 Linux .deb 安装、或 `cargo build --release -p miyo-mcp` 后放入 `/usr/local/bin`）。

配好后直接让 AI 把接口存进 Miyo（贴 cURL 或贴代码都行）：

```
把这个接口保存到 Miyo：
@PostMapping("/orders")
public Result<Long> createOrder(@RequestBody CreateOrderReq req) { ... }
```

之后对话里说「把这个 curl 存到 Miyo」即可。共提供 **31 个工具**，覆盖接口
导入 / 查询 / 执行、场景编排与运行、测试用例与结果分析——完整清单见
[docs/AGENT.md](docs/AGENT.md)（或在对话里让 AI 调 `agent_info` 自举）。代表示例：

| 工具 | 说明 |
| --- | --- |
| `save_curl` | 解析 cURL 并保存为接口（URL 拆 base_url + 路径 + query） |
| `run_scenario` | 运行集成测试场景：步骤串行执行、链路变量自动传递、返回分步结果 |
| `get_run_result` | 轮询异步场景运行结果（配合 `run_scenario` 的 `wait=false`；可用 `cancel_run` 取消） |
| `agent_info` | 控制面地址与令牌文件位置 |

### 直接 HTTP

任何能发 HTTP 的工具也可以直接调用控制面：

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/agent/curl` | `{command, projectId?, name?, folderId?}` → endpointId |
| GET | `/agent/projects` | 项目列表 |
| GET | `/agent/endpoints/:projectId` | 接口列表 |
| GET | `/agent/health` | 存活探针 |
| POST | `/agent/scenarios/:id/run` | 运行场景（`wait=false` 返回 runId） |
| GET | `/agent/runs/:id` | 运行结果 / 状态 |
| POST | `/agent/runs/:id/cancel` | 取消运行中的场景 |

上表是入门子集——控制面还覆盖场景 / 测试用例 CRUD 与环境查询，完整路由见
`docs/AGENT.md` 或 `agent_info` 工具（内嵌完整路由清单）。
鉴权：请求头 `Authorization: Bearer <token>` 或 `X-Agent-Token`；
token 位于数据目录 `agent-token` 文件（0600），应用内「Agent 状态」或
`agent_status` 命令可查路径。

> **两种接入形态别混淆**（初次接入最容易踩）：**MCP 是独立的 stdio 进程**，
> 不监听任何端口——`GET http://127.0.0.1:<port>/mcp` 返回 404 是预期行为；
> HTTP 控制面是另一套东西，路由全部在 `/agent/*` 下。
>
> 端口从 `4110` 起自动探测，**实际端口以数据目录 `agent-port` 文件为准**
> （纯数字无换行），例如 macOS：`cat "$HOME/Library/Application Support/Miyo/agent-port"`。
> 开发版（`tauri dev`）使用 `Miyo-dev` 目录，dev 与正式版各有一套
> `agent-port` / `agent-token`。

安全设计：只绑定 `127.0.0.1`（Host 须为 127.0.0.1 / localhost / ::1）；
全部路由要求 Bearer 令牌。写操作覆盖场景与测试用例的增删改及执行——每个
工具可在 Miyo 设置 → MCP 服务中单独停用；运行历史与 `get_environment`
返回中的敏感变量已脱敏（`__MIYO_REDACTED__`）。不覆盖已有 `base_url`
配置（冲突时返回 warning）。

## 下载与安装

在 [Releases](https://github.com/weihubeats/Miyo/releases) 下载对应平台安装包：

| 平台 | 安装包 |
| --- | --- |
| Windows | `Miyo_*-x64-setup.exe`（NSIS 安装包，SmartScreen 提示时选「更多信息 → 仍要运行」） |
| macOS | `Miyo_*-aarch64.dmg`（Apple Silicon）/ `Miyo_*-x64.dmg`（Intel） |
| Linux | `.deb` / `.rpm` / `.AppImage` |

> **macOS 首次打开提示「无法验证开发者」？** 应用仅 ad-hoc 签名、未做 Apple 公证（未购买开发者证书），
> 属正常现象——右键应用 →「打开」即可。若旧版本安装器提示「已损坏，无法打开」，
> 在终端执行一次 `xattr -cr /Applications/Miyo.app` 解除隔离后重试。详见[使用手册](docs/USER_GUIDE.md#12-macos)。

安装后可在应用内「关于 → Check for Updates」检查并一键升级新版本（v0.0.3 起支持自动更新）。

## 从源码构建（开发者）

前置：Rust 工具链 + Node 22。

```bash
cargo build --workspace        # 构建全部后端 crate
cargo test --workspace         # 运行全部测试
npm --prefix frontend install
npm --prefix frontend run tauri dev     # 开发模式（Vite HMR）
scripts/package-tauri.sh                # 一键打包分发包
```

> 架构原理、Crate 划分、IPC 与数据流等开发细节见 **[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)**（Tauri 2 + Vue 3 三层架构，含架构图）。

## CLI 测试运行器

不打开界面，在终端 / CI 里运行集合：`miyo-cli run ./备份.json --env 生产环境 --junit report.xml`。详见 [docs/CLI.md](docs/CLI.md)。

## 文档

**官网**：[https://weihubeats.github.io/Miyo/zh/](https://weihubeats.github.io/Miyo/zh/) —— 用户手册、迁移手册、Agent 集成与更新日志在线阅读。

| 文档 | 说明 |
| --- | --- |
| [docs/README.md](docs/README.md) | 文档索引（用户 / 开发者 / 运维分流） |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 架构总览（Tauri 2 + Vue 3，含架构图与界面布局图） |
| [docs/USER_GUIDE.md](docs/USER_GUIDE.md) | 用户手册 |
| [docs/AGENT.md](docs/AGENT.md) | AI Agent 集成（MCP / HTTP 控制面） |
| [docs/MIGRATION.md](docs/MIGRATION.md) | 从 Apifox / Postman 迁移 |
| [CHANGELOG.md](CHANGELOG.md) | 版本变化速览 |
| [docs/SPEC.md](docs/SPEC.md) | 历史规范（已冻结，仅追溯用） |
| [docs/SMOKE_TEST.md](docs/SMOKE_TEST.md) | 手动验收清单 |
| [docs/DEPLOY.md](docs/DEPLOY.md) | 发布与部署 |
| [docs/MILESTONES.md](docs/MILESTONES.md) | 里程碑总览 |
| [docs/PROGRESS.md](docs/PROGRESS.md) | 开发进度记录 |
| [docs/en/README.md](docs/en/README.md) | English docs index |

## License

[Apache-2.0](LICENSE)