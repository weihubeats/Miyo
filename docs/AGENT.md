# AI Agent 集成指南

**语言 / Language**：[简体中文](AGENT.md) · [English](en/AGENT.md)

Miyo 内置 **Agent 控制面**：应用启动时自动拉起一个本机 HTTP 服务（仅 `127.0.0.1`），
AI Agent 可以直接把 cURL 命令保存为 Miyo 接口，也可以查询项目与接口列表。

两种接入方式：

| 方式 | 适用 | 前置 |
| --- | --- | --- |
| MCP Server（推荐） | Claude Code、Cursor 等支持 MCP 的客户端 | 额外获取 `miyo-mcp` 二进制 |
| 直接 HTTP | 任意能发 HTTP 请求 / 执行命令的 Agent | 无 |

---

## 1. 工作原理

```
AI Agent ──(stdio MCP)──→ miyo-mcp ──┐
                                        ├──→ Agent 控制面 (127.0.0.1:4110~4129)
AI Agent ──(HTTP + Bearer token)────────┘         │
                                             SQLite 落库 → UI 实时刷新
```

- 控制面随 **Miyo 桌面应用启动自动拉起**，无需手动开启；
- 端口从 `4110` 起自动探测（避开 Mock 的 4010 段）；
- 所有请求需携带令牌：`Authorization: Bearer <token>` 或 `X-Agent-Token`。

## 2. 找到你的 Token

Token 是控制面的访问凭证，首次启动应用时自动生成：

- 文件位置：`{数据目录}/agent-token`（权限 0600）
- 数据目录：
  - Windows：`%APPDATA%\Miyo\agent-token`
  - Linux：`~/.local/share/Miyo/agent-token`
- 应用内也可确认：顶栏「设置」旁无入口时，可在 DevTools Console 执行
  `await __TAURI_INTERNALS__.invoke('plugin:miyo|agent_status')` 查看 `tokenPath`。

> 开发构建（tauri dev）使用 `Miyo-dev` 目录，与正式版隔离。

## 3. 方式一：MCP Server

### 3.1 获取 miyo-mcp

**v0.0.10 起，安装包已内置 `miyo-mcp`**，按平台取对应路径即可：

| 平台 / 安装方式 | 路径 |
| --- | --- |
| macOS（/Applications 安装） | `/Applications/Miyo.app/Contents/MacOS/miyo-mcp` |
| Windows（NSIS 默认目录） | `C:\Program Files\Miyo\miyo-mcp.exe` |
| Linux（.deb） | `/usr/bin/miyo-mcp`（已在 PATH，配置直接写 `miyo-mcp`） |
| Linux（.AppImage） | 挂载镜像内的 `miyo-mcp` |
| 开发模式（tauri dev） | 仓库内 `frontend/src-tauri/binaries/miyo-mcp-<三元组>` |

<details>
<summary>旧版本（&lt; v0.0.10）或从源码构建</summary>

```bash
git clone https://github.com/weihubeats/Miyo.git
cd Miyo && cargo build --release -p miyo-mcp
# 产物：target/release/miyo-mcp
sudo cp target/release/miyo-mcp /usr/local/bin/   # 可选：放进 PATH
```
</details>

### 3.2 配置客户端

v0.0.10+ 安装包的二进制**不在 PATH**，推荐直接填 §3.1 表中绝对路径（macOS 安装包常见坑：裸 `miyo-mcp` 报 `spawn ... ENOENT`）。仅在已加入 PATH 时才能写裸命令。

**Claude Code** — 项目根目录 `.mcp.json`：

```json
{
  "mcpServers": {
    "miyo": { "command": "/Applications/Miyo.app/Contents/MacOS/miyo-mcp" }
  }
}
```

开发模式（tauri dev）或已在 PATH，则可写：

```json
{ "command": "miyo-mcp" }
```

**Cursor** — Settings → MCP → Add Server，Command 填绝对路径 `/Applications/Miyo.app/Contents/MacOS/miyo-mcp`（macOS）或对应平台路径。

配置后重启客户端 / 重载会话，工具列表出现 7 个工具即成功：

| 工具 | 说明 |
| --- | --- |
| `save_curl` | 解析 cURL 并保存为接口（返回 endpointId） |
| `list_projects` | 项目列表 |
| `list_endpoints` | 项目下的接口列表（需 projectId） |
| `list_scenarios` | 集成测试场景列表（projectId 可选） |
| `get_scenario` | 场景详情：步骤 + 接口当前配置 + 链路变量 + 项目变量（AI 写脚本的完整上下文） |
| `save_scenario_script` | 写入 / 清除场景自定义脚本（空串 = 清除） |
| `agent_info` | 控制面地址与令牌路径 |

之后对话里说「把这个 curl 存到 Miyo」即可；导入成功桌面端侧栏自动刷新并弹提示。
场景脚本协作：让 AI 先 `get_scenario` 拿上下文（每步接口配置 + 链路变量 +
base_url），再 `save_scenario_script` 写回——脚本在桌面端「场景 → 脚本」页签
可见、可继续手改；应用内运行始终以手动步骤为准。

### 3.3 save_curl 参数说明

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| command | ✓ | 完整 cURL 命令字符串 |
| name | | 接口名；缺省从 URL 路径末段推导 |
| projectId | | 目标项目；缺省时唯一项目自动选中，零项目自动创建「Agent 导入」，多项目报错并列出候选 |
| folderId | | 归属文件夹 |

导入行为：

- URL 拆解为 base_url + 路径 + query 参数（与手动 cURL 导入一致），接口状态为「设计中」草稿；
- 项目变量 `base_url` 缺失时自动写入本次 URL 的 origin；
- **已有不同 base_url 时不覆盖**，响应带 `warning` 字段，Agent 会转达。

### 3.4 使用示例

**① 已有 cURL，一句话入库**（Claude Code / Cursor 通用）：

```
把这个 curl 存到 Miyo：
curl -X POST https://api.example.com/orders \
  -H "Content-Type: application/json" \
  -d '{"userId":1,"amount":99}'
```

**② Java 代码里直接让 AI 存接口**：贴出 Controller 代码（或 Cursor 选中方法），AI 自动构造 cURL 并调用 `save_curl`：

```
把下面这个接口保存到 Miyo：
@PostMapping("/orders")
public Result<Long> createOrder(@RequestBody CreateOrderReq req) {
    return orderService.create(req);
}
```

用中文汇报结果：POST /orders 已保存，base_url 推导为 ...，接口名缺省为 orders。

**③ 指定项目 / 文件夹**：先 `list_projects` 拿到 `projectId`，再导入到目标项目：

```
list_projects 里找 projectId，然后 save_curl：
curl -X POST http://127.0.0.1:8080/api/users -d '{"name":"foo"}' name="创建用户" projectId=<projectId>
```

多项目库时若漏填 `projectId`，`save_curl` 会报错并列出候选，照抄 `projectId` 重试即可。

## 4. 方式二：直接 HTTP

适合自研脚本或暂不支持 MCP 的 Agent。

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| POST | `/agent/curl` | `{command, projectId?, name?, folderId?}` → `{endpointId, ...}` |
| GET | `/agent/projects` | 项目列表 |
| GET | `/agent/endpoints/:projectId` | 接口列表 |
| GET | `/agent/health` | 存活探针 |

示例：

```bash
TOKEN=$(cat "$HOME/Library/Application Support/Miyo/agent-token")
curl -s http://127.0.0.1:4110/agent/curl \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"command":"curl -X POST -H \"Content-Type: application/json\" -d \"{\\\"a\\\":1}\" https://api.example.com/orders"}'
```

字段同时接受 camelCase 与 snake_case（`projectId` / `project_id`）。
错误体统一 `{code, message}`：`VALIDATION`(400) / `NOT_FOUND`(404) / `UNAUTHORIZED`(401)。

## 5. 安全设计

- 只绑定 `127.0.0.1`，外网不可达；
- 随机 UUID 令牌，文件权限 0600；
- 写操作仅限「导入接口」一种，不可删除/修改已有数据；
- 不读取密钥环中的加密环境变量明文。

## 6. 排障

| 现象 | 处理 |
| --- | --- |
| `spawn miyo-mcp ENOENT` | 二进制不在 PATH：v0.0.10+ 改用安装包内绝对路径（见 §3.1），或按旧版方式构建后放入 PATH |
| `miyo-mcp` 报「未发现运行中的控制面」 | 先启动 Miyo 桌面应用；确认端口 4110~4129 未被防火墙拦本机回环 |
| 401 UNAUTHORIZED | token 文件与应用不一致（如混用了 dev/正式版目录）；删掉 `agent-token` 后重启应用重新配 |
| 多项目时报 VALIDATION | 让 Agent 先调 `list_projects`，带上 projectId 重试 |
| MCP 工具列表没出现 | 检查 `.mcp.json` 语法与 miyo-mcp 可执行权限（`chmod +x`）；查看客户端 MCP 日志 |
| 导入后 UI 没刷新 | 仅当桌面端正打开同一项目才实时刷新；切项目或重进工作区可见 |
