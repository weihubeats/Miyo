# MCP 集成测试（AI Agent 自动化）

通过 MCP（`miyo-mcp` stdio）让 AI Agent 完成集成测试的完整生命周期：查询项目/接口/环境 → 创建场景（含变量提取与断言）→ 校验 → 执行 → 逐步查看请求/响应 → 分析失败原因。UI 与 MCP 共享同一套数据模型和执行引擎（`miyo-test`），AI 建的场景在应用内「集成测试」页面立即可见、可编辑、可重跑。

## 工具清单（31 个）

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

失败时 MCP `isError: true`。错误码：`PROJECT_NOT_FOUND` / `ENDPOINT_NOT_FOUND` / `ENVIRONMENT_NOT_FOUND` / `SCENARIO_NOT_FOUND` / `SCENARIO_VALIDATION_FAILED` / `VARIABLE_NOT_FOUND` / `VARIABLE_USED_BEFORE_DEFINED` / `STEP_EXECUTION_FAILED` / `RUN_NOT_FOUND` / `RUN_CANCELLED` / `PERMISSION_DENIED`（工具停用或鉴权失败）/ `INVALID_ARGUMENT` / `UNKNOWN_TOOL`。

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
