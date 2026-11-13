//! MCP 工具统一注册表（单源）。
//!
//! 全部 MCP 工具在这里**一次性**定义：名称 / 标题 / 描述 / Input Schema /
//! 业务模块 / 权限等级 / 只读 / 副作用 / 破坏性 / 幂等 / 联网 / 超时 /
//! 并发。消费方：
//!
//! - `miyo-agent/server.rs`：`ensure_tool_allowed` 按 [`TOOL_META`] 做服务端
//!   权限校验（工具停用 + 权限等级），`list_tools` 回显启停状态；
//! - `miyo-mcp/tools.rs`：`tools/list` 用 [`tool_definitions_json`] 生成
//!   （含 MCP 标准注解 annotations + `x-miyo` 扩展元数据）；
//! - `miyo-tauri/commands/agent.rs`：设置页工具清单与权限矩阵的数据源。
//!
//! **新增工具的统一入口**：① [`TOOL_META`] 加一行元数据；② [`TOOL_SCHEMA`]
//! 加一条描述 + Schema；③ miyo-agent 控制面加路由；④ miyo-mcp 的
//! `call_tool` / `validate_tool_call` 加分发分支。注册完整性测试
//! （registry 与 tools/list、与 miyo-mcp 分发的 1:1 一致）会拦截漏项。

use serde_json::{json, Value};

/// 工具权限等级（从低到高；等级独立开关，不做隐含包含）。
///
/// - `Read`：读取项目 / 接口 / 环境 / 场景 / 历史等数据；
/// - `Write`：创建 / 修改项目数据（落库副作用）；
/// - `Execute`：执行测试用例与集成测试场景（引擎编排的受控执行）；
/// - `NetworkExecute`：直接向外部发起真实网络请求（send_endpoint 的
///   URL 可任意覆盖——不受项目接口约束的自由网络能力，独立高风险控制）；
/// - `Destructive`：删除项目数据等不可恢复操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PermissionLevel {
    Read,
    Write,
    Execute,
    NetworkExecute,
    Destructive,
}

impl PermissionLevel {
    /// settings 表存储 / 前端展示用的小写标识。
    pub fn as_str(&self) -> &'static str {
        match self {
            PermissionLevel::Read => "read",
            PermissionLevel::Write => "write",
            PermissionLevel::Execute => "execute",
            PermissionLevel::NetworkExecute => "network_execute",
            PermissionLevel::Destructive => "destructive",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "read" => PermissionLevel::Read,
            "write" => PermissionLevel::Write,
            "execute" => PermissionLevel::Execute,
            "network_execute" => PermissionLevel::NetworkExecute,
            "destructive" => PermissionLevel::Destructive,
            _ => return None,
        })
    }

    /// 中文标签（设置页 / 拒绝消息用）。
    pub fn label(&self) -> &'static str {
        match self {
            PermissionLevel::Read => "读取数据",
            PermissionLevel::Write => "创建与修改数据",
            PermissionLevel::Execute => "执行测试用例与场景",
            PermissionLevel::NetworkExecute => "发起真实网络请求",
            PermissionLevel::Destructive => "删除与清理（不可恢复）",
        }
    }

    /// 高风险等级：默认关闭，必须用户明确确认后才能启用。
    pub fn is_high_risk(&self) -> bool {
        matches!(
            self,
            PermissionLevel::NetworkExecute | PermissionLevel::Destructive
        )
    }

    /// 全部等级（固定顺序，UI 与校验共用）。
    pub const ALL: [PermissionLevel; 5] = [
        PermissionLevel::Read,
        PermissionLevel::Write,
        PermissionLevel::Execute,
        PermissionLevel::NetworkExecute,
        PermissionLevel::Destructive,
    ];
}

/// 工具所属业务模块（设置页分组展示 + 审计归类）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Project,
    Endpoint,
    Environment,
    Scenario,
    TestCase,
    Run,
    System,
}

impl Module {
    pub fn as_str(&self) -> &'static str {
        match self {
            Module::Project => "project",
            Module::Endpoint => "endpoint",
            Module::Environment => "environment",
            Module::Scenario => "scenario",
            Module::TestCase => "test_case",
            Module::Run => "run",
            Module::System => "system",
        }
    }
}

/// 工具元数据（Schema 与描述见 [`TOOL_SCHEMA`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolMeta {
    pub name: &'static str,
    /// 短标题（MCP `annotations.title` / 设置页）。
    pub title: &'static str,
    pub module: Module,
    pub permission: PermissionLevel,
    /// 只读（MCP `readOnlyHint`）。
    pub read_only: bool,
    /// 有副作用：落库或对外部系统产生状态变化。
    pub side_effects: bool,
    /// 破坏性：删除 / 不可恢复（MCP `destructiveHint`）。
    pub destructive: bool,
    /// 幂等（MCP `idempotentHint`）。
    pub idempotent: bool,
    /// 会访问应用之外的世界（MCP `openWorldHint`：真实网络请求）。
    pub open_world: bool,
    /// 建议超时毫秒（客户端可据此设置等待上限；服务端有独立兜底）。
    pub timeout_ms: u64,
    /// 是否允许并发调用（false = miyo-mcp 层按工具名串行执行，
    /// 防 read-modify-write 交错互相覆盖）。
    pub concurrent: bool,
}

/// 工具元数据表：全部 MCP 工具的唯一清单。
pub static TOOL_META: &[ToolMeta] = &[
    ToolMeta {
        name: "save_curl",
        title: "保存 cURL 为接口",
        module: Module::Endpoint,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "list_projects",
        title: "列出项目",
        module: Module::Project,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "list_endpoints",
        title: "列出接口",
        module: Module::Endpoint,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_endpoint",
        title: "读取接口定义",
        module: Module::Endpoint,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "send_endpoint",
        title: "发送真实请求",
        module: Module::Endpoint,
        permission: PermissionLevel::NetworkExecute,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: true,
        timeout_ms: 660000,
        concurrent: true,
    },
    ToolMeta {
        name: "run_scenario",
        title: "运行测试场景",
        module: Module::Scenario,
        permission: PermissionLevel::Execute,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: true,
        timeout_ms: 660000,
        concurrent: true,
    },
    ToolMeta {
        name: "list_scenarios",
        title: "列出场景",
        module: Module::Scenario,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_scenario",
        title: "读取场景详情",
        module: Module::Scenario,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "create_scenario",
        title: "新建场景",
        module: Module::Scenario,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "save_scenario_script",
        title: "保存场景脚本",
        module: Module::Scenario,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: false,
    },
    ToolMeta {
        name: "agent_info",
        title: "控制面连接信息",
        module: Module::System,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_context",
        title: "Agent 上下文快照",
        module: Module::System,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "list_environments",
        title: "列出环境",
        module: Module::Environment,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_environment",
        title: "读取环境详情",
        module: Module::Environment,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "update_scenario",
        title: "更新场景",
        module: Module::Scenario,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: false,
    },
    ToolMeta {
        name: "delete_scenario",
        title: "删除场景",
        module: Module::Scenario,
        permission: PermissionLevel::Destructive,
        read_only: false,
        side_effects: true,
        destructive: true,
        idempotent: false,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "add_scenario_step",
        title: "追加场景步骤",
        module: Module::Scenario,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: false,
        timeout_ms: 30000,
        concurrent: false,
    },
    ToolMeta {
        name: "update_scenario_step",
        title: "替换场景步骤",
        module: Module::Scenario,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: false,
    },
    ToolMeta {
        name: "delete_scenario_step",
        title: "删除场景步骤",
        module: Module::Scenario,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: false,
    },
    ToolMeta {
        name: "validate_scenario",
        title: "校验场景",
        module: Module::Scenario,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 60000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_run_result",
        title: "查询运行结果",
        module: Module::Run,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_step_result",
        title: "查询单步详情",
        module: Module::Run,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "list_test_cases",
        title: "列出测试用例",
        module: Module::TestCase,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_test_case",
        title: "读取测试用例",
        module: Module::TestCase,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "create_test_case",
        title: "创建测试用例",
        module: Module::TestCase,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "update_test_case",
        title: "更新测试用例",
        module: Module::TestCase,
        permission: PermissionLevel::Write,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: false,
    },
    ToolMeta {
        name: "delete_test_case",
        title: "删除测试用例",
        module: Module::TestCase,
        permission: PermissionLevel::Destructive,
        read_only: false,
        side_effects: true,
        destructive: true,
        idempotent: false,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "run_test_case",
        title: "执行测试用例",
        module: Module::TestCase,
        permission: PermissionLevel::Execute,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: false,
        open_world: true,
        timeout_ms: 660000,
        concurrent: true,
    },
    ToolMeta {
        name: "list_test_case_runs",
        title: "列出用例执行历史",
        module: Module::Run,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "get_test_case_run",
        title: "读取用例执行详情",
        module: Module::Run,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    ToolMeta {
        name: "analyze_run_failure",
        title: "分析运行失败",
        module: Module::Run,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "cancel_run",
        title: "取消运行",
        module: Module::Run,
        permission: PermissionLevel::Execute,
        read_only: false,
        side_effects: true,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 15000,
        concurrent: true,
    },
    // ---- AI 测试中心：分析 / 生成类工具（全部只读，产出草稿不落库）----
    ToolMeta {
        name: "analyze_api_dependencies",
        title: "分析接口依赖",
        module: Module::Project,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "analyze_project_test_coverage",
        title: "分析测试覆盖率",
        module: Module::Project,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "generate_test_cases",
        title: "生成测试用例草稿",
        module: Module::TestCase,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "generate_integration_scenario",
        title: "生成场景草稿",
        module: Module::Scenario,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
    ToolMeta {
        name: "propose_scenario_fix",
        title: "生成修复草稿",
        module: Module::Scenario,
        permission: PermissionLevel::Read,
        read_only: true,
        side_effects: false,
        destructive: false,
        idempotent: true,
        open_world: false,
        timeout_ms: 30000,
        concurrent: true,
    },
];

/// 描述 + Input Schema 表：`(name, description, inputSchema)`。
/// Schema 为 MCP JSON Schema 文本（惰性解析，[`tool_definitions_json`] 消费）。
pub static TOOL_SCHEMA: &[(&str, &str, &str)] = &[
    (
        "save_curl",
        r#"把一条 cURL 命令解析并保存为 Miyo 接口（API 调试工具中的可编辑接口）。URL 拆为 base_url + 路径 + 查询参数；返回 endpointId。多个项目时必须传 projectId。"#,
        r#"{"type":"object","properties":{"command":{"type":"string","description":"完整 cURL 命令字符串"},"name":{"type":"string","description":"可选，接口名称；缺省从 URL 路径推导"},"projectId":{"type":"string","description":"可选，目标项目 UUID；缺省时唯一项目自动选中、零项目自动创建"},"folderId":{"type":"string","description":"可选，归属文件夹 UUID"}},"required":["command"]}"#,
    ),
    (
        "list_projects",
        r#"列出 Miyo 中全部项目（id / 名称 / 描述 / 变量）。"#,
        r#"{"type":"object","properties":{}}"#,
    ),
    (
        "list_endpoints",
        r#"列出指定项目下的接口（方法 / 路径 / 名称 / 请求规格）。支持按名称子串（匹配名称或路径）、HTTP 方法、所属分组过滤，支持分页；全缺省时返回全量（与旧版兼容）。创建场景前用它了解项目里有哪些接口。"#,
        r#"{"type":"object","properties":{"projectId":{"type":"string","description":"项目 UUID（先调 list_projects 获取）"},"name":{"type":"string","description":"可选，名称/路径子串过滤（大小写不敏感）"},"method":{"type":"string","description":"可选，HTTP 方法过滤（GET/POST/…，大小写不敏感）"},"folderId":{"type":"string","description":"可选，分组 UUID 过滤"},"limit":{"type":"number","description":"可选，分页条数（配合 offset；缺省全量）"},"offset":{"type":"number","description":"可选，分页偏移（缺省 0）"}},"required":["projectId"]}"#,
    ),
    (
        "get_endpoint",
        r#"读取单个接口的完整定义（方法 / 路径 / 请求头 / 参数 / Body / 认证形态），调用 send_endpoint 前先看它。"#,
        r#"{"type":"object","properties":{"endpointId":{"type":"string","description":"接口 UUID（list_endpoints / save_curl 返回）"}},"required":["endpointId"]}"#,
    ),
    (
        "send_endpoint",
        r#"发送一次真实 HTTP 请求（链式调用的核心工具）。按 Miyo 中已保存接口的当前配置执行，支持覆盖：url（完整地址）、headers、query、body（请求体原文）、variables（额外变量，渲染进 {{var}} 占位）。返回完整响应（状态码 / 耗时 / 正文，超 64KB 截断）。典型链路：调用下单接口 → 从响应正文提取订单号 → 用 variables/body 把订单号传给支付预览接口 → 再调支付接口。"#,
        r#"{"type":"object","properties":{"endpointId":{"type":"string","description":"接口 UUID（list_endpoints / save_curl 返回）"},"url":{"type":"string","description":"可选，覆盖完整 URL 模板（支持 {{var}} 占位）"},"headers":{"type":"object","description":"可选，追加/覆盖请求头 {\"名\": \"值\"}","additionalProperties":{"type":"string"}},"query":{"type":"object","description":"可选，追加/覆盖 query 参数","additionalProperties":{"type":"string"}},"body":{"type":"string","description":"可选，覆盖请求体原文（Json/Text 原位替换；其余类型降级为 text）"},"variables":{"type":"object","description":"可选，额外变量注入（优先级最高），如 {\"orderId\": \"A1024\"}","additionalProperties":{"type":"string"}},"environmentId":{"type":"string","description":"可选，环境 UUID（缺省用全局+项目变量）"},"timeoutMs":{"type":"number","description":"可选，本次请求超时毫秒（缺省 接口级→全局设置→300s）"}},"required":["endpointId"]}"#,
    ),
    (
        "run_scenario",
        r#"运行集成测试场景：按步骤顺序执行，条件/循环/并行按配置生效，前一步提取的链路变量自动传入后续步骤，逐步执行断言并落运行历史。dataSets 传多组数据时按数据驱动模式执行：每组变量隔离、独立落历史、返回逐行汇总。返回 runId 与每步摘要（状态码/断言/提取值/响应预览）。长场景建议 wait=false 拿 runId 后用 get_run_result 轮询。生产环境或含写操作（POST/PUT/DELETE）的场景先经用户确认，建议带幂等键（如 Idempotency-Key 请求头）防重复下单。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID（list_scenarios 返回）"},"environmentId":{"type":"string","description":"可选，环境 UUID（缺省用全局+项目变量；生产环境先经用户确认）"},"stopOnFailure":{"type":"boolean","description":"可选，失败即停（默认 true）"},"variables":{"type":"object","description":"可选，初始变量注入（优先级最高）","additionalProperties":{"type":"string"}},"dataSets":{"type":"array","description":"可选，数据驱动：多组数据独立执行（每组变量隔离、独立结果、独立落历史）","items":{"type":"object","properties":{"name":{"type":"string"},"vars":{"type":"object","additionalProperties":{"type":"string"}}},"required":["name"]}},"autoIdempotency":{"type":"boolean","description":"可选，true 时为写操作步骤自动补 Idempotency-Key 头（键 = runId-步骤位次，重试不重复执行业务）——AI 重试或不确定幂等性时建议开启"},"wait":{"type":"boolean","description":"可选，true（默认）同步等待完整结果；false 立即返回 runId，用 get_run_result 轮询"}},"required":["scenarioId"]}"#,
    ),
    (
        "list_scenarios",
        r#"列出 Miyo 中的集成测试场景（接口链路）。返回 id / 名称 / 描述 / 步骤数 / 是否已有自定义脚本。"#,
        r#"{"type":"object","properties":{"projectId":{"type":"string","description":"可选，项目 UUID；缺省时仅一个项目自动选中"}}}"#,
    ),
    (
        "create_scenario",
        r#"新建集成测试场景。两种步骤来源：① steps 完整定义（推荐）——每步含内联请求或接口引用、变量提取（extractors 或 extract）、断言（source/operator 形状自动翻译）、执行条件/循环，顺序即数组顺序；② endpointIds 顺序引用（仅建骨架，无提取/断言）。返回 scenarioId。推荐工作流：list_endpoints → get_endpoint 看清参数与响应 → create_scenario（steps 一步到位）→ validate_scenario → run_scenario。接口信息不足时先问用户，不要编造字段。"#,
        r#"{"type":"object","properties":{"name":{"type":"string","description":"场景名称（不可为空）"},"projectId":{"type":"string","description":"可选，目标项目 UUID；缺省时仅一个项目自动选中"},"description":{"type":"string","description":"可选，场景说明"},"category":{"type":"string","description":"可选，分类标签"},"endpointIds":{"type":"array","items":{"type":"string"},"description":"可选，按顺序引用的接口 UUID（无提取/断言的骨架）"},"steps":{"type":"array","description":"可选，完整步骤数组（优先于 endpointIds）。每步：{\"name\":\"步骤名\",\"request\":{\"method\":\"POST\",\"url\":\"{{base_url}}/path\",\"spec\":{}}} 或 {\"endpointId\":\"UUID\",\"extractors\":[{\"name\":\"orderNo\",\"source\":\"response.body\",\"expression\":\"$.data.orderNo\"}],\"assertions\":[{\"source\":\"response.status\",\"operator\":\"equals\",\"expected\":200},{\"source\":\"response.body\",\"expression\":\"$.data.status\",\"operator\":\"equals\",\"expected\":\"SUCCESS\"}],\"condition\":{\"source\":\"var\",\"name\":\"paid\",\"op\":\"exists\"},\"loop\":{\"mode\":\"while\",\"while_condition\":{...},\"max_loops\":10,\"interval_ms\":500},\"waitMs\":0,\"parallel\":false}。URL/Body 里用 {{变量名}} 引用前一步提取的值。","items":{"type":"object"}}},"required":["name"]}"#,
    ),
    (
        "get_scenario",
        r#"读取集成测试场景的完整上下文，用于编写或修改场景脚本（Node fetch 风格）：每一步引用接口的当前配置（方法 / 路径 / 请求规格）、提取规则与断言、链路变量（前步提取、后步引用）、项目变量（含 base_url 兜底地址）与当前自定义脚本。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID（先调 list_scenarios 获取）"}},"required":["scenarioId"]}"#,
    ),
    (
        "save_scenario_script",
        r#"写入集成测试场景的自定义脚本（覆盖保存）。脚本为 Node 18+ 原生 fetch 风格（顶层 await），链路变量直接用前步提取的变量名引用；传空字符串表示清除自定义、回到由手动步骤实时生成。应用内运行始终以手动步骤为准，脚本用于复制 / CI 导出。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"},"script":{"type":"string","description":"脚本全文（空字符串 = 清除自定义）"}},"required":["scenarioId","script"]}"#,
    ),
    (
        "list_environments",
        r#"列出项目的执行环境（名称 / Base URL / 启用变量数）。场景执行前用它确认目标环境；生产命名的环境执行前必须经用户确认。"#,
        r#"{"type":"object","properties":{"projectId":{"type":"string","description":"可选，项目 UUID；缺省时仅一个项目自动选中"}}}"#,
    ),
    (
        "get_environment",
        r#"读取环境详情：Base URL 与全部启用变量（敏感变量的值已脱敏为占位符，不可读出明文）。用于确认 {{变量}} 引用是否在环境中可用。"#,
        r#"{"type":"object","properties":{"environmentId":{"type":"string","description":"环境 UUID（list_environments 返回）"}},"required":["environmentId"]}"#,
    ),
    (
        "update_scenario",
        r#"更新场景：名称 / 描述 / 分类按需覆盖；steps 提供时整体替换全部步骤（形状同 create_scenario 的 steps，支持 AI 方言）。字段缺省保留原值。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"},"name":{"type":"string","description":"可选，新名称"},"description":{"type":"string","description":"可选，新描述"},"category":{"type":"string","description":"可选，新分类"},"steps":{"type":"array","items":{"type":"object"},"description":"可选，整体替换的步骤数组（形状同 create_scenario.steps）"}},"required":["scenarioId"]}"#,
    ),
    (
        "delete_scenario",
        r#"删除场景（不可恢复）。运行历史随场景一并删除，响应中 deletedRuns 为被删历史条数——如需保留历史请先告知用户确认。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"}},"required":["scenarioId"]}"#,
    ),
    (
        "add_scenario_step",
        r#"向场景追加一个步骤（index 缺省追加到末尾）。步骤形状同 create_scenario 的 steps 元素（支持 AI 方言：extractors + source/operator 断言）。用于逐步构建或修改场景，免整场景重发。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"},"step":{"type":"object","description":"步骤定义（同 create_scenario.steps 元素）"},"index":{"type":"number","description":"可选，插入位次（0-based；缺省 = 末尾）"}},"required":["scenarioId","step"]}"#,
    ),
    (
        "update_scenario_step",
        r#"替换场景中指定位次的步骤（完整定义，形状同 create_scenario.steps 元素）。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"},"index":{"type":"number","description":"步骤位次（0-based）"},"step":{"type":"object","description":"新步骤定义"}},"required":["scenarioId","index","step"]}"#,
    ),
    (
        "delete_scenario_step",
        r#"删除场景中指定位次的步骤（其后步骤前移）。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"},"index":{"type":"number","description":"步骤位次（0-based）"}},"required":["scenarioId","index"]}"#,
    ),
    (
        "validate_scenario",
        r#"运行前校验场景：项目/接口/环境存在性、步骤配置完整性、变量引用分析（缺失/先引用后定义）、提取与断言表达式合法性、条件/循环配置、写操作步骤清单（副作用提示）。返回结构化 issues（可多条）。执行 run_scenario 前必须先校验通过；校验不通过不要绕过，缺接口信息时先询问用户。"#,
        r#"{"type":"object","properties":{"scenarioId":{"type":"string","description":"场景 UUID"},"environmentId":{"type":"string","description":"可选，按指定环境校验变量可用性"},"runtimeVariables":{"type":"array","items":{"type":"string"},"description":"可选，运行期将经 run_scenario 的 variables 注入的变量名（如每次运行传唯一订单号/幂等键）——这些引用不再报 VARIABLE_NOT_FOUND"}},"required":["scenarioId"]}"#,
    ),
    (
        "get_run_result",
        r#"获取场景执行结果摘要：runId / 状态（running/completed/failed/unknown）/ 汇总（通过/失败/跳过步数、耗时）与每步摘要（状态码/断言通过数/提取值——不含响应正文）。run_scenario 传 wait=false 后用它轮询；completed 后用 get_step_result 看单步详情，用 analyze_run_failure 分析失败。"#,
        r#"{"type":"object","properties":{"runId":{"type":"string","description":"运行 UUID（run_scenario 返回）"}},"required":["runId"]}"#,
    ),
    (
        "get_step_result",
        r#"获取场景运行中单个步骤的完整详情：实际请求（URL/方法/头/Body 快照）、完整响应（状态码/头/正文，超 64KB 截断）、变量提取结果、逐条断言（预期值/实际值/失败原因）。避免一次性拉全部响应，按需逐步查看。"#,
        r#"{"type":"object","properties":{"runId":{"type":"string","description":"运行 UUID"},"stepIndex":{"type":"number","description":"步骤位次（0-based，get_run_result 摘要中的 index）"}},"required":["runId","stepIndex"]}"#,
    ),
    (
        "list_test_cases",
        r#"列出接口的测试用例（名称/分组/分类/最近执行状态与断言通过数）。先 get_endpoint 看接口，再列它的用例。"#,
        r#"{"type":"object","properties":{"endpointId":{"type":"string","description":"接口 UUID"}},"required":["endpointId"]}"#,
    ),
    (
        "get_test_case",
        r#"获取测试用例完整定义：请求快照（方法/路径/路径变量/Params/Headers/Body/认证/超时）、断言规则、变量提取规则、最近执行状态。"#,
        r#"{"type":"object","properties":{"caseId":{"type":"string","description":"测试用例 UUID"}},"required":["caseId"]}"#,
    ),
    (
        "create_test_case",
        r#"创建测试用例（保存为一个可重复执行的请求快照 + 断言 + 提取）。request.method/urlPath 必填；assertions/extractors 为引擎方言数组（断言 type: status/jsonpath/header/body/response_time_ms/length/graphql_errors/javascript；提取 from: body/header/cookie/body_regex）。会落库（副作用）。"#,
        r#"{"type":"object","properties":{"endpointId":{"type":"string","description":"归属接口 UUID（request_id）"},"name":{"type":"string","description":"用例名"},"category":{"type":"string","description":"分组：正向/负向/边界值/安全性/其他","enum":["正向","负向","边界值","安全性","其他"]},"folder":{"type":"string","description":"可选，自由文件夹名（空 = 未分组）"},"request":{"type":"object","description":"请求快照：{method, urlPath, pathVariables?, params?, headers?, bodyType?, bodyContent?, auth?, timeoutMs?, disableCookies?}","properties":{"method":{"type":"string"},"urlPath":{"type":"string","description":"相对路径（配合环境 base_url）或完整 URL；{var} 路径变量随 pathVariables 赋值"},"params":{"type":"array","items":{"type":"object","properties":{"key":{"type":"string"},"value":{"type":"string"},"enabled":{"type":"boolean"}}}},"headers":{"type":"array","items":{"type":"object","properties":{"key":{"type":"string"},"value":{"type":"string"},"enabled":{"type":"boolean"}}}},"pathVariables":{"type":"array","items":{"type":"object","properties":{"key":{"type":"string"},"value":{"type":"string"}}}},"bodyType":{"type":"string","enum":["none","json","raw","urlencoded","form-data","graphql","binary"]},"bodyContent":{"type":"string","description":"json/raw 为文本；urlencoded/form-data 为 KV 数组的 JSON 字符串"},"auth":{"type":"object","description":"可选，AuthSpec（如 {type:'bearer', token:'…'}）——属敏感信息，仅用户显式提供时写入"},"timeoutMs":{"type":"number"},"disableCookies":{"type":"boolean"}},"required":["method","urlPath"]},"assertions":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"type":{"type":"string"},"path":{"type":"string"},"op":{"type":"string"},"expected":{}}}},"extractors":{"type":"array","items":{"type":"object","properties":{"name":{"type":"string"},"from":{"type":"string"},"path":{"type":"string"}}}}},"required":["endpointId","name","request"]}"#,
    ),
    (
        "update_test_case",
        r#"整体更新测试用例（字段覆盖语义；会落库）。结构同 create_test_case。"#,
        r#"{"type":"object","properties":{"caseId":{"type":"string"},"name":{"type":"string"},"category":{"type":"string"},"folder":{"type":"string"},"request":{"type":"object","description":"同 create_test_case 的 request"},"assertions":{"type":"array"},"extractors":{"type":"array"}},"required":["caseId"]}"#,
    ),
    (
        "delete_test_case",
        r#"删除测试用例（不可恢复，执行历史级联删除）。删除前必须与用户确认。"#,
        r#"{"type":"object","properties":{"caseId":{"type":"string"}},"required":["caseId"]}"#,
    ),
    (
        "run_test_case",
        r#"执行一个测试用例（真实发送请求）：返回结构化结果（状态码/耗时/逐条断言 expected/actual/passed/reason/提取值/响应快照摘要），并落执行历史。写操作（POST/PUT/PATCH/DELETE）执行前必须与用户确认目标环境。"#,
        r#"{"type":"object","properties":{"caseId":{"type":"string","description":"测试用例 UUID"},"environmentId":{"type":"string","description":"可选，指定执行环境（缺省用项目默认环境）"}},"required":["caseId"]}"#,
    ),
    (
        "list_test_case_runs",
        r#"列出测试用例的执行历史（新→旧：时间/状态/耗时/断言通过数）。"#,
        r#"{"type":"object","properties":{"caseId":{"type":"string"},"limit":{"type":"number","description":"可选，1-200，默认 50"}},"required":["caseId"]}"#,
    ),
    (
        "get_test_case_run",
        r#"获取一次用例执行的完整详情（请求快照/响应/逐条断言/提取值）。"#,
        r#"{"type":"object","properties":{"runId":{"type":"string","description":"执行历史 UUID（run_test_case 或 list_test_case_runs 返回）"}},"required":["runId"]}"#,
    ),
    (
        "analyze_run_failure",
        r#"分析场景运行的失败步骤：结合请求/响应/断言/提取情况，按规则分类错误类型（请求错误/断言失败/依赖阻断/取消）并给出可能原因与建议。输出为启发式推测而非确定结论，修复前请结合 get_step_result 的原始响应核对。"#,
        r#"{"type":"object","properties":{"runId":{"type":"string","description":"运行 UUID"}},"required":["runId"]}"#,
    ),
    (
        "cancel_run",
        r#"取消一个正在运行的场景（run_scenario wait=false 返回的 runId）。引擎在当前请求中止后返回已完成步骤（cancelled=true）并落历史；已结束的运行返回 already-finished（无需取消）。长场景跑错目标或用户要求停止时使用。"#,
        r#"{"type":"object","properties":{"runId":{"type":"string","description":"运行 UUID（run_scenario wait=false 返回）"}},"required":["runId"]}"#,
    ),
    (
        "agent_info",
        r#"返回 Miyo Agent 控制面连接信息（地址与令牌文件路径），供需要直接发 HTTP 的场景使用。"#,
        r#"{"type":"object","properties":{}}"#,
    ),
    (
        "get_context",
        r#"获取 Agent 上下文快照（一次性了解工作现场，避免盲目枚举）：当前激活项目（名称 / 描述 / 变量——敏感值脱敏）、当前激活环境（Base URL 与启用变量，敏感值脱敏）、项目接口清单（id / 方法 / 路径，不含请求规格明细）、环境清单、场景清单、测试用例数量、最近一次场景运行摘要。需要某个接口的完整定义时再用 get_endpoint 按需拉取。"#,
        r#"{"type":"object","properties":{}}"#,
    ),
    // ---- AI 测试中心：分析 / 生成类工具（只读，产出草稿不落库）----
    (
        "analyze_api_dependencies",
        r#"分析项目内接口的业务依赖关系：识别「A 接口响应字段 → B 接口请求参数」的数据流。证据分两层：场景提取链（用户已配置的 extract → {{var}} 引用，置信度 high）与响应字段同名匹配（响应示例 / Mock 模板字段 vs 请求参数，置信度 medium）。返回节点（每个接口的产出 / 消费字段与 {{var}} 引用）、依赖边（via 变量、置信度、证据）与未解析变量清单。设计集成测试场景、确定步骤顺序与提取规则前先调用它。"#,
        r#"{"type":"object","properties":{"projectId":{"type":"string","description":"项目 UUID；缺省时仅一个项目自动选中"}}}"#,
    ),
    (
        "analyze_project_test_coverage",
        r#"分析项目测试覆盖率（七个维度）：无用例接口、无成功运行、无异常（负向/安全性）用例、有必填参数但无边界值用例、缺响应体断言、写接口缺幂等/重复请求测试、未被任何场景覆盖。返回 0-100 评分、逐维度命中数、每个接口的问题清单与中文总评。制定补测计划时使用。"#,
        r#"{"type":"object","properties":{"projectId":{"type":"string","description":"项目 UUID；缺省时仅一个项目自动选中"}}}"#,
    ),
    (
        "generate_test_cases",
        r#"为单个接口生成测试用例草稿（只返回草稿，不落库；审阅修改后用 create_test_case 保存）：正常请求、缺少必填参数、错误类型、边界值（空串/超长/零值）、非法参数（畸形 JSON/注入探测）、权限不足（去认证）、资源不存在（改写 id）、重复请求、幂等键重放。每条含存库分类（正向/负向/边界值/安全性/其他）、请求快照与建议断言。生成后逐条向用户说明再保存。"#,
        r#"{"type":"object","properties":{"endpointId":{"type":"string","description":"接口 UUID（list_endpoints 返回）"}},"required":["endpointId"]}"#,
    ),
    (
        "generate_integration_scenario",
        r#"把步骤意图物化为集成测试场景草稿（只返回草稿，不落库；审阅后用 create_scenario 保存）。每步给 endpointId（或内联 method+url+body）+ bindings（请求字段名 → 值模板，如 {"orderNo": "{{orderNo}}"}）+ extract + assertions。物化器会：校验接口存在、bindings 翻译为步骤覆盖、按依赖分析自动补缺失的提取规则、产出变量依赖图与写操作风险清单。设计前先调 analyze_api_dependencies 确定步骤顺序。返回的 scenario 可直接传给 create_scenario 的 steps。"#,
        r#"{"type":"object","properties":{"projectId":{"type":"string","description":"项目 UUID；缺省时仅一个项目自动选中"},"name":{"type":"string","description":"场景名称"},"description":{"type":"string","description":"可选，场景说明"},"category":{"type":"string","description":"可选，分类标签"},"steps":{"type":"array","description":"步骤意图数组（顺序即执行顺序）","items":{"type":"object","properties":{"endpointId":{"type":"string","description":"接口 UUID（与内联二选一）"},"method":{"type":"string","description":"内联请求方法（无 endpointId 时必填）"},"url":{"type":"string","description":"内联请求 URL 模板，支持 {{var}}（无 endpointId 时必填）"},"body":{"type":"string","description":"可选，内联 JSON Body 原文"},"name":{"type":"string","description":"可选，步骤名"},"bindings":{"type":"object","description":"可选，变量绑定：请求字段名 → 值模板","additionalProperties":{"type":"string"}},"extract":{"type":"array","description":"可选，提取规则 [{\"name\":\"orderNo\",\"path\":\"$.data.orderNo\"}]"},"assertions":{"type":"array","description":"可选，断言 [{\"type\":\"status\",\"op\":\"eq\",\"expected\":200}] 或 [{\"type\":\"jsonpath\",\"path\":\"$.status\",\"op\":\"eq\",\"expected\":\"PAID\"}]"},"waitMs":{"type":"number","description":"可选，执行前等待毫秒"},"condition":{"type":"object","description":"可选，执行条件"}}}}},"required":["name","steps"]}"#,
    ),
    (
        "propose_scenario_fix",
        r#"基于一次失败运行为场景生成修复草稿（只返回草稿与差异说明，不落库；审阅后用 update_scenario 保存）。流程：读取运行分析定位失败步骤 → 结合场景当前步骤配置生成修复意图（改参数 / 补提取 / 改 JSONPath / 调断言 / 补步骤依赖）→ 返回修改前后的字段级对照与风险评估。对写操作的调整会标注风险，需向用户确认后再保存。"#,
        r#"{"type":"object","properties":{"runId":{"type":"string","description":"运行 UUID（list_scenarios → run_scenario / get_run_result 获得）"},"notes":{"type":"string","description":"可选，用户的修复要求（如「只修断言，不动请求参数」）"}},"required":["runId"]}"#,
    ),
];

/// 默认启用的权限等级（未配置 settings 时）：read + write + execute；
/// network_execute / destructive 为高风险，必须用户明确确认后才启用。
pub const DEFAULT_ENABLED_LEVELS: [PermissionLevel; 3] = [
    PermissionLevel::Read,
    PermissionLevel::Write,
    PermissionLevel::Execute,
];

/// 解析 settings 表中的启用等级 JSON（None / 解析失败 → 默认集；
/// 未知等级忽略——旧版本键内容前向兼容）。
pub fn parse_enabled_levels(raw: Option<&str>) -> Vec<PermissionLevel> {
    match raw {
        None => DEFAULT_ENABLED_LEVELS.to_vec(),
        Some(json) => {
            let levels: Vec<PermissionLevel> = serde_json::from_str::<Vec<String>>(json)
                .unwrap_or_default()
                .iter()
                .filter_map(|s| PermissionLevel::parse(s))
                .collect();
            if levels.is_empty() {
                // 显式空数组 = 用户全关（含 read）——合法配置，不回退默认集。
                // 但解析失败（非数组）时 from_str 也给空——区分：原文非空但
                // 解析失败视为损坏，回退默认集。
                if json.trim() == "[]" {
                    Vec::new()
                } else {
                    DEFAULT_ENABLED_LEVELS.to_vec()
                }
            } else {
                levels
            }
        }
    }
}

/// 按名查工具元数据。
pub fn tool_meta(name: &str) -> Option<&'static ToolMeta> {
    TOOL_META.iter().find(|t| t.name == name)
}

/// 按名查描述 + Schema（未注册返回 None）。
pub fn tool_schema(name: &str) -> Option<(&'static str, Value)> {
    TOOL_SCHEMA
        .iter()
        .find(|(n, _, _)| *n == name)
        .map(|(_, desc, schema)| {
            let parsed: Value =
                serde_json::from_str(schema).unwrap_or_else(|_| json!({ "type": "object" }));
            (*desc, parsed)
        })
}

/// 全部工具元数据（tools/list / 设置页 / 完整性测试共用）。
pub fn all_tool_meta() -> &'static [ToolMeta] {
    TOOL_META
}

/// `tools/list` 的完整定义：MCP 标准字段（name / description / inputSchema /
/// annotations）+ `x-miyo` 扩展（module / permission / timeoutMs / concurrent /
/// sideEffects），单一来源即本注册表。
pub fn tool_definitions_json() -> Vec<Value> {
    TOOL_META
        .iter()
        .filter_map(|meta| {
            let (description, input_schema) = tool_schema(meta.name)?;
            Some(json!({
                "name": meta.name,
                "description": description,
                "inputSchema": input_schema,
                "annotations": {
                    "title": meta.title,
                    "readOnlyHint": meta.read_only,
                    "destructiveHint": meta.destructive,
                    "idempotentHint": meta.idempotent,
                    "openWorldHint": meta.open_world,
                },
                "x-miyo": {
                    "module": meta.module.as_str(),
                    "permission": meta.permission.as_str(),
                    "timeoutMs": meta.timeout_ms,
                    "concurrent": meta.concurrent,
                    "sideEffects": meta.side_effects,
                },
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 名称唯一、Schema 可解析且为 object、元数据自洽（只读 ↔ 无副作用等）。
    #[test]
    fn registry_is_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for meta in TOOL_META {
            assert!(seen.insert(meta.name), "工具名重复：{}", meta.name);
            let (_, schema) =
                tool_schema(meta.name).unwrap_or_else(|| panic!("{} 缺 Schema", meta.name));
            assert_eq!(
                schema["type"], "object",
                "{} Schema 顶层必须是 object",
                meta.name
            );
            // 元数据自洽
            assert_eq!(
                meta.destructive,
                meta.permission == PermissionLevel::Destructive,
                "{} 的 destructive 标记须与权限等级一致",
                meta.name
            );
            if meta.read_only {
                assert!(!meta.side_effects, "{} 只读不应有副作用", meta.name);
            }
            if meta.open_world {
                assert!(
                    meta.permission == PermissionLevel::NetworkExecute
                        || meta.permission == PermissionLevel::Execute,
                    "{} 联网工具权限应为 network_execute / execute",
                    meta.name
                );
            }
        }
        // 描述 / Schema 表不得多出未注册项
        for (name, _, _) in TOOL_SCHEMA {
            assert!(tool_meta(name).is_some(), "Schema 表多出未注册工具：{name}");
        }
        assert_eq!(TOOL_META.len(), TOOL_SCHEMA.len());
    }

    /// 权限矩阵：高风险等级默认关闭的含义 —— 删除类必须是 Destructive、
    /// 自由网络请求必须是 NetworkExecute；Execute 与 NetworkExecute 分离。
    #[test]
    fn permission_matrix_is_risk_separated() {
        let level = |n: &str| tool_meta(n).unwrap().permission;
        assert_eq!(level("send_endpoint"), PermissionLevel::NetworkExecute);
        assert_eq!(level("run_scenario"), PermissionLevel::Execute);
        assert_eq!(level("run_test_case"), PermissionLevel::Execute);
        assert_eq!(level("delete_scenario"), PermissionLevel::Destructive);
        assert_eq!(level("delete_test_case"), PermissionLevel::Destructive);
        assert_eq!(level("list_projects"), PermissionLevel::Read);
        assert_eq!(level("save_curl"), PermissionLevel::Write);
        // 只读清单不得包含执行 / 网络 / 破坏权限
        for meta in TOOL_META.iter().filter(|m| m.read_only) {
            assert_eq!(meta.permission, PermissionLevel::Read, "{}", meta.name);
        }
    }

    /// tools/list 生成：字段齐全、注解与元数据一致。
    #[test]
    fn tool_definitions_json_matches_meta() {
        let defs = tool_definitions_json();
        assert_eq!(defs.len(), TOOL_META.len());
        for d in &defs {
            let name = d["name"].as_str().unwrap();
            let meta = tool_meta(name).unwrap();
            assert!(
                d["description"].as_str().unwrap().len() > 10,
                "{name} 描述过短"
            );
            assert_eq!(d["annotations"]["readOnlyHint"], meta.read_only);
            assert_eq!(d["annotations"]["destructiveHint"], meta.destructive);
            assert_eq!(d["x-miyo"]["permission"], meta.permission.as_str());
            assert_eq!(d["x-miyo"]["timeoutMs"], meta.timeout_ms);
            assert_eq!(d["x-miyo"]["concurrent"], meta.concurrent);
        }
    }
}
