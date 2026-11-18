# Miyo 插件系统（Core + Plugin 架构）

> **2.0 已落地（2026-11）**：全文件完整性清单 + Ed25519 官方签名 keyring、
> 权限细分后端强制（java.* 子权限 + std 族会话凭证）、ui-sidecar 形态与
> Sidecar Runtime Supervisor、本地包更新 / 回滚 / 依赖解析、市场静态索引。
> 2.0 详细文档见 **docs/plugin/**（AUDIT / ARCHITECTURE / MANIFEST / SDK /
> PERMISSIONS / SIGNING / SIDECAR / MARKETPLACE / DEV-GUIDE / TESTING）；
> 本文保留 v1 架构与协议基础描述。

> 状态：已实现并全量测试通过（2026-11）。本文档是插件系统的设计规范与开发指南，
> 也是交付物清单的索引：架构分析 / 设计 / 目录结构 / SDK / Manifest / Java 插件迁移 /
> 安全设计 / 性能 / 测试结果 / 第三方插件开发指南。

---

## 1. 当前架构分析（插件化前）

Miyo 是 **Tauri 2 + Vue 3** 的跨平台调试工作台，三层结构：

| 层 | 形态 | 位置 |
|---|---|---|
| Layer 1 前端 | Vue 3 + Pinia + vue-router（全部视图懒加载） | `frontend/src/` |
| Layer 2 命令层 | Tauri 2 插件 `miyo`（229 个 command，`plugin:miyo|*`；2.0 新增会话/资产读/更新回滚/市场/Sidecar 等 21 条） | `crates/miyo-tauri/`（独立 workspace） |
| Layer 3 领域层 | 纯库 crate（miyo-core / storage / http / java / …） | `crates/` |

关键事实（决定了插件系统的形态）：

- **前端已有 `domains/` 分层**：`java-debug` 本来就是独立 domain（视图 / store / utils
  / spec 全部内聚），抽离成本低；
- **路由全部懒加载**（`router/index.ts` 动态 import），且构建分包已把重型依赖拆
  vendor chunk——插件按需加载有现成基建；
- **Java 调试天然是"进程外插件"形态**：Java Debug Runtime（`java/miyo-java-runtime`）
  是嵌入目标 JVM 的 HTTP 服务（协议 v1 + Bearer Token），Miyo 主进程**从不执行任何
  Java 代码**——这满足"不在主进程执行第三方代码"的硬约束；
- **Java 已有的安全机制**：Runtime 侧显式白名单（空白名单拒绝启动）、危险类黑名单
  （两端双保险）、token 常数时间比较 + AES-GCM 落库、1 MiB 体积上限、300s 超时上限、
  并发 4 / 每分钟 60 限流、调用审计（`java_invocations` 滚动 200 条）、MCP 工具
  NetworkExecute 高风险门；
- **耦合点清单**（改造时切断的硬编码）：前端 3 处导航（DashboardNav / WorkspaceView
  工具组 / 溢出菜单）+ 路由 + `useMiyoApi` 14 个方法 + i18n；后端 14 个 command + 场景
  引擎 hook 注入（3 处）+ `miyo-agent::java_hook` 单源执行器 + MCP 9 个工具；
- **约束**：`miyo-tauri` 有三向命令契约测试（lib.rs ↔ build.rs COMMANDS ↔
  permissions/default.toml 源码扫描），且 `#[cfg]` 不能放进 `generate_handler!` 宏
  参数——**编译期裁剪 Java 代码不可行**（详见 §7.6），因此采用运行时门禁方案。

## 2. 插件架构设计

```text
┌─────────────────────────── WebView（前端） ───────────────────────────┐
│  PluginManager（pinia，生命周期编排 + 注册表 + 指标）                    │
│    ├─ builtinModules   内置插件表：id → 懒加载工厂（未启用绝不 import）  │
│    ├─ PluginRegistry   entries：PluginView + runtime 状态 + 指标       │
│    ├─ hostBridge       导航 / 路由注册（vue-router 动态 addRoute）      │
│    ├─ PluginSDK        createPluginSession → ctx（权限门禁在 SDK 层）   │
│    └─ host/            第三方插件沙箱宿主：sandbox iframe + RPC 桥      │
│  DashboardNav / WorkspaceView 工具组：渲染 activeNavItems（响应式）     │
│  PluginsView（/plugins）：市场 / 已安装 / 详情 / 权限 / 配置 / 日志      │
└──────────────────────────────┬───────────────────────────────────────┘
                               │ Tauri IPC（plugin_* 19 个命令 + 门禁错误码）
┌──────────────────────────────▼───────────────────────────────────────┐
│  miyo-tauri commands/plugins.rs（安装/启停/卸载/更新/权限/日志/存储）    │
│    ├─ 插件门禁：java_* 14 命令按插件状态拒绝（PLUGIN_NOT_INSTALLED /    │
│    │   PLUGIN_DISABLED）；场景 hook 注入按状态开关                      │
│    └─ miyo-agent::java_hook：场景 / Agent 控制面 / MCP 的单源兜底门禁   │
┌──────────────────────────────┬───────────────────────────────────────┐
│  miyo-plugin（新 crate，纯逻辑）│ miyo-storage repository/plugins        │
│  manifest 严格校验（真源）      │ plugins / plugin_data / plugin_logs     │
│  compat 版本兼容               │ 状态机跃迁校验 + 行级数据隔离            │
│  permission 权限目录（封闭集）  │ 日志滚动保留 200 条/插件                │
│  package .miyo 解包/完整性/签名 │                                       │
│  builtin 内置清单（include_str）│                                       │
└──────────────────────────────┴───────────────────────────────────────┘
```

### 生命周期状态机

```text
（市场）─install→ installed ─enable→ enabled ─activate→ 运行时 active
                     ↑                  │  │                    │
                     │        deactivate│  │activate 失败        │deactivate
                     └─────── disable ──┘  └→ error ──重试──────┘
（卸载）uninstall：删除注册行（级联清 plugin_data），运行时先 deactivate
（更新）update：manifest/版本替换；启用态的插件自动重激活
```

- 持久化状态：`installed / enabled / error`（`plugins.state`，跃迁合法性在
  repository 层强制校验）；
- 运行时状态：`inactive / activating / active / error`（仅前端内存）；
- 加载失败恢复：`error` 态保留 `error_json` 现场，管理中心提供「恢复」按钮
  （`retryActivate`）。

## 3. 核心模块目录结构

```text
crates/miyo-plugin/                 # 新 crate（根 workspace 成员）
├── builtins/java-debug/manifest.json   # 内置清单（唯一事实源，include_str! 嵌入）
├── src/
│   ├── manifest.rs                 # PluginManifest + 严格校验（deny_unknown_fields）
│   ├── compat.rs                   # x.y.z 逐段比较 + SDK 协议版本检查
│   ├── permission.rs               # 权限目录（封闭集合）+ 校验
│   ├── package.rs                  # .miyo 解包 / sha256 完整性 / ed25519 签名 / 穿越防护
│   ├── builtin.rs                  # 内置插件目录 + 一致性自检
│   └── lib.rs
crates/miyo-storage/
├── migrations/0031_plugins.sql     # plugins / plugin_data / plugin_logs 三表
└── src/repository/plugins.rs       # 状态机 / 行级隔离 KV / 日志滚动
crates/miyo-tauri/src/
├── commands/plugins.rs             # 19 个插件命令 + IPC view（ts-rs 导出）
├── commands/java_debug.rs          # 14 命令加插件门禁 + 调用审计
└── error.rs                        # + PLUGIN_NOT_INSTALLED / PLUGIN_DISABLED / PLUGIN_INCOMPATIBLE
frontend/src/plugins/               # 前端插件系统（平台层）
├── types.ts                        # IPC 类型 re-export（generated/）+ SDK 接口
├── permissions.ts                  # 权限目录镜像 + 命令→权限映射表
├── compatibility.ts                # 版本比较（UX 预检；后端为强制真源）
├── events.ts                       # 插件事件总线（命名空间隔离 + dispose）
├── sdk.ts                          # createPluginSession：受控上下文 + 权限门禁
├── manager.ts                      # PluginManager（pinia）：生命周期编排 + 注册表
├── builtinModules.ts               # 内置插件懒加载表
├── host/                           # 第三方插件沙箱宿主
│   ├── protocol.ts                 # RPC 帧协议（白名单方法 + 参数校验）
│   ├── bridge.ts                   # 宿主 RPC 桥（→ SDK 会话）
│   ├── PluginIframeHost.vue        # sandbox iframe 宿主页面
│   └── bridge.spec.ts
frontend/src/domains/plugins/       # 插件管理中心 UI
├── views/PluginsView.vue           # /plugins（市场 / 已安装 / 安装确认）
└── components/                     # 详情抽屉 / 配置表单
frontend/src/domains/java-debug/plugin.ts   # Java 插件入口（PluginFactory）
examples/plugins/hello-world/       # 第三方插件最小示例（manifest + index.html）
scripts/pack_plugin.sh              # 目录 → .miyo（tar.gz）打包
docs/PLUGIN-SYSTEM.md               # 本文档
```

## 4. Manifest 规范

唯一事实源：Rust `miyo-plugin::manifest`（`deny_unknown_fields` 严格校验）；前端
类型经 ts-rs 导出到 `frontend/src/types/generated/PluginManifest.ts`（CI bindings
diff 防漂移）。

完整字段（JSON，字段名 snake_case）：

```jsonc
{
  "id": "com.example.hello-world",       // reverse-DNS，2-4 段小写 [a-z0-9-]
  "name": "Hello World",                 // ≤ 96 字符
  "name_key": "plugins.xxx",             // 可选：i18n 键（内置插件用，展示优先）
  "description": "……",                    // ≤ 512 字符
  "description_key": "plugins.xxxDesc",  // 可选
  "version": "0.1.0",                    // 严格 x.y.z（0-999，无预发布/元数据）
  "authors": ["Miyo"],                   // ≤ 10 项
  "icon": "package",                     // IconName 或 data:image/*（≤ 16 KiB）
  "entry": "index.html",                 // 第三方 UI 插件必填；禁绝对路径 / .. / 非 .html
  "kind": "ui",                          // ui | ui-sidecar（sidecar 协议预留，校验拒绝）
  "min_app_version": "0.0.60",           // 最低宿主版本
  "sdk_version": 1,                      // SDK 协议版本（当前 1，不支持更高）
  "nav": [                               // ≤ 8 项；route 唯一、/ 开头、无穿越段
    { "id": "main", "label_key": "pnav.javaDebug", "label": "Hello 插件",
      "icon": "package", "route": "/java-debug", "order": 40 }
  ],
  "permissions": ["std.storage", "java.debug"],  // ≤ 16 项；必须命中权限目录（未知名拒绝）
  "runtime_deps": [ { "kind": "java", "min_version": "11", "description": "…" } ],
  "plugin_deps": { "com.other.plug": "1.2.0" },  // ≤ 8 项（v1 要求存在 + 启用）
  "config_schema": {                     // JSON Schema 严格子集（≤ 32 键）
    "properties": {
      "audit_log": { "type": "boolean", "title": "…", "default": true }
    },
    "required": []
  },
  "integrity": "<64 位 hex，sha256(manifest.json 字节)>",   // 可选；不符即视为篡改
  "signature": "<128 位 hex，ed25519(manifest 字节)>",       // 可选；官方公钥校验
}
```

校验规则要点（全部在 `PluginManifest::parse` 强制）：

1. 未知字段拒绝（`deny_unknown_fields`）——未来字段不构成绕过通道；
2. `permissions` 逐条命中 `miyo-plugin::permission` 目录，未知名 / 重复 / 超量拒绝
   （**不可信插件无法通过声明未知权限拿到能力**）；
3. `id` / `entry` / `nav[].route` 全部做穿越与字符白名单校验；
4. `sdk_version` 高于宿主支持版本即拒绝；
5. 清单总大小 ≤ 64 KiB。

### 权限目录（封闭集合）

| 权限 | 风险 | 能力 |
|---|---|---|
| `std.storage` | low | 插件私有 KV 存储（按 plugin_id 行级隔离） |
| `std.log` | low | 统一日志（plugin_logs，审计条目可见） |
| `std.notify` | low | 应用内 Toast 通知 |
| `std.events` | low | 插件事件总线（命名空间隔离） |
| `std.clipboard.write` | low | 写系统剪贴板 |
| `std.env.read` | medium | 读环境变量合并表（可能含敏感值） |
| `java.debug` | high | 连接并调用 Java Debug Runtime（方法发现与执行） |

## 5. 插件 SDK 接口定义

`createPluginSession(view, hostBridge)` 返回 `{ context, dispose }`；插件的
`activate(ctx)` 只见到 `context`：

```ts
interface PluginContext {
  readonly manifest: PluginManifest
  storage: PluginStorage        // get/set/delete/keys —— 后端按 plugin_id 隔离
  logger: PluginLogger          // debug/info/warn/error/audit —— 落 plugin_logs
  notify: PluginNotify          // info/success/warn/error —— Toast
  events: PluginEvents          // emit/on —— 命名空间 `${pluginId}:${event}`
  env: PluginEnvAccess          // getVariableMap() —— 环境变量合并表（只读）
  commands: PluginCommands      // invoke(cmd, args) —— 白名单 + 权限映射
  config: PluginConfig          // get/set —— 存私有存储 __config 键
  clipboard: PluginClipboard    // writeText
  ui: PluginUiRegistry          // nav.register/unregister + route.register/unregister
}
interface PluginInstance { deactivate?(): void | Promise<void> }
type PluginFactory = (ctx: PluginContext) => PluginInstance | Promise<PluginInstance>
```

边界强制（SDK 层抛错，后端门禁兜底）：

- `commands.invoke` 只放行「命令 → 权限」映射表内命令（当前：`java.*` 14 命令 →
  `java.debug`；`list_environments` / `get_projects` → `std.env.read`），且权限必须在
  用户批准快照内；未登记命令一律 `PLUGIN_COMMAND_DENIED`；
- `env / notify / events / clipboard` 分别要求对应权限，越权抛
  `PLUGIN_PERMISSION_DENIED`；
- 插件不能直接访问主程序内部状态（workspace store 不进 SDK 面）与其他插件私有数据
  （存储按 plugin_id 行级隔离，键名白名单校验）；
- `dispose()` 由宿主在 deactivate 时调用：解绑插件注册的全部事件监听。

### 后端命令面（19 个，`plugin:miyo|plugin_*`）

`plugin_list_installed / plugin_list_catalog / plugin_get / plugin_inspect_package /
plugin_install_from_path / plugin_install_builtin / plugin_uninstall / plugin_set_enabled /
plugin_update_builtin / plugin_set_permissions / plugin_report_error / plugin_verify_integrity /
plugin_logs / plugin_clear_logs / plugin_record_log / plugin_data_get / plugin_data_set /
plugin_data_delete / plugin_data_keys`

## 6. Java 插件迁移结果

| 项 | 结果 |
|---|---|
| 清单 | `crates/miyo-plugin/builtins/java-debug/manifest.json`（id `com.miyo.java-debug`，v0.1.0，6 项权限，nav 指向 `/java-debug`，runtime_deps 声明 Java 11+，config_schema 带 audit_log 开关） |
| 前端 | `domains/java-debug/plugin.ts`：activate 注册导航 + 路由（懒加载 JavaDebugView）；deactivate 调 javaDebug store 的 `dispose()`（停心跳 / 清在途态 / 清发现缓存） |
| 导航 | DashboardNav / WorkspaceView 工具组 / 溢出菜单全部改为渲染 `manager.activeNavItems`（原 3 处硬编码移除） |
| 后端命令 | 14 个 `java_*` command 全部加 `ensure_java_plugin` 门禁（未安装 → `PLUGIN_NOT_INSTALLED`；已装未启用 → `PLUGIN_DISABLED`） |
| 场景引擎 | `scenarios.rs` 三处 hook 注入按插件状态开关（未启用注入 None → runner 报「Java 调试插件未启用」，其余步骤照常） |
| Agent / MCP | `miyo-agent::java_hook`（场景 + 控制面 + MCP 单源）执行前查启用态——兜底门禁 |
| 审计 | `java_invoke` 成功后写 `audit` 级插件日志（方法 id / 状态 / 耗时，**不含参数原文**；受 config `audit_log` 开关控制），详细历史仍按原口径落 `java_invocations` |
| 卸载影响 | 核心功能零影响（门禁只是状态查询；java_* 命令拒绝不抛崩溃；场景 Java 步骤显式失败可读） |

**未改变的事实（有意保留）**：Java Runtime 仍由用户嵌入目标应用（`io.miyo.debug`
SDK），方法白名单在目标应用侧显式注册——这是比「Miyo 分发可执行代码」更强的
安全模型，迁移不改变该边界。

## 7. 插件安全设计

### 7.1 权限最小化

- 权限封闭目录（§4），声明即校验；启用时展示权限清单，用户批准集
  （`granted_permissions` ⊆ `manifest.permissions`）落库，收权立即生效（SDK 调用
  现场被拒）；
- 高危权限（`java.debug`）在 UI 上以 danger 徽章单独标示。

### 7.2 文件 / 路径安全（本地包安装）

- `.miyo` = tar.gz；解包防护：仅常规文件 + 目录条目，拒绝符号链接 / 硬链接 /
  设备条目；条目名白名单 `[A-Za-z0-9._- ]`，`..` / 绝对路径 / 反斜杠拒绝；条目数
  ≤ 200、单文件 ≤ 10 MiB、总量 ≤ 64 MiB；落盘路径相对 `{data_dir}/plugins/<id>/`；
  卸载只删除受管目录内的内容（install_path 指向开发目录时不删）。

### 7.3 完整性与签名

- `integrity` = sha256(manifest.json 字节)，安装时计算落库，`plugin_verify_integrity`
  可复核（包被篡改 → 明确报错要求重装）；
- `signature` = ed25519（对 manifest 字节），官方公钥编译期内置
  （`OFFICIAL_SIGNING_PUBKEY_HEX`）；**发布流水线接入前为占位零值**——此时任何
  签名都校验失败，本地包一律走「未签名 + 用户显式信任勾选」通道；接入后替换公钥
  即可，校验代码零改动。这是诚实声明：签名验证代码是真实实现，官方签名流水线
  （CI 生成密钥对 + release 签名）是后续工作。

### 7.4 第三方代码隔离（不在主进程执行不可信代码）

第三方 UI 插件运行在 **`sandbox="allow-scripts"` iframe（不带 allow-same-origin）**：

- opaque origin：插件 JS 无法访问主程序 DOM / localStorage / cookie / 父窗口对象；
- 唯一通道是 postMessage RPC（`host/protocol.ts`）：方法白名单 + 参数形状校验 +
  未知帧静默丢弃；桥校验消息源窗口 === 宿主 iframe 的 contentWindow；
- 权限在 SDK 会话层强制（与内置插件同一套门禁）；
- 插件页面文件经 `read_text_file` 读入 → Blob URL 加载（CSP `frame-src` 已放行
  `blob:`），目录内容只读；
- **Worker 不是安全沙箱**（同 origin、可访问部分宿主 API）——本项目不用 Worker
  做隔离，也不用 `import()` 把第三方代码拉进主 WebView。
- 已知边界（诚实声明）：沙箱 iframe 与主 WebView 同进程（WKWebView/Chromium 的
  site isolation 粒度），进程级强隔离需 sidecar 进程（协议已预留 `ui-sidecar`
  形态，校验层当前拒绝该 kind，实现为后续阶段）。

### 7.5 敏感信息治理

- Java token AES-256-GCM 落库（既有机制，未改动）；插件日志不允许参数原文——
  java_invoke 审计只记录方法 id / 状态 / 耗时；展示层沿用 `mask.ts` 脱敏；
- `std.env.read` 定为 medium 风险（合并表可能含敏感变量值），按权限治理而非硬禁。

### 7.6 为什么不做编译期裁剪（诚实说明）

需求「Java 依赖不进 Core 默认依赖」在 Rust 静态编译下的理想形态是 cargo feature
裁剪。调研结论：**当前不可行**——`tauri::generate_handler!` 宏参数不接受
`#[cfg]` 属性（编译错误），而 miyo-tauri 的三向命令契约测试（lib.rs ↔ build.rs ↔
default.toml 源码扫描）正是为防命令清单漂移设计的，绕开它等于拆掉防线。因此采用：

- **运行时门禁**（本方案）：未安装 / 停用 → 14 个命令全部拒绝、场景 hook 不注入、
  MCP 调用拒绝、UI 零加载。行为上满足验收标准（卸载后核心不受影响）；
- miyo-core / miyo-storage（核心领域层）本就**不依赖** miyo-java（依赖方向是
  miyo-java → miyo-core），"Core 默认依赖不含 Java"在领域层成立；
- 后续若 Tauri 支持条件化 handler 注册，可在 `plugin-java` feature 下做编译期裁剪
  （预留设计，见 §10）。

### 7.7 危险操作保护（Java 方法执行）

既有机制（保留并接入插件门禁）：目标应用侧显式白名单（空白名单拒绝启动）、危险
类黑名单两端双保险、300s 超时上限 + 协作式中断、并发 4 / 每分钟 60 限流、请求
1 MiB / 结果 1 MiB 上限、每次调用落审计历史；前端危险方法名（删除 / 支付 / 改单
等）二次确认（JavaMethodPanel 既有逻辑）。新增：插件停用后全部入口即时失效 +
audit 级插件日志。

## 8. 性能设计与结果

**按需加载策略**（构建产物已验证）：

- 插件模块（`builtinModules`）只在 activate 时动态 import；未安装 / 停用的插件
  **零字节加载**；
- 插件视图组件再懒一层（`JavaDebugView` 独立 chunk，导航进入才拉取）；
- 产物 grep 验证：`dist/assets/index-*.js`（主 chunk）不含 JavaDebugView /
  PluginsView 代码；`JavaDebugView-*.js` / `PluginsView-*.js` /
  `PluginIframeHost-*.js` / `javaDebug-*.js`（插件入口模块）均为独立 chunk；
- SDK 的 env 访问动态 import workspace store——不把工作区状态机拖进首屏链；
- 启动路径：`main.ts` 的 `manager.boot()` 是 fire-and-forget，不阻塞首屏挂载；
- **避免重复加载**：activate 幂等（active/activating 直接返回）；`loadLocale` /
  `builtinModules` 同 promise 共享（既有基建）；
- **依赖冲突**：插件不携带自己的 JS 依赖进主 WebView（第三方插件自持 HTML/JS 于
  iframe 内）；插件间依赖 `plugin_deps` 声明 + 启用态校验（v1 不做 semver 区间求解，
  声明为后续扩展）；
- **监控**：activate 计时（performance.now）记录 `lastActivateMs / activateCount /
  lastActivatedAt` 到 PluginEntry，管理中心「概览」页展示（Java 插件首激活
  ~个位数 ms，纯注册开销）。

## 9. 自动化测试结果

| 套件 | 结果 |
|---|---|
| **Rust `cargo test --workspace`**（根 workspace，55 个套件） | 全部通过；新增 miyo-plugin 25（清单校验 / 穿越 / 签名 / 兼容 / 权限目录 / 内置自洽）+ storage plugins 5（状态机 / 门禁错误分型 / 数据隔离 / 键规则 / 日志滚动）；场景引擎 Java 步骤断言随门禁文案更新 |
| **Rust `cargo test`（miyo-tauri，独立 workspace）** | 46 通过（43 命令层 + 2 权限契约 + 1 ts 绑定）；19 个新命令过三向契约（lib.rs ↔ build.rs ↔ default.toml）；`--features ts` 导出 PluginManifest 全套 TS 绑定 |
| **Rust `cargo clippy --workspace --all-targets`** | 0 警告 |
| **Rust `cargo fmt --all -- --check`** | 通过 |
| **前端 `vitest run`** | **148 文件 / 1194 测试全绿**；新增 41：manager 生命周期 7（激活注册 / 停用回收 / 卸载 / 失败上报 / 兼容拒绝 / 路由冲突）、SDK 12（命令白名单 / 权限门禁 / 存储作用域 / config 合并 / 事件命名空间 / dispose）、RPC 桥 11（帧校验 / 白名单 / 超时 / 权限传播 / evt 推送）、Java 插件独立运行 4（全生命周期 / store 资源回收 / 重激活 / 卸载后路由消失）、核心无插件回归 4（导航无插件项 / 核心路由全可达 / 安装后入口出现 / 卸载复原）、兼容性 3 |
| **验收标准断言**（`coreNoPlugins.spec.ts`） | ① 未装插件：导航只含核心项、`/java-debug` 不可解析、核心路由全可达 ② 安装 Java 插件：导航入口出现、路由可达、视图 chunk 可加载 ③ 卸载：导航 / 路由复原、核心零影响 |
| **前端 `eslint .` / `vue-tsc --noEmit` / `vite build`** | 全部干净 |
| **i18n 门禁**（i18nPattern / locale 键集 1:1） | 通过（新增 119 个 `plugins.*` 键中英对齐；错误信息走 `tFallback`） |
| Java SDK `SelfTest` / demo E2E | 未改动（miyo-java crate 本次零修改） |

## 10. 第三方插件开发指南

### 10.1 最小插件（三步）

```bash
mkdir my-plugin && cd my-plugin
# 1. 写 manifest.json（字段规范见 §4；第三方用 label 字面量 + entry: "index.html"）
# 2. 写 index.html：自持样式 + 一段 RPC 客户端（复制 examples/plugins/hello-world/
#    index.html 里的 rpc() 函数即可，~20 行）
# 3. 打包安装
../../scripts/pack_plugin.sh .          # 产出 my-plugin.miyo
# 插件中心 → 右上「从本地 .miyo 包安装」→ 检视确认（权限清单 + 未签名信任勾选）
```

### 10.2 RPC 方法面（`window.parent.postMessage`，帧协议见 `host/protocol.ts`）

| 方法 | 参数 | 能力（需对应权限） |
|---|---|---|
| `storage.get/set/delete/keys` | `{key}` / `{key,value}` | 私有存储（std.storage） |
| `log.write` | `{level,message}`（audit 级用于高危动作） | 日志（std.log） |
| `notify.show` | `{kind,title,message?}` | 通知（std.notify） |
| `events.emit/subscribe` | `{event,payload?}` / `{event}` | 事件总线（std.events） |
| `config.get/set` | 无 / `{values}` | 配置（std.storage 通道） |
| `env.map` | 无 | 环境变量合并表（std.env.read） |
| `commands.invoke` | `{command,args}` | 受控主程序命令（按命令映射权限） |

### 10.3 规则与红线

- manifest 权限声明必须是目录内的已知名（未知即安装失败）；
- 不要试图访问 `window.parent` 的任何对象（sandbox 阻断，仅 postMessage 可达）；
- 不要在日志输出敏感信息（token / 密钥）——管理中心日志页对所有插件日志可见；
- 内置插件（同仓 Vue 模块）开发：在 `crates/miyo-plugin/builtins/<id>/manifest.json`
  落清单 → `frontend/src/plugins/builtinModules.ts` 补一行懒加载映射 → 前端类型经
  ts-rs 生成（跑 `cargo test -p miyo-tauri --features ts`）。

### 10.4 后续路线（2.0 已落地大半；剩余项见 docs/plugin/TESTING.md 已知限制）

1. **官方签名流水线**：CI 生成 ed25519 密钥对 → release 包签名 → 替换
   `OFFICIAL_SIGNING_PUBKEY_HEX`（校验代码零改动）；
2. **`ui-sidecar` 进程形态**：manifest.kind = ui-sidecar（校验层当前显式拒绝），
   sidecar 可执行 + JSON-RPC over stdio，用进程边界做强隔离（Java Runtime 客户端
   若需完全编译期剥离，可走此形态）；
3. **插件市场远端源**：目录 JSON + 签名校验（当前市场 = 内置官方目录）；
4. **插件依赖 semver 区间求解**（当前仅要求存在与启用）；
5. **miyo-tauri 支持条件化 handler 后的编译期裁剪**（§7.6）。

---

*测试基线：2026-11-17，miyo v0.0.60 主干。Rust 55 + 5 套件、前端 148 文件 1194 测试、
clippy/fmt/eslint/vue-tsc/build 全绿。*
