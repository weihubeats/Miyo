# MCP 集成测试（AI Agent 自动化）

通过 MCP（`miyo-mcp` stdio）让 AI Agent 完成集成测试的完整生命周期：查询项目/接口/环境 → 创建场景（含变量提取与断言）→ 校验 → 执行 → 逐步查看请求/响应 → 分析失败原因。UI 与 MCP 共享同一套数据模型和执行引擎（`miyo-test`），AI 建的场景在应用内「集成测试」页面立即可见、可编辑、可重跑。

## 统一工具注册表（单源）

全部工具在 `crates/miyo-agent/src/registry.rs` **一次性**定义：名称、标题、描述、
Input Schema、业务模块、权限等级、只读 / 副作用 / 破坏性 / 幂等 / 联网、超时、
是否允许并发。消费方共用这一份注册信息：

- `miyo-mcp` 的 `tools/list`（含 MCP 标准注解 `annotations` 与 `x-miyo` 扩展元数据）；
- `miyo-agent` 控制面的服务端权限校验（`ensure_tool_allowed`：注册存在性 →
  工具停用 → 权限等级，逐调用强制）；
- `miyo-tauri` 设置页的工具清单与权限矩阵。

**新增工具的统一入口**：① `TOOL_META` 加元数据；② `TOOL_SCHEMA` 加描述 +
Schema；③ 控制面加路由；④ `miyo-mcp` 的 `validate_tool_call` / `call_tool` 加
分发分支。注册完整性测试（registry ↔ tools/list ↔ 分发的 1:1）会拦截漏项。

## 权限矩阵

五级权限，服务端逐调用校验；高风险等级（network_execute / destructive）**默认
关闭**，需用户在设置页明确确认（`set_mcp_permissions` 要求
`acknowledgeHighRisk: true`）。权限拒绝返回 `PERMISSION_DENIED` 错误码并附
等级说明。

| 等级 | 默认 | 覆盖工具（数） | 说明 |
| --- | --- | --- | --- |
| `read` | 启用 | 17 个查询类（项目 / 接口 / 环境 / 场景 / 运行历史 / 上下文） | 读取数据；返回值经统一脱敏 |
| `write` | 启用 | 9 个写入类（save_curl / 场景与步骤增改 / 用例增改） | 落库副作用 |
| `execute` | 启用 | 3 个执行类（`run_scenario` / `run_test_case` / `cancel_run`） | 引擎编排的受控测试执行 |
| `network_execute` | **关闭** | `send_endpoint` | 直接向外部发起真实 HTTP 请求——URL 可任意覆盖、不受项目接口约束的自由网络能力，独立风险控制 |
| `destructive` | **关闭** | `delete_scenario` / `delete_test_case` | 不可恢复删除 |

另加每工具独立开关（`mcp_disabled_tools`，32 个工具逐个停用，停用后 AI 不可见
且控制面拒绝调用）。设置页「MCP 服务」提供权限矩阵 + 高风险确认对话框 +
首次启用权限说明。

> `execute` 与 `network_execute` 的分界：场景 / 用例执行是「引擎编排的受控执行」
> （目标为项目内已配置接口）；`send_endpoint` 允许覆盖任意 URL——即不受约束的
> 外部网络能力，因此单独一级。

## 工具清单（32 个）

| 分组 | 工具 | 用途 |
| --- | --- | --- |
| 查询 | `list_projects` | 项目列表 |
| | `list_endpoints` | 接口列表（name/method/folderId 筛选 + limit/offset 分页） |
| | `get_endpoint` | 接口完整定义（方法/URL/参数/头/Body/认证） |
| | `list_environments` | 项目环境列表（名称/Base URL/变量数） |
| | `get_environment` | 环境详情（敏感变量值脱敏为占位符） |
| 场景管理 | `list_scenarios` | 场景列表 |
| | `create_scenario` | 新建场景（`steps` 完整定义或 `endpointIds` 骨架） |
| | `update_scenario` | 更新（字段缺省保留；steps 提供即整体替换） |
| | `get_scenario` | 完整上下文（步骤/链路变量/项目变量/脚本） |
| | `delete_scenario` | 删除（响应含被删历史条数） |
| 步骤管理 | `add_scenario_step` | 追加步骤（index 缺省末尾） |
| | `update_scenario_step` | 替换位次步骤 |
| | `delete_scenario_step` | 删除位次步骤 |
| 执行 | `validate_scenario` | 运行前校验（结构化 issues，可多条；`runtimeVariables` 声明运行期注入变量，避免误报缺失） |
| | `run_scenario` | 执行（`wait=false` 异步拿 runId 轮询） |
| 结果 | `get_run_result` | 运行摘要 + 每步摘要（不含响应正文） |
| | `get_step_result` | 单步完整详情（请求快照/响应正文/提取/逐条断言） |
| | `analyze_run_failure` | 失败原因分析（启发式，标注为推测） |
| | `cancel_run` | 取消运行中的场景（返回已完成步骤，落历史 cancelled=true） |
| 测试用例 | `list_test_cases` | 接口的测试用例列表 |
| | `create_test_case` / `update_test_case` / `delete_test_case` | 用例增删改 |
| | `get_test_case` | 单个用例定义 |
| | `run_test_case` | 执行单个用例（真实请求，落执行历史） |
| | `list_test_case_runs` / `get_test_case_run` | 用例执行历史与单次详情 |
| 其他 | `send_endpoint` / `save_curl` / `get_scenario` 脚本 / `save_scenario_script` / `agent_info` | 兼容保留 |

## 统一返回协议

所有工具的 `content[0].text` 为统一信封 JSON：

```json
{ "success": true, "data": { … }, "error": null }
{ "success": false, "data": null, "error": { "code": "SCENARIO_VALIDATION_FAILED", "message": "…", "details": null } }
```

失败时 MCP `isError: true`。错误码：`PROJECT_NOT_FOUND` / `ENDPOINT_NOT_FOUND` / `ENVIRONMENT_NOT_FOUND` / `SCENARIO_NOT_FOUND` / `SCENARIO_VALIDATION_FAILED` / `VARIABLE_NOT_FOUND` / `VARIABLE_USED_BEFORE_DEFINED` / `STEP_EXECUTION_FAILED` / `RUN_NOT_FOUND` / `RUN_CANCELLED` / `PERMISSION_DENIED`（权限等级未启用或工具停用，附等级说明）/ `TOOL_NOT_FOUND`（未注册工具）/ `TOOL_DISABLED` / `INVALID_ARGUMENT` / `INTERNAL`。

## 步骤定义（AI 方言自动翻译）

```json
{
  "name": "创建订单",
  "request": {
    "method": "POST",
    "url": "{{base_url}}/shop/orders",
    "spec": { "body": { "mode": "json", "raw": "{\"sku\": \"A\"}" } }
  },
  "extractors": [
    { "name": "orderNo",  "source": "response.body", "expression": "$.data.orderNo", "required": true },
    { "name": "payToken", "source": "response.body", "expression": "$.data.payToken" }
  ],
  "assertions": [
    { "source": "response.status", "operator": "equals", "expected": 200 },
    { "source": "response.body", "expression": "$.data.status", "operator": "equals", "expected": "SUCCESS" },
    { "source": "response.body", "expression": "$.data.amount", "operator": "greaterThan", "expected": 0 },
    { "source": "response.header", "expression": "x-req-id", "operator": "exists" },
    { "source": "performance", "operator": "lessThan", "expected": 2000 }
  ],
  "condition": { "source": "var", "name": "paid", "op": "exists" },
  "loop": { "mode": "while", "while_condition": { "source": "response.body", "expression": "$.code", "op": "neq", "value": 200 }, "max_loops": 10, "interval_ms": 500 },
  "waitMs": 0,
  "parallel": false
}
```

- `request`（内联请求）与 `endpointId`（接口引用）二选一；URL / Body / Query 里用 `{{变量名}}` 引用前序提取值（JSON Body 整值占位保留原始类型）。
- `extractors.source`：`response.body`（JSONPath）/ `response.header`；`response.status` 不支持提取（用断言）。
- `assertions.operator`：equals / notEquals / contains / notContains / greaterThan / greaterThanOrEquals / lessThan / lessThanOrEquals / exists / notExists / matches（正则）/ empty / notEmpty。
- `loop.mode`：`count`（固定轮数 ≤1000）或 `while`（每轮后求值继续条件，`max_loops` ≤100，达上限仍成立判失败）。`while_condition.source` 支持 `var` / `prevStatus` / `response.status` / `response.body` / `response.header`——响应源直接对本轮响应求值（`response.body` 用 `expression` 传 JSONPath、`response.header` 传头名），异步轮询「本轮响应码 ≠ X 则重试」无需先 extract 再引用；`var` 源继续用 `name`。
- `parallel: true` 的相邻步骤成批并发（≤8），变量提取按完成序合并。

## 典型工作流（AI 全自动）

```
list_projects → list_endpoints → get_endpoint（看清参数与响应结构）
→ create_scenario（steps 一步到位，提取 + 断言）
→ validate_scenario（变量缺失/先引用后定义/写操作清单）
→ run_scenario（生产环境先经用户确认；写操作建议幂等键）
→ get_run_result → get_step_result（逐步核对）
→ analyze_run_failure（失败时）→ 修复步骤 → 重跑
```

接口信息不足时 AI 应先询问用户，不要编造参数；校验不通过不要绕过。

## 安全策略

- 控制面 Bearer 令牌 + 端口文件发现；工具级启停（设置 → MCP）映射 `PERMISSION_DENIED`。
- 环境变量敏感值、请求快照认证头、变量池落库均脱敏（`__MIYO_REDACTED__`）。
- URL scheme 白名单（http/https）；响应体 64KB 截断；步骤超时（接口级 → 场景级 → 300s）；同场景并发守卫。
- `validate_scenario` 返回写操作步骤清单与幂等键建议；支付/退款/删除等有副作用操作执行前必须经用户确认。
- `analyze_run_failure` 输出为规则推测，明确标注非确定结论。

## MCP 协议能力（2025-06-18 核心集）

协议版本回显客户端请求值（兼容 2024-11-05 及以后客户端）。`initialize` 声明：

```json
{
  "capabilities": {
    "tools": { "listChanged": true },
    "resources": {},
    "prompts": {}
  }
}
```

### Resources（`resources/list` / `resources/read`）

只读资源通道，读取走既有控制面端点（权限 = read，脱敏同口径）：

| URI | 内容 |
| --- | --- |
| `miyo://context` | Agent 上下文快照（当前项目 / 环境 / 资产摘要） |
| `miyo://projects` | 全部项目（敏感变量脱敏） |
| `miyo://projects/{projectId}/endpoints` | 项目接口（认证与敏感头脱敏） |
| `miyo://projects/{projectId}/environments` | 项目环境清单 |
| `miyo://projects/{projectId}/scenarios` | 项目场景摘要 |
| `miyo://environments/{environmentId}` | 环境详情（敏感值占位符） |
| `miyo://runs/{runId}` | 场景运行结果 |

未注册 URI 一律 `NOT_FOUND`（不做自由路径拼接）。

### Prompts（`prompts/list` / `prompts/get`）

三个标准化工作流模板（纯文本生成，不读项目数据）：`design-api-tests`
（接口测试用例设计）、`author-scenario`（集成场景编排）、`analyze-failure`
（运行失败分析）。

### 通知

- `notifications/tools/list_changed`：initialize 后每 15s 轮询 `/agent/tools`，
  启停集合变化即通知（客户端无需重启会话即可刷新工具列表）；
- `notifications/progress`：`tools/call` 携带 `_meta.progressToken` 时，
  `run_scenario`（wait=true）运行期间每 1.5s 轮询
  `GET /agent/runs/{id}/progress` 转发 `{progress, total, message}`。

### 工具注解与元数据

`tools/list` 每个工具附 MCP 标准注解（`title` / `readOnlyHint` /
`destructiveHint` / `idempotentHint` / `openWorldHint`）与 `x-miyo` 扩展
（`module` / `permission` / `timeoutMs` / `concurrent` / `sideEffects`）。
注册表 `concurrent=false` 的工具（`save_scenario_script` / `update_scenario` /
`update_test_case` 等 read-modify-write 类）在 miyo-mcp 层按名串行执行。

### get_context 工具

`get_context`（read 权限）一次性返回工作现场：当前激活项目（敏感变量脱敏）、
当前激活环境、接口清单（id / 方法 / 路径，**不含请求规格明细**）、环境清单、
场景摘要、用例数量、最近一次运行摘要。AI 需要详情时用 `get_endpoint` 等工具
按需拉取——不默认把全部项目数据发给 AI。

## 集成测试引擎升级（数据驱动 / 生命周期 / 重试 / JSON Schema / 报告）

场景数据模型（`ScenarioStep` / `Scenario`）扩展了以下字段，全部 `serde(default)`
向后兼容（旧场景零迁移加载；数据库为增量列 + 严格默认值，见迁移 0027）：

| 能力 | 字段 / 参数 | 行为 |
| --- | --- | --- |
| **步骤重试** | `ScenarioStep.retry`（max_attempts ≤10 / interval_ms / exponential / retry_on_status / retry_on_network） | 显式配置才启用；只重试命中状态码与网络错误；**副作用步骤（POST/PUT/PATCH/DELETE）自动注入步骤级幂等键**（全部尝试共用一键）；重试日志随结果落库（`retry_log`），最终失败原因 = 最后一次尝试 |
| **生命周期** | `Scenario.setup` / `teardown` / `teardown_enabled` | Setup 提取产物流入主步骤；Setup 失败 → 主步骤全部 skipped，**Teardown 仍执行**（独立取消令牌，失败 / 超时 / 取消时尽可能执行）；清理失败单独记录（`teardown_results`），不影响主步骤成败统计；清理写操作自动幂等键防重复清理副作用 |
| **数据驱动** | `Scenario.data_sets`（inline / json / csv / environment）+ MCP `run_scenario` 的 `dataSets` 参数 | 每组数据独立执行、独立结果、独立落历史（meta.matrix 标注 + groupId 关联）；行变量覆盖环境变量，行间变量零污染 |
| **JSON Schema 断言** | 断言 `type: "json_schema"`（path = JSONPath，expected = Schema 对象） | 完整校验（必填 / 类型 / 数值范围 / 格式 / 数组 / 嵌套 / 枚举 / additionalProperties）；错误逐条带 JSON 路径（`$.data.orderId: …`，required 缺失指向 `$.data.字段`） |
| **环境矩阵** | Tauri `run_scenario_matrix`（rows = 环境行，变量按行独立解析） | 同场景逐环境执行，逐行落历史（groupId），汇总返回 |
| **结果增强** | `EndpointResult.started_at` / `retry_log` / `phase` | 每步请求时间、重试记录、执行阶段（setup/teardown）；敏感信息脱敏口径延续（请求快照 / 提取值 / URL query） |
| **测试报告** | Tauri `export_scenario_report`（html / junit_xml / json） | 场景成功率 / 步骤成功率 / 平均耗时 / P50-P99 / 失败步骤统计 / 断言失败统计 / 环境对比；JUnit XML 每步一个 testcase（CI 消费）；HTML 自包含 |
| **失败恢复** | meta.varsSnapshots（v2 落库，脱敏） | 历史回看可重跑失败步骤（此前仅实时运行可用）；「从失败步骤运行」恢复链路上下文并显示来源 |

执行入口全覆盖：应用内编辑器、CLI（`run_scenario` 同一引擎）、MCP `run_scenario`
（`dataSets` 数据驱动 / 生命周期随场景自动执行）。

**setup/teardown 与统计口径**：`results_json` 行 = setup(phase=setup) + 主步骤 +
teardown(phase=teardown)；成功率 / ok_count / fail_count 只统计主步骤
（`phase = None`），生命周期失败单独可见不混入。