# Miyo 全局导航规范

> 适用范围：全部核心视图与插件页面。实现单点在
> `frontend/src/components/navigation/` 与 `frontend/src/composables/useGoBack.ts`。
> 改动导航前先读本文——历史上 6 种返回钮样式、4 份 goBack 拷贝就是绕过规范的产物。

## 1. 页面分级

| 分级 | 页面 | 导航形态 |
|---|---|---|
| 根 | `/projects` 仪表板 | 无返回钮（Brand + 顶部标签 + 左侧导航） |
| 容器 | `/workspace` API 工作区 | 自有完整导航（Brand + 项目页签 + 工具组），不套 PageHeader |
| 普通子页面 | `/plugins` | PageHeader（Brand + 返回钮 + 操作区） |
| 独立调试工作台 | `/realtime` `/graphql` `/integration` `/ai-test` `/java-debug` | PageHeader 同骨架，中部 slot 放工作台上下文（模式切换 / 连接状态 / 面包屑） |
| 插件宿主 | `/p/*` 等插件路由 | PluginIframeHost 顶部走 PageHeader（沙箱徽章 + 返回钮 + 插件中心快捷钮） |

工作台与普通子页顶栏结构同构（Brand + BackButton + 上下文 + 操作区），因此合并为单一
`PageHeader` 实现——再拆 `WorkspaceHeader` 只会造出两份同构代码重新漂移。

## 2. 返回行为（useGoBack 单点实现）

```ts
const goBack = useGoBack()
goBack()            // 兜底 /projects
goBack('/plugins')  // 指定兜底目标
```

1. `window.history.state?.back` 存在（vue-router 仅在应用内导航时写入）→
   `router.back()` 原路返回来源页；
2. 深链直开 / 刷新后无来源 → `push(fallback)`（默认 `/projects`）；
3. 永远不会 `back()` 出应用：Tauri 单窗口历史栈只由本应用写入，
   `state.back` 为 null 的一律走 fallback（外部站点 → 应用、地址栏直开均如此）；
4. **300ms 导航锁**：双击返回钮 / 快捷键连按只生效一次——防 back 两步跳过
   来源页与 back↔push 死循环；
5. 模块深层页面的 fallback 应传**所属模块首页**（插件宿主 `/plugins` 范式），
   不允许新增硬编码返回路径绕过 useGoBack。

场景对照：

- **场景 A（列表 → 详情再返回）**：搜索词 / 分页 / 选中 / 树展开等状态由 Pinia
  store 承担（workspace / integration / aiTest 均跨路由存活），返回不丢；
  滚动容器为自绘（非 window 滚动），router `scrollBehavior` 不适用。
- **场景 E（弹窗优先）**：弹层 ESC 仲裁见 `utils/escStack.ts`（后开先关）；
  返回钮被弹层遮罩遮挡，不存在穿透。
- **场景 F（未保存确认）**：页面监听 `@back` 接管（见下），如集成测试编辑器态
  `requestBack` 弹脏确认后再决定去向。

## 3. 组件

### PageHeader（`components/navigation/PageHeader.vue`）

```
[Brand(标题，点击回首页)] [BackButton] [default slot] [spacer] [actions slot]
```

- 整行拖拽窗口（`.miyo-topbar` 全局类，style.css 单点定义），交互子元素豁免；
  macOS 交通灯落 Brand 区上方（JavaDebugView 曾因无 Brand 顶栏被交通灯压住返回钮）；
- 高 44px（对齐 WorkspaceView top-bar 现状；AiTestView 36px 异构高度已收口）；
- props：`title`（Brand 标题）/ `description`（页面级描述，返回钮右侧次级文字，
  1080 以下隐藏）/ `hideBack`（根页面）/ `hideBrand`（Tauri 弹出子窗）/
  `backLabel`（未传 = 「返回」；传空串 = 纯图标）/ `backTitle` / `backFallback`；
- **`@back` 监听即完全接管**（默认返回策略不再执行）——分层返回、脏确认由此实现。

### BackButton（`components/navigation/BackButton.vue`）

28px 高（--h-sm）、chevron-left 14px、可选文字（--fs-xs）、中性常态 → hover 微底色
→ active 按压 → focus 主题色描边、原生 title Tooltip。视觉层级低于页面标题：
不描边、不填充、不上主题色。不得在其他页面自行拼返回钮。

### Breadcrumbs（`components/navigation/Breadcrumbs.vue`）

- 只在存在明确层级时使用（如 集成测试 / 场景编辑器），**与返回钮不堆叠同一层级入口**；
- 当前层不可点（`aria-current="page"`），上级 `to`（路由跳转）或 `onSelect`
  （页面内动作，可先过脏确认）可点；
- 超 `max`（默认 4）层折叠中段为省略号（title 提示完整路径）。

### 弹窗 / 抽屉

不用页面级返回钮：右上角关闭 + Esc（escStack 仲裁）+ 遮罩关闭；
未保存内容由业务组件二次确认（Popconfirm / Modal）。

### 入口去重（自指收口）

- 已在目标页时入口隐藏或退化为纯展示：Brand 在 `/projects` 自动进入
  `brand-home` 纯展示态（无 tooltip、点击无效）；ProjectTabs ⋯ 菜单
  「项目列表」项在首页隐藏。
- 多入口属产品便利的场景（DashboardNav + 工作区工具组跨页直达、
  新建项目三入口）有意保留。

### Tab Navigation（页签管理）

- 持久化：`miyo.open-tabs.<projectId>` = `{ tabs, drafts, activeTabId, pinned }`；
  草稿 600ms 防抖落盘，2MB 上限；`scenario:` 伪页签恢复时丢弃；
- 固定页签（pin）：置左区、图钉标识、不受「关闭其他/右侧/全部」影响、
  关闭需先取消固定（Popconfirm 提示）；
- 最近关闭：会话内栈（上限 10），⋯ 菜单「重新打开关闭的页签」恢复草稿并激活；
- 排序：拖拽为用户显式意图，不按最近访问自动重排；最近访问由 QuickSearch
  「最近」分组承担（`miyo.recent-endpoints`）。

### 弹层层级语义

`--z-*` 阶梯（style.css 单源）：modal 100 → drawer 120 → modal-drawer 130 → popconfirm 200 →
popover 300 → tip 400 → overlay 900（全屏遮罩/命令面板）→ dropzone 999 →
toast 1000 → progress 1100。

- Modal 默认 `--z-modal`；在 Drawer 内打开的弹窗必须 `layer="drawer"`
  （`--z-modal-drawer` 130，高于抽屉遮罩 120）；
- 400px 级浮层（快捷预览等）用 `--z-popover`，不用 `--z-overlay`（后者只给
  全屏接管/命令面板）；
- 全屏接管（useOverlayFullscreen）的 Esc 一律走 escStack 压栈——禁止组件
  自挂 `window.addEventListener('keydown')` 处理 Esc（双轨已收口）。

### 响应式断点档位

统一档位（新增断点就近归档，不发明新值）：**720 / 900 / 1080 / 1280 / 1560px**。
1180 为唯一 JS matchMedia 档（WorkspaceView 顶栏直达工具 ↔ 溢出菜单互斥）。
SplitPanes 窄容器堆叠走 `stack-below`（Realtime 880 / TestCaseDrawer 720 /
RunReport 720 范式）。

## 4. 视觉 Token

- 顶栏：高 44px、`var(--border)` 分隔、`var(--bg-panel)` 底（深/浅主题变量自动适配，
  构建产物已验证规则存在——不许 `:global(html[data-theme=…])` 覆盖）；
- 返回钮：高 `--h-sm`(28px)、图标 14、文字 `--fs-xs`、圆角 `--radius`、间距 `--sp-2/3`；
- 不使用大按钮 / 渐变 / 发光（紫色 pill 仅限 DashboardNav 选中态）。

## 5. 插件导航接入规范

1. **内置插件（Vue 模块，如 Java 调试）**：视图顶栏一律用 `PageHeader`；
   返回钮经 `@back` 走 `useGoBack`（或分层逻辑），禁止自带返回实现与拖拽区代码；
   插件卸载时 PluginManager 统一回收路由与导航项，PageHeader 无状态残留
   （无全局订阅，组件卸载即释放）。
2. **第三方 iframe 插件**：宿主 PluginIframeHost 已提供统一顶栏与返回，
   插件页面本身不实现导航；返回 fallback `/plugins`。
3. **插件声明入口**：经 SDK `ctx.ui.nav.register`（manifest.nav 声明集合内），
   PluginManager 落地；插件不得直接触碰 vue-router 实例或修改核心路由。
4. 插件内跳转宿主页面（如保存用例后跳集成测试）走 `router.push` 只读使用，
   返回仍由宿主导航体系兜底。

## 6. 验收清单（回归时逐条过）

1. 首页 → 各子页面 → 返回（原路回来源页）；
2. 仪表板导航直接进实时调试 → 返回回仪表板（不会落到某个项目里）；
3. 工作区工具 → GraphQL → 返回回工作区；
4. 集成测试进编辑器 → 面包屑「集成测试」→ 脏确认 → 回列表；
5. 插件中心 ← 工作区 / 仪表板 / 插件工作台 三来源返回均正确；
6. 直链深开 `/realtime`、刷新后返回 → 回仪表板；
7. 实时调试弹窗（独立窗口）无 Brand / 返回钮，仅状态与上下文；
8. 深/浅主题下顶栏与返回钮对比度正常（`npm run build` 后 grep dist CSS）。
