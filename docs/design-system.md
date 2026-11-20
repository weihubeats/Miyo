# Miyo Design System（前端设计系统）

> 本文档是 Miyo 前端视觉与交互规范的**单源说明**。所有页面必须消费本文描述的
> Token 与公共组件；新增样式前先查本文，禁止页面自造第二套颜色/圆角/字号。

## 1. 架构总览

```
style.css                    ← 设计 Token 单源（:root 深色基线 + [data-theme] 覆盖块）
  ├─ @theme                  ← Tailwind v4 语义色映射（bg-panel / text-ink1 / border-line …）
  ├─ [data-theme-id]         ← 8 个内置主题块（深 3 + 浅 5）
  └─ 全局组件类              ← .miyo-btn / .miyo-input / .miyo-card / .miyo-status …
theme/
  ├─ types.ts                ← 主题数据模型（ThemeConfigV1 持久化结构）
  ├─ presets.ts              ← 主题预设 + parseThemeConfig 校验（非法即回退默认）
  ├─ apply.ts                ← 自定义主题 → <html> 内联变量（密度/字体/颜色/圆角/字号）
  └─ backgroundImage.ts      ← 背景图压缩管线
stores/theme.ts              ← 状态机：system 跟随系统 / manual 固定，切换免刷新
components/ui/               ← 公共组件（26+，见 §4）
components/navigation/       ← PageHeader / Breadcrumbs / BackButton
```

DOM 约定：`<html data-theme="dark|light">` 恒为二值基调（Tailwind `dark:` 变体与
CodeMirror 消费点），主题身份写 `<html data-theme-id>`，自定义主题差异由 apply.ts
写内联 CSS 变量表达。**组件内禁止出现 `html[data-theme]` 选择器**
（`themePattern.spec.ts` 强制——该写法经 Vite 构建后丢失，已实证踩坑）。

## 2. 设计 Token

### 2.1 颜色（深/浅各一套，组件只消费 var()）

| Token | 语义 | 对应请求里的别名 |
| --- | --- | --- |
| `--bg-app` | 页面最底层背景 | background |
| `--bg-panel` | 主面板背景 | surface |
| `--bg-card` | 卡片/输入井底 | surface-secondary |
| `--bg-elevated` | 浮层/菜单底 | — |
| `--bg-code` | 代码井底 | — |
| `--bg-hover` / `--bg-active` | hover / 选中态 | surface-hover |
| `--border` / `--border-strong` | 常规/强调描边 | border |
| `--text-1/2/3` | 主/次/三级文本 | foreground / muted |
| `--text-value` | 数据数值档 | — |
| `--accent` / `--accent-hover` / `--accent-tint` | 强调色家族 | primary |
| `--primary` / `--grad-primary`（±hover/glow） | 主行动按钮渐变真源 | — |
| `--success / --warning / --danger / --info / --orange` | 语义状态色 | success/warning/danger/info |
| `--get/post/put/patch/delete/grpc` | HTTP 方法语义色 | — |
| `--*-tint` | 各语义色柔底 | — |

Tailwind 工具类走 `@theme` 映射（`bg-panel`、`text-ink2`、`border-line`、
`text-method-get` …）。方法色组合唯一来源：`utils/methodTone.ts` 映射表，
**禁止拼接构造类名**（Tailwind 扫描不到）。

### 2.2 尺寸

- 控件高度：`--h-xs/sm/md/lg` = 24/28/32/36px（**随密度缩放**，见 §3.2）
- 间距：`--sp-1..6` = 4/8/12/16/20/24px（**随密度缩放**）
- 圆角：`--radius-sm/md/lg/xl` = 4/6/8/10/16px（--radius=6px；胶囊用 `--radius-pill`，
  禁写 9999px/999px 字面量）
- 字号：`--fs-2xxs..xl` = 10/11/12/13/14/16/20px（正文 13）
- 图标：Icon.vue 统一 24 网格 viewBox，常用 12/13/14/16

### 2.3 交互状态

- hover：`--bg-hover`；active：`--bg-active`
- focus：全局 `:where(:focus-visible)` 零特异性光环（`--focus-ring`），
  组件自有 focus 样式优先；输入类 focus = accent 描边 + 2px accent-tint 光环
- disabled：`opacity: .45~.5` + `cursor: default`
- selected：`--accent-tint` 底 + accent 描边
- loading：`Spinner.vue` / `.miyo-spinner`（1em currentColor）
- 校验错误：全局 `.miyo-input-error`（danger 描边 + danger-tint 光环，单源无 !important）

### 2.4 层级（z 阶梯，全仓单源）

```
--z-modal 100 → --z-drawer 120 → --z-popconfirm 200 → --z-popover 300
→ --z-tip 400 → --z-overlay 900 → --z-dropzone 999 → --z-toast 1000 → --z-progress 1100
```

组件一律 `z-index: var(--z-*)`，字面量视为违规（低风险局部层叠 0/1/2 除外）。

## 3. 主题引擎

### 3.1 内置主题（8 个，深 3 + 浅 5 成对）

| 深色 | 浅色 | 色相 |
| --- | --- | --- |
| 石墨深色 `dark` | 极简浅色 `light` | 中性灰 + 靛紫强调 |
| 海盐蓝 `ocean` | 海盐浅蓝 `ocean-light` | 蓝灰 + 海蓝强调 |
| 森林绿 `forest-dark` | 森林浅绿 `forest` | 绿灰 + 林绿强调 |
| — | 暖沙色 `sand` / 紫雾色 `lavender` | 追加浅色（向后兼容保留） |

模式：跟随系统（`mode: system`，深浅记忆对 lastDark/lastLight）或手动固定。
切换实时生效（写 DOM + `theme-switching` 守卫类禁用全局过渡防闪烁），
CodeMirror 编辑器经 `miyo:theme` 事件换主题，**不重新初始化**。

### 3.2 自定义主题（单槽位 `custom`）

| 能力 | 字段 | 实现层 |
| --- | --- | --- |
| 界面基调 | `base` | data-theme 切换 |
| 五色覆盖 | `colors.{bg,card,border,text,accent}` | apply.ts 派生整套（hover/tint/渐变/光晕/文本梯度） |
| 背景 | `background.{color,gradient,image}` | 纯色/渐变写 --bg-app；图片走 AppBackground 固定层（降采样 ≤1080、dataURL 持久化） |
| 图片模糊/透明度/位置 | `image.{blur,opacity,position,fit}` | AppBackground 内联样式 |
| 卡片透明度 | `cardOpacity` | 面板/卡片降不透明（背景图模式让图片透出工作区） |
| 圆角 | `radius` | --radius 四档映射 |
| 字号缩放 | `fontScale` | --fs-* 全档 0.85~1.3 |
| **界面密度** | `density: compact/default/cozy` | 内联 `--density: 0.9/1/1.1`，--h-*/--sp-* 全档 calc 缩放 |
| **界面字体** | `font: default/system` | 内联 --font-ui 覆盖（系统字体栈） |

持久化：localStorage `miyo.theme.config`（ThemeConfigV1，version 字段预留迁移）。
`parseThemeConfig` 对导入/读取统一校验：非法字段回退默认、整体不可用返回 null；
density/font 为 v1 后增字段，旧配置缺省即默认，无需迁移。设置页提供
导入 / 导出 / 恢复默认主题。

**可读性约束**：背景图模式下面板仍保持实体底（降不透明但不透明化代码井），
编辑器对比度由 `--code-*` 独立控制，不受背景自定义影响。

## 4. 公共组件（components/ui/ + navigation/）

高频：`Icon`(92 引用) / `IconButton`(44) / `Modal`(38，焦点陷阱+Esc 栈) /
`CustomSelect`(30，listbox 键盘导航) / `EmptyState`(19) / `Popconfirm`(16) /
`Tabs`(13) / `Menu`(12，openAt 定位 + **↑↓/Home/End/Enter 键盘漫游** +
aria-activedescendant) / `Tooltip`(11) / `SegmentedControl`(10) / `StatusBadge`(8) /
`SearchInput`（**新增**：图标+清空+Esc 清空+focus()/select() 暴露，11 处自绘搜索框已迁移）/
`Skeleton` / `Spinner` / `KeyValueTable` / `CustomNumberInput` / `SplitPanes`（拖拽+键盘调宽+窄容器堆叠）/
`JsonCodeMirror` / `JsonTree` / `CollapsibleSection` / `FindBar` / `VarSuggest` / `EnvBadge` / `MethodText`。
导航：`PageHeader`（.miyo-topbar 44px 单源，7 处使用）/ `Breadcrumbs` / `BackButton`。

通知：`useToast` + ToastHost（全仓唯一 toast 实现，aria-live）。
右键菜单：`@contextmenu.prevent` + `Menu.openAt()`（EndpointTree / TabBar）。

**分工约定**
- 溢出截断提示 → `v-tooltip-overflow` 指令；主动提示 → `Tooltip.vue`；
- 搜索框 → 一律 `SearchInput.vue`（CustomSelect 内嵌 #search 槽除外）；
- 按钮视觉 → `.miyo-btn` 全局类族（primary/send/danger/danger-solid/sm/ghost），
  scoped 内只允许**布局覆写**（宽度/flex），禁止复制整套视觉；
- 表单校验错误态 → 全局 `.miyo-input-error`，禁止 scoped 复制。

## 5. 布局规范

- **顶栏单源 44px**：PageHeader(.miyo-topbar) = WorkspaceView(.top-bar) = ProjectList(.dash-top)；
  macOS 主窗 `data-window="main"` 左侧预留 88px 交通灯区；
- **侧边栏**：WorkspaceView 左栏可拖拽（220–520px，useSidebarResize：键盘微调/双击复位/
  持久化）且可折叠；面板分割 → SplitPanes（JavaDebug 四层嵌套 / Realtime / WorkflowEditor）；
- 面板间距走 `--sp-*`；弹窗宽度由 Modal `width` prop 统一（420px 表单级 / 720px+ 内容级）。

## 6. 回归测试

| 测试 | 守护内容 |
| --- | --- |
| `themePattern.spec.ts` | 组件内禁 `html[data-theme]` 覆写、禁恒深底/亮紫字面量 |
| `i18nPattern.spec.ts` | 禁硬编码中英文用户可见文案 |
| `stores/theme.spec.ts` | 主题切换/持久化/迁移/自定义槽位/密度字体/导入导出 |
| `stores/locale.spec.ts` | zh/en 键集 1:1 |
| `Menu.spec.ts` | openAt/close/选择 + 键盘漫游 + 确认视图 |
| `SearchInput.spec.ts` | v-model / Enter / 清除钮 / Esc 清空且阻断冒泡 |
| `ThemeAppearance.spec.ts` | 8 内置卡 + 自定义入口 radiogroup |

构建验收：`npm run build` 后 grep `dist/assets/*.css` 确认主题块/新类存在
（dev 与构建行为可能分叉，深/浅验证必须看产物——已实证）。
