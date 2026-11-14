# Miyo UI/UX 精细化优化交付报告

> 范围：统一交互规范 / 集成测试编辑器 / 测试用例抽屉 / 首页 / 性能与细节。
> 原则：不改变整体视觉风格（深色开发者工具风 + 紫色强调），全部改动建立
> 在既有 token 体系（`frontend/src/style.css`）之上；统计指标只用真实落库数据。

## 一、统一交互规范

### 1.1 新增 / 补齐的单源资产

| 资产 | 位置 | 吸收的旧实现 |
| --- | --- | --- |
| `.miyo-status` 九态徽章类 | style.css | `.tcp-status-meta.tone-*`、GrpcResponsePanel `.status-pill`、各处内联状态色 |
| `StatusBadge.vue` | components/ui | 徽章组件封装（tone×9 + 可选状态点） |
| `.miyo-spinner` + `Spinner.vue` | style.css / components/ui | 7 处散装 spinner（btn-spinner/drw-spinner/tcp-spinner/ce-spin 等） |
| `CollapsibleSection.vue` | components/ui | 各面板自绘折叠头（grid-rows 动画 + reduced-motion 降级） |
| `SplitPanes.vue` | components/ui | 请求\|响应左右分栏单源（拖拽 + 键盘 + 窄容器自动堆叠） |
| `useOverlayFullscreen` | composables | ResponsePanel 的 CSS 全屏模式泛化（escStack 仲裁退出） |
| `useVirtualList` | composables | 固定行高虚拟滚动（无第三方依赖，<80 行自动回退） |

### 1.2 Token 体系补齐

- `--radius-pill: 999px`：63 处胶囊圆角收编，禁止再写字面量；
- `--fs-2xxs: 10px`：微徽章字号入阶梯（原有 --fs-xxs=11px 覆盖不到）；
- `--z-progress: 1100`：全局加载条层级入阶梯，修复 ProgressBar(2000) 越过
  Toast(1000) 的越层；CreatePanel/QuickSearch/HeaderUpdateTag 的裸 z-index
  全部改用阶梯 token；
- `.miyo-input` 高度锚定 `--h-md`（与 `.miyo-btn` 同高，统一控件高度阶梯）。

### 1.3 全量收敛（机械迁移）

- **按钮**：全仓 275 处未走统一规范的动作按钮收敛到 `.miyo-btn` 变体系
  （primary/send/danger/danger-solid/sm/ghost），约 30+ 文件；结构性交互
  元素（列表行 / 页签 / 菜单项 / 行内微按钮 / 开关）按规范保留；
- **输入框**：文本输入收敛到 `.miyo-input(-sm)`、多行到 `.miyo-textarea`，
  自建高度/字号/圆角全部落到 token；
- **硬编码色值**：61 处 hex/rgba 字面量换语义变量（SyncDialog 9 处、
  AboutDialog、TeamManageModal、GraphQLView 孤儿 `#c084fc`→`--patch` 等）；
- **字号/圆角**：12.5px 等非 token 档归入最近档；`var(--xx, #fallback)`
  死 fallback 剥离；
- **Tabs 统一**：TestCaseDrawer 的 `.drw-tab` 迁入共享 `Tabs.vue`（新增
  `fill` 等宽变体），三套 Tab 视觉（下划线+glow / 纯下划线 / pill）收敛为
  组件单源。

### 1.4 顺手修复的既有缺陷

- `TestCasesPanel.vue`：`var(--ok)` **未定义变量**（色调失败）→ `var(--success)`；
- `MockPanel.vue`：同类 `var(--ok)` 未定义 → `var(--success)`；
- `RequestExamplesPanel.vue`：`.rex-row:hover` 引用不存在的 `var(--hover)` → `--bg-hover`；
- `style.css` L1073 未闭合块注释修复；
- `Menu.vue` 重复 keyframes：核查后确认只有一处定义（探索报告有误），未改动。

## 二、集成测试编辑器

| 需求 | 改动 |
| --- | --- |
| 步骤列表与详情面板拖宽 | 新增 SplitPanes（`widthOn=second`），右栏 420px 固定 → 320~窗口 60% 可拖，双击恢复默认 |
| 记住面板宽度 | `localStorage['miyo.wf-detail-width']`，窄窗（<窗口 60% 上限）自动夹紧不挤压流程列 |
| 失败步骤高亮 | `.wf-step.failed`：红左缘 + 危险淡底（区别于校验问题 `.invalid` 纯红描边） |
| 运行进度展示 | 保留 2px 顶栏 + Dock 进度条；新增在途步骤实时耗时徽章（秒级跳动，等宽数字） |
| 当前执行步骤自动定位 | watch 在途位次 → scrollIntoView 跟随；用户滚轮滚动即暂停，新一轮运行自动恢复 |
| 请求/响应左右分栏 | Dock 展开行改 SplitPanes（RequestDetail \| ResponsePanel），<720px 容器自动回退上下堆叠；分栏位置记忆 `miyo.wf-run-split` |
| JSONPath 快速复制 | tree 视图已有行悬停复制路径/值（保留）；pretty/raw 行视图新增 hover 行级复制按钮（`response.copyLine`） |
| 大响应体懒加载 | 现状已完善（IPC 2MB 预览截断 + `getResponseBody` 按需取全文 + 1000 行分块 + 1MB parse 上限），本次未改动 |
| 全屏查看运行结果 | ScenarioRunReport Dock 与 RunResultView 各加全屏入口（CSS 覆盖视口 + Esc 经 escStack 退出） |
| 变量来源追踪 | 连线变量 chip 变按钮：点击定位定义步骤（`flowGraph.definedBy`）；ScenarioVarsPanel 来源/引用步骤本就可点击定位（既有能力，确认保留） |
| 步骤依赖可视化 | 保留既有相邻连线 + 变量 chip + 并行虚线；chip hover 提示升级为「点击定位定义步骤」（无 depends_on 字段，依赖=顺序，跨步骤链路经由变量 chip 传导） |

## 三、测试用例抽屉

| 需求 | 改动 |
| --- | --- |
| 宿主统一 | TestCaseDrawer 从 300 行自绘 Teleport/遮罩/拖拽/焦点逻辑迁入共享 `Drawer.vue`（扩展 resizable / fullscreenable / keepMounted / guardClose / header+footer 插槽 / layer） |
| 开关动画流畅 | 共享 Drawer 统一 220ms translateX + 遮罩淡入（`--ease` 曲线）；焦点陷阱 / Esc 栈 / body 锁滚自动获得 |
| 避免切换闪烁 | `keepMounted`：关闭不卸载内容，Tab / 执行历史缓存 / 展开态保留；宽度与全屏状态持久化（`miyo.case-drawer-width`） |
| 保存与运行操作位置统一 | 底部操作栏迁入 Drawer footer 插槽（固定不滚动），按钮层级：取消（danger ghost）→ 立即运行（primary）→ 保存 |
| 请求与响应支持折叠 | 结果 Tab 分区化：断言明细 / 提取值 / 执行历史各成 CollapsibleSection（可折叠 + 计数徽章）；请求快照在分栏左栏独立滚动 |
| 请求配置和执行结果视觉分离 | 保持三 Tab（配置 / 断言提取 / 结果）；结果 Tab 内摘要条 → 分区 → 分栏的层级重构 |
| 宽度记忆 | 拖拽 + 键盘调宽（←/→ 16px、Shift 48px、Enter 恢复 680），持久化到 localStorage |
| 运行中状态明确 | 运行按钮 Spinner 化 + 取消按钮分离（沿用既有 runId 取消链路） |

## 四、首页

### 4.1 项目卡片信息密度（网格卡 + 列表行同步）

- 新增**质量行**（真实数据，`list_project_quality` + `list_project_stats`）：
  - 请求成功率：`ok/(ok+fail)`；**无请求记录显示 `—`，不虚构 0%**；
  - 场景最近运行：`{ok}✓ {fail}✗`；**从未运行显示「未运行」**；
  - 用例最近状态：同上口径（全部 Untested = 未运行）；
  - 项目无场景/用例/请求时不渲染该行（不占位）。
- **最近运行失败红点**：场景或用例最近一次运行有失败时，卡片名旁红点
  （hover 显示失败数与时间明细）；
- 列表模式新增「请求成功率」列（窄窗逐级收起：场景列 → 成功率列 → 时间列）。

### 4.2 质量概览数据源（硬约束：无真实数据不出指标）

新增后端聚合命令 `list_project_quality`（不迁移、纯查询）：

- `crates/miyo-storage/src/repository/projects.rs::list_project_quality`：
  单条 SQL——场景侧按「每场景最近一次运行」窗口函数（rn=1，复用
  `latest_run_summaries_for_project` 范式）聚合通过/失败；用例侧按
  `test_cases.last_run_status` 分布（'Success'/'Failed'/'Untested'）；
  附全项目最近失败时刻（场景 + 用例两个维度）；
- 命令层 `crates/miyo-tauri/src/commands/project.rs::list_project_quality`
  （snake_case 契约与 `ProjectStat` 一致）+ lib.rs 注册；
- 口径说明：成功 = 未取消且零失败且有通过步骤；取消或有失败计失败；
  全 skipped 的运行不两侧计入（前端由「场景 N · 通过 M · 失败 K」并列展示，
  不做隐含分区）；
- 前端 `quiet()` 在途去重，快照入 dashboard store（返回首页不闪）；
- **test_runs 孤儿表未使用**（无命令暴露、主流程不写入）；压测指标非持久化，
  不纳入质量概览。

### 4.3 其他

- 搜索增强：名称 + 描述 + 所属团队名匹配（原仅名称）；
- 快速创建入口：已有 6 处（快速开始卡 / 工具栏 / 组头 / 空状态 / Dropzone /
  ProjectTabs），本次仅视觉统一到 `.miyo-btn` 规范，未新增入口；
- 最近活动 / Hero 汇总卡：既有能力保留（活动流 100% 来自 request_histories）。

## 五、性能与细节

| 项 | 改动 |
| --- | --- |
| 虚拟滚动 | 新增 `useVirtualList`（固定行高、零依赖）；HistoryPanel（500 条历史上限）启用，56px 行高 + <80 行自动回退全量；原 `content-visibility` 由虚拟化取代 |
| 场景库 | 分组嵌套结构不适合虚拟滚动（分组行高不定、估算失准会错位），改用 `content-visibility: auto`（屏外行跳过布局/绘制） |
| EndpointTree | 维持既有决策不做虚拟化（SortableJS 拖拽命中测试冲突） |
| 弹窗布局跳动 | 共享 Modal/Drawer 已有 max-height + 内滚；新组件 CollapsibleSection 用 grid-rows 动画（0fr→1fr，内容高度自适应，无 JS 测高） |
| 动画规范 | 新组件一律 `--dur/--ease`；Spinner/状态点呼吸/折叠动画全部登记 `prefers-reduced-motion` 例外层（低幅透明度脉冲降级） |
| 键盘快捷键 | 首页 ⌘K 从手写 window keydown 迁入 `useShortcuts` 注册表（`dashboard.search-focus`，键位可自定义、帮助面板自动收录、卸载自动注销） |
| 焦点管理 | TestCaseDrawer 迁共享 Drawer 后获得焦点陷阱 + 焦点归还；SplitPanes 分割线与 Drawer 拖拽把手 `role="separator"` + 键盘调宽 + aria-valuenow |

## 六、验收

### 6.1 门禁

| 项 | 结果 |
| --- | --- |
| 前端 vitest | 126 文件 / 1026 用例全过（含新增 StatusBadge/Spinner/CollapsibleSection/SplitPanes/useVirtualList spec） |
| 前端 vue-tsc / eslint | 0 报错 |
| 前端 vite build | 通过；`dist/assets/*.css` 逐项 grep 确认新规则全部存在（AGENTS.md 深/浅验证要求） |
| 根工作区 cargo fmt/check/clippy/test | 通过（新增 `list_project_quality_aggregates_latest_runs` 仓库测试） |
| miyo-tauri 独立工作区 check/clippy/test | 通过 |
| i18nPattern / themePattern spec | 全过（新增文案全部走 `i18n/zh.ts`+`en.ts` 键集 1:1；无 `:global(html[data-theme])`、无黑名单 Tailwind 类） |

### 6.2 视口覆盖

构建产物在三视口（1366 / 1440 / 1920）下验证（详见 §6.3 截图）：

- 集成编辑器右栏宽度上限 = 窗口 60%（1366px 下约 820px，流程列保有 466px+）；
- SplitPanes 展开详情 <720px 容器自动上下堆叠（1366px 小窗不横向溢出）；
- 列表模式成功率列在 ≤1024px 自动收起，≤900px 收起时间列——无无意义滚动条；
- 抽屉 keepMounted + 弹性宽度，任何视口重开不跳位。

### 6.3 截图与界面说明

截图（`docs/imags/ui-refinement/`，浏览器 + `?miyo-e2e=1` 固定数据桩渲染）：

| 文件 | 内容 | 验收点 |
| --- | --- | --- |
| `home-1440.png` / `home-1366.png` / `home-1920.png` | 仪表板三视口 | 项目卡质量行（请求 94% · 场景 2✓ 1✗ · 用例 9✓ 2✗）、失败红点、未运行诚实展示、无横向溢出、卡片两列/三列自适应 |
| `editor-1920.png` | 集成编辑器（未选中步骤） | 左流程列（连线+变量 chip+校验面板）\| 右详情栏可拖分栏，「选中步骤」占位文案修复（原显示原始键名 `integration.stepSelect`） |
| `editor-detail-1920.png` | 编辑器步骤详情 | 步骤选中态、详情栏六 Tab、失败即停策略、校验问题红边、顶部策略/保存/运行 |
| `drawer-1920.png` | 测试用例抽屉 | 共享 Drawer 壳（统一动画/焦点陷阱）、等宽 Tabs、请求配置分区、底部固定操作条（导出代码 \| 立即运行 \| 保存修改） |

优化前对照（文字说明，改前状态已在探索阶段逐一确认）：

- 首页项目卡：仅「API 数 · 场景数 · 更新时间」一行，无请求成功率、无运行质量、无失败提示；
- 编辑器：右栏固定 420px 不可调、刷新即丢；无失败步骤卡片级高亮；无运行自动定位与在途耗时；
  展开详情上下堆叠不可分栏；运行结果无全屏入口；步骤占位文案显示原始 i18n 键；
- 用例抽屉：自绘壳（无焦点陷阱、独立动画曲线）、每次关闭整体重挂载（Tab/历史缓存丢失）、
  宽度不记忆、`.drw-tab` 与全局 Tabs 视觉不一致、底部操作栏自绘 sticky；
- 请求历史面板：500 条全量渲染（仅 content-visibility 兜底）；
- 快捷键：首页 ⌘K 为手写监听，不在帮助面板/自定义键位体系内。

### 6.4 截图基建说明

`frontend/public/dev-screenshot-stub.js` + `frontend/index.html` 的 `?miyo-e2e=1`
分支构成浏览器截图数据桩：仅 dev 且显式带查询参数时注入固定 IPC fixture，
生产构建不含该分支的任何执行路径，桩文件也从未被应用代码 import。

## 七、兼容性与风险

- **migrations 零改动**：质量聚合是纯查询（scenario_runs/test_case_runs/
  test_cases 既有表 + 既有索引），无新迁移文件；
- **Drawer/Modal 向后兼容**：新 props 全部带默认值，既有 ToolsDrawer 用法
  不变；TestCaseDrawer 迁移后其 spec（14 用例）与 TestCasesPanel spec 同步
  更新为共享 Drawer 的 DOM 契约（`.d-drawer` / `.tab` / `.sec-head`）；
- **已知口径限制**（如实呈现，不做假数据）：
  - 用例覆盖率天然不全：`last_run_status` 默认 'Untested'，未运行用例计
    「未运行」而非 0%；
  - 历史运行 64KB 正文截断不可恢复（既有后端限制，UI 已警示）；
- **行内表格输入**（ParamDefineTable 等）迁为 boxed `miyo-input-sm`，与
  KeyValueTable 的无边框行内风格存在分叉（后者是 ui 原语内部，本次未动）；
- **WindowControls 的 `#e81123`**：Windows 关闭钮系统红，无语义 token，
  属平台惯例色保留。
