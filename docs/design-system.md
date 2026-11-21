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
stores/theme.ts              ← 状态机：system 跟随系统 / manual 固定 + 视觉风格档，切换免刷新
components/ui/               ← 公共组件（26+，见 §4）
components/navigation/       ← PageHeader / Breadcrumbs / BackButton
```

**首帧预置（防 FOUC，2026-11 实证 bug）**：`index.html` 的 `<head>` 内联脚本
在解析期同步写 `data-theme / data-theme-id / data-style / .dark|.light`（镜像
store 解析含旧三态键迁移与记忆对回退）。**修改主题解析逻辑时必须同步该脚本**，
否则跟随系统浅色的用户每次启动都会先闪一帧默认深色。

DOM 约定：`<html data-theme="dark|light">` 恒为二值基调（Tailwind `dark:` 变体与
CodeMirror 消费点），主题身份写 `<html data-theme-id>`，**视觉风格写
`<html data-style>`**（studio/minimal/soft，与颜色主题正交），自定义主题差异由
apply.ts 写内联 CSS 变量表达。**组件内禁止出现 `html[data-theme]` 选择器**
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
| `--text-on-accent` | 彩色底上的文字/白色元素（按钮字、开关旋钮、徽章） | — |

**设计决策（专业感走查结论，2026-11）**：主行动色一律**扁平 accent 实色**——
`--grad-primary/hover` 的值为同色双 stop（token 名保留兼容），`--primary-glow`
降到噪底级（0.10~0.14），按钮无内高光/无上浮；**激活态 = tint 底 + accent 字 +
左缘指示条**（DashboardNav / EndpointTree），禁止「渐变 pill + 白字 + 光晕」的
营销页语言。新主题色板直接给 accent 实色即可。

**色彩活度决策（沉闷走查结论，2026-11）**：八套主题的中性色**必须携带主题色相**
（石墨=蓝紫调灰、海盐=蓝调灰、森林=绿调灰…禁止纯中性灰；浅色主题 app 底与
边框带 4~6% 色相），accent 饱和度取「鲜明但不到荧光」档（深色主题可更亮，
浅色主题文字用色保 ≥4.5:1 对比）。表面明度阶梯 app < panel < card < elevated
保持 ≥3% 差距。预览 swatch（presets.ts）与 style.css 色值必须同步。

Tailwind 工具类走 `@theme` 映射（`bg-panel`、`text-ink2`、`border-line`、
`text-method-get` …）。方法色组合唯一来源：`utils/methodTone.ts` 映射表，
**禁止拼接构造类名**（Tailwind 扫描不到）。

### 2.1.1 组件级语义 token（视觉风格的驱动面）

业务组件与全局类**只消费**这些语义 token，不得直写主题色板（风格三态在
`[data-style]` 块覆盖它们）：

| 组 | Token |
| --- | --- |
| button | `--btn-bg / --btn-bg-hover / --btn-border / --btn-border-hover` |
| input | `--input-bg / --input-border / --input-border-hover` |
| menu/dropdown | `--menu-bg / --menu-border / --menu-shadow / --menu-radius` |
| card | `--card-bg / --card-border / --card-shadow / --card-radius` |
| modal | `--modal-bg / --modal-border / --modal-shadow / --modal-radius`（设置弹窗深一档特例走 `--dialog-*`） |
| toolbar | `--toolbar-border` |
| editor/code | `--code-bg / --code-fg / --code-caret / --code-placeholder` + `--tok-*` 语法色 |

圆角四档为 `calc(Npx + var(--radius-shift))`，风格档经 `--radius-shift`（soft
+2px）整体偏移；自定义主题 radius 走内联覆盖（优先级高于风格档）。

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

### 3.0 视觉风格（与颜色主题正交的第二维度）

`ThemeConfigV1.style`（v1 后增字段，旧配置缺省 → 'studio'，导入导出兼容），
`<html data-style>` 承载，style.css 的 `[data-style]` 块覆盖组件语义 token：

| 风格 | 视觉语言 |
| --- | --- |
| **Developer Studio**（默认） | IDE 质感：hairline 分割、实体卡片（--card-border 描边）、紧凑层级、shadow-sm 卡影 —— 即 :root 组件 token 基线 |
| **Minimal** | 卡片/次级按钮**无边框无阴影**（bg 用 text-1 与 panel 的 color-mix 构建对比），输入框降为轻描边，浮层/弹窗阴影减档，顶栏 hairline 减淡 |
| **Soft** | 圆角整体 +2px（--radius-shift）、卡底带 6% accent tint（color-mix）、边框 60% 弱化、阴影升为主题柔影档、按钮实底无边框 |

约束（防风格块破坏主题）：
- 风格块只覆盖**组件语义 token 与 --radius-shift**，不碰主题色板 token；
- 阴影一律引用主题块给定的 --shadow-*（深浅自适应），禁止黑色字面量——
  深色主题下 text-1 近白，text 派生阴影会变成发光；
- 自定义主题 colors/radius 走内联变量，优先级高于风格档；
- 设置页 StyleCard 用真实组件 token 渲染迷你界面，预览子树经
  `:data-style` 重新作用域 token——「当前色板 × 该风格」所见即所得。

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
  **全屏面板（fixed inset:0）首行工具栏同样预留 88px**——全局规则集中在
  style.css「全屏面板」段（rp/json-editor/se-report/wf-root/rv-root/drawer 六面），
  新增全屏面必须同步登记；inset 留缝会让底层 chrome 渗出（用户实测）；
- **侧边栏**：WorkspaceView 左栏可拖拽（220–520px，useSidebarResize：键盘微调/双击复位/
  持久化）且可折叠；面板分割 → SplitPanes（JavaDebug 四层嵌套 / Realtime / WorkflowEditor）；
- 面板间距走 `--sp-*`；弹窗宽度由 Modal `width` prop 统一（420px 表单级 / 720px+ 内容级）。

## 6. Impeccable 设计质检（外部工具链）

[impeccable](https://github.com/pbakaus/impeccable) 的 62 条确定性反模式规则
已接入日常流程：`npm run detect:ui`（源码扫描 + 运行页扫描）。
**2026-11 全量清零**，含 AA 对比度修复（实心按钮 `--grad-primary` 与亮
`--accent` 分离：按钮底为 accent 加深档，白字 ≥4.5:1）、side-tab 竖条移除
（激活态仅 tint+accent 字）、功能徽章 11px 地板、CodeMirror/原生占位文字
token 化、装饰性 glow 清零。

**保留的例外（有意豁免，勿机械「修复」）**：
- `monotonous-spacing`（BodyPanel 8px 均匀间距）：开发工具的 4px 网格密度意图；
- `0 0 0 2px accent-tint` 焦点环：a11y 焦点可见性（WCAG），非装饰 halo；
- `sc-key-pulse`（快捷键录制脉冲）：真实交互进行中状态，pulsing-dot 规则的
  「genuinely live」例外。

## 7. 回归测试

| 测试 | 守护内容 |
| --- | --- |
| `themePattern.spec.ts` | 组件内禁 `html[data-theme]` 覆写、禁恒深底/亮紫字面量 |
| `i18nPattern.spec.ts` | 禁硬编码中英文用户可见文案 |
| `stores/theme.spec.ts` | 主题切换/持久化/迁移/自定义槽位/密度字体/**视觉风格**/导入导出 |
| `stores/locale.spec.ts` | zh/en 键集 1:1 |
| `Menu.spec.ts` | openAt/close/选择 + 键盘漫游 + 确认视图 |
| `SearchInput.spec.ts` | v-model / Enter / 清除钮 / Esc 清空且阻断冒泡 |
| `ThemeAppearance.spec.ts` | 8 内置卡 + 3 风格卡（含真实预览 data-style 断言）radiogroup |
| `npm run detect:ui` | impeccable 反模式扫描（源码 + 渲染页，2026-11 起清零基线） |

构建验收：`npm run build` 后 grep `dist/assets/*.css` 确认主题块/新类存在
（dev 与构建行为可能分叉，深/浅验证必须看产物——已实证）。
