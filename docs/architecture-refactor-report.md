# Miyo 架构·性能·质量治理——交付对比报告

> 治理周期：2026-11-13 ~ 11-14，8 个主题提交（`b0df79b`..`c650909`）。
> 原则：不为重构而重构、不改变稳定业务行为、不为提升性能牺牲一致性。
> 每阶段结束跑全套门禁（cargo fmt/check/clippy/test + miyo-tauri 同标准 +
> 前端 vue-tsc/eslint/vitest/build），全绿才提交。

## 一、总览

| 维度 | 治理前 | 治理后 | 说明 |
|---|---|---|---|
| Rust 测试 | 651 通过 | **663 通过**（workspace） | +12（LoadSchedule 5 个调度模式测试、materialize 4 个契约测试、CommandError 2 个序列化契约测试、args 2 个 wire 契约测试；部分归并计数） |
| miyo-tauri 测试 | 40 | **42** | +2（错误码序列化契约） |
| 前端测试 | 121 文件 / 1007 | **121 文件 / 1007** | 迁移零回归；新增 i18n/theme 契约守门 |
| 前端源码（非 spec） | 77,407 行 | 78,658 行 | +1,251（功能增强 + 契约测试 + 文档注释） |
| Rust 源码 | 72,783 行 | 74,158 行 | +1,375（压测增强 + materialize + 迁移 0029） |
| 新增依赖 | — | **3 个**：ts-rs（可选 feature）、schemars（miyo-agent）、sysinfo（miyo-test） | 均有直接诉求支撑；前端零新增运行时依赖 |

## 二、分项成果

### 1. 类型一致性（防漂移机制从 0 → 1）
- **ts-rs 生成**：miyo-core 53 个领域模型 + miyo-tauri 关键 view DTO（CommandError /
  ExecuteResponse / ScenarioLibraryItem）+ miyo-http RequestSnapshot + miyo-test
  LoadConfig/LoadSchedule/LoadResult/LoadProgress，全部导出 `frontend/src/types/generated/`（62 文件）。
  miyoApi.d.ts 从 1575 行手写镜像改为 re-export（收缩到 ~1100 行）。
- **修复的漂移实锤**：前端 CommandError union 缺 `DATABASE_SCHEMA_TOO_NEW / FORBIDDEN /
  PORT_BIND / CANCELLED` 4 个码（前端已靠 as any 绕过）；`CANCELLED` 分支现由生成 union 覆盖。
- **错误码单源**：`CommandError.code` 从裸字符串改为 `CommandErrorCode` 枚举（32 码），
  serde 产物与历史字符串逐字一致（契约测试守护）；新增 `AppError::Sync` 专属码
  （原借道 VALIDATION）。
- **CI 防漂移**：bindings job 重新导出 + `git diff --exit-code`，改 Rust 模型不重新导出即红灯。
- **MCP Schema 试点**：`miyo-agent/args.rs` typed 入参结构体 derive JsonSchema，
  dispatch 用 `serde_json::from_value` 同源反序列化（get_endpoint / list_endpoints），
  完整性测试改为「手写表 + 生成表」双源计数。
- **JSON 透传口径**：方言字段（ScenarioStep 的 condition/loop/overrides 等）保留前端
  紧类型手写（生成侧为 JsonValue 宽类型）；`serde_json::Value` 字段 TS 侧对齐旧契约，
  规避递归 JsonValue 在 Vue UnwrapRef 下的 TS2589 深度实例化爆炸。

### 2. SQLite 性能
- **迁移 0029**（追加式，未动存量迁移）：
  - `request_histories(created_at DESC)`——救 `list_recent_activities` 跨项目全表排序；
  - `endpoints(project_id, updated_at)`——救仪表板 latest-endpoint 子查询；
  - `projects(team_id)`——救删团队计数全表扫。
- **用例历史列表瘦身**：`list_test_case_runs` 排除 `results_json`（每条最大 64KB×步数），
  tauri/agent 同切 summary；agent 侧逐行 parse+脱敏循环随之消除；前端查看/对比按
  runId 拉详情并缓存（详情命令/路由此前已存在）。
- **append_ai_message 原子化**：MAX(seq)+INSERT 合入单事务，防并发撞号。
- **审计结论**：test_runs 表为休眠表（仅 smoke 测试使用，无生产写入路径）——保留策略
  暂不投入；历史保留策略（每项目 500 条节流裁剪）已存在且健康。
- 有意不动：list_endpoints 全量加载 request_json（编辑器核心数据流）、历史 summary
  JSON（单条 ≤2KB 封顶）。

### 3. 压测引擎（miyo-test/src/load.rs，334 → 656 行）
- **四种调度模式**（LoadSchedule，serde tag=mode，TS 绑定生成）：
  Fixed 固定总量（原行为）/ FixedQps 恒定速率 / Ramp 匀速阶梯 / Duration 持续时长。
- **关键教训（已写入代码注释与提交）**：tokio `SemaphorePermit` 的 Drop 会归还许可——
  `acquire→send→drop` 的许可自我循环使限速完全失效（初版实测 7700 req/s vs 目标 50）。
  最终实现为全局槽位游标（Mutex<Instant> 预约下一可发时刻，间隔 1/当前 qps）。
- **错误率熔断**：子令牌 + 阈值触发中止，`aborted_reason=error_rate` 回传。
- **指标**：延迟直方图（10ms~10s 固定桶 + ∞ 桶，互斥计数守恒）；LoadProgress 增
  elapsed/rps（2s 滑窗）/error_rate；sysinfo 进程 CPU%/内存 MB 均值+峰值。
- **前端**：LoadTestPanel 模式选择 + 实时 RPS/错误率 + 直方图条形图 + 资源指标，
  i18n zh/en 1:1。
- **协议评估**：`docs/loadtest-protocol-assessment.md`——WS（VU 持久连接 + RTT 口径）、
  SSE（流建立 + 事件速率）、gRPC 流式（需流式客户端扩展）各自连接模型与实施拆解。

### 4. Rust 编排单源（IPC 层减厚）
- **materialize_step**（miyo-test/materialize.rs，新模块）：tauri scenarios.rs 的
  **3 份相同物化闭包**（单场景 / 矩阵并行 / 矩阵串行）+ agent server.rs 的 **2 份**
  （主循环 / materialize_lifecycle）= 5 份同构代码收敛为单函数。
  差异项（步骤来源解析、变量来源、脱敏、事件、落库）刻意留在调用方——它们是真实边界。
- **顺带修复**：agent 引用式步骤的 is_write 判定重复 get_endpoint 查询（每写方法步骤
  一次 N+1），下沉后复用已取的 endpoint。
- **prepare_request**（miyo-core）：补缺→渲染→URL 渲染单链，统一 tauri
  execute_request/test_endpoint/load_test/test_collection + agent execute_endpoint
  共 5 个调用点；顺带统一了 tauri 旧 render→fill 顺序不渲染全局参数值 {{var}} 的
  隐性口径分叉（与场景引擎同口径）。
- **有意不做**：commands/ws.rs 事件转发循环是「领域事件→IPC 视图载荷」薄适配器
  而非业务逻辑，移入 miyo-http 会让传输 crate 依赖视图 DTO（分层更差）。

### 5. 前端 domains 分层（287 文件迁移）
- **7 领域落位**：endpoint（34 组件+9 composables+2 视图）/ environment / test-case /
  integration（含 store + useScenarioRun）/ load-test / mock / agent（含 store）/
  workspace（项目列表 + 壳 + dashboard store）。
- **共享层保留**：components/ui、通用 composables（useMiyoApi/useToast/lazyComponent/
  useShortcuts 等）、utils 纯函数层、stores/locale|theme|execution|workspace|treeOps、
  types/i18n/router/directives/constants/testUtils。
- **@ 别名**（vite+vitest+tsconfig 三处同步）：1488 处相对导入重写；
  vi.mock / importActual / 源码契约测试（readFileSync）/ i18nPattern 白名单同步更新。
- **收敛项**：
  - useTauriEvent / useEventSubscription：晚到监听竞态兜底收敛（原 3 个组件各自
    手写 disposed 标志），6 个组件站点转换；2 个 store 的应用级监听有意保留；
  - api.joinPath：6 处 path API 直调收编；
  - quiet() in-flight 去重：同命令+同参数并发复用（不缓存结果，零陈旧风险）；
  - content-visibility: HistoryPanel / EnvVarTable（EndpointTree 有意跳过：DOM 遍历
    键盘导航 + SortableJS 命中测试风险）；
  - CodeMirror 按需：WorkflowEditor / docs/ResponseExamplesPanel 的 JsonCodeMirror
    转 lazy（91KB 独立 chunk）；GrpcPanel 尝试后回退（spec 环境不可解析异步组件，
    且宿主 ToolsDrawer 已是 lazy）。
- **WorkflowEditor**：数据集/生命周期编辑桥（145 行）抽为 useLifecycleEditor（192 行），
  组件 3372 → 3242 行；职责三分成形（useScenarioRun 915 行运行态机 /
  useScenarioValidation / useLifecycleEditor）。

## 三、明确不做的（评估记录）

| 项 | 结论 | 理由 |
|---|---|---|
| 拆分 crate | 不拆 | 18 crate DAG 健康，无循环依赖；miyo-http→storage 横切点改动收益 < 风险 |
| E2E 框架 | 不引入 | 用户确认；121 spec + 663 Rust 测试是安全网 |
| workspace store 切片 | 暂缓 | histories 等切片与工作区快照（防闪屏）深度耦合，需先设计快照组合层 |
| WorkflowEditor 全量物理拆分 | 后续专项 | 剩余块（脚本视图 316 行 / 顶栏保存 168 行）与编辑器状态深度耦合，需先设计编辑器状态契约；拆分服务于可读性而非行数 |
| MCP Schema 全量生成 | 渐进 | 试点 2 工具建立模式，其余 33 个按模式迁移 |
| WS/SSE/gRPC 压测实现 | 评估文档 | `docs/loadtest-protocol-assessment.md`，各自连接模型不同 |
| test_runs 保留策略 | 不投入 | 休眠表（无生产写入路径） |
| TTL IPC 结果缓存 | 不做 | 陈旧风险 > 收益；in-flight 去重已覆盖主要场景 |

## 四、兼容性风险清单

1. **迁移 0029 为追加式**：仅新增索引，不改列/数据；存量迁移文件零改动（AGENTS.md 红线）。
2. **CommandError 枚举化**：序列化产物与历史字符串逐字一致（2 个契约测试守护）；
   前端 union 由生成覆盖，`as any` 已删。
3. **AppError::Sync**：sync 传输失败从 VALIDATION 变为 SYNC 码——前端无 VALIDATION
   分支逻辑（已核验仅 DECRYPT/CANCELLED 两处分支），i18n 错误文案不受影响。
4. **execute_request 渲染顺序统一**（fill→render）：全局参数值含 {{var}} 时行为从
   「不渲染」变为「渲染」——与场景引擎口径一致，属缺陷修复而非行为破坏。
5. **test_case_runs 列表瘦身**：前端 TestCaseDrawer 已改为详情按需拉取；
   MCP 工具 list_test_case_runs 输出变薄（详情走 get_test_case_run）。
6. **LoadConfig 结构变更**：仅 miyo-tauri 内部构造（前端入参向后兼容——schedule/
   max_error_rate 均可缺省，缺省 = 原 Fixed 行为）。
7. **前端全量迁移**：纯移动 + import 重写，vue-tsc/vitest/eslint/build 四绿；
   源码契约测试（readFileSync 类）路径已同步。

## 五、构建与产物

| 指标 | 前 | 后 |
|---|---|---|
| vite build（纯构建） | 2.88s | 1.14s（含 JsonCodeMirror 独立 chunk 拆分） |
| vue-tsc + vite（CI 路径） | 29.1s | typecheck 独立步骤（vue-tsc 0 错误） |
| vendor_codemirror chunk | 383KB（gzip 125KB） | 同（首屏不再强制下发：WorkflowEditor/docs 按需） |
| JsonCodeMirror chunk | 并入宿主 | **91KB 独立 chunk，按需拉取** |

> 说明：本报告构建时间为本机（M 系列自持）热缓存数据；CI 数值以 bindings/typecheck
> 新 job 首次跑出的基线为准。

## 六、门禁增量（CI）

- 新增：`Typecheck (vue-tsc)` 独立步骤（build 瘦身为纯 vite build）；
- 新增：`Export TS bindings (miyo-core / miyo-tauri)` + `git diff --exit-code` 防漂移；
- 已有且继续有效：cargo fmt/check/clippy(-D warnings)/test、迁移测试（workspace 内）、
  MCP registry 完整性测试、前端 eslint/vitest/build、tauri 插件版本一致性。
