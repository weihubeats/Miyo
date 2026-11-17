# AGENTS.md

Miyo：Tauri 2 + Vue 3 的跨平台 API 调试工具（Rust 工作区 + 前端工作区 + 两个独立的 Tauri 工作区）。

## 技能（Skills）

技能正文统一放 `.agents/skills/<name>/SKILL.md`，`.claude/skills/<name>` 为指向它的符号链接：

- `.agents/skills/`：ZCode 原生识别（跨工具标准位置）；
- `.claude/skills/`：Claude Code 原生识别，OpenCode 亦兼容读取——因此**无需**再为
  OpenCode 单独建 `.opencode/skills`；
- 修改技能只改 `.agents` 一份，两个链接同步生效。

技能清单：

- **release**：发布流程——CI success 前置 → `node scripts/release.mjs` 一键升版推送打 tag →
  轮询 Release workflow → draft release 转正。触发词：「发布 / 发版 / release / 打 tag」。
- **push**：推送流程——按 CI 同款门禁预检（根 workspace fmt/check/clippy/test、
  miyo-tauri check/test、前端 lint/test/build）→ 主题化提交 → 推送 → 监控 CI 直到绿。
  触发词：「推送 / push / 提交推送」。

## 后端规范（实证踩坑）

- **`crates/miyo-storage/migrations/*.sql` 已应用的迁移文件一律不可修改**（含注释）：
  sqlx 把每个文件的 SHA-384 校验和记在 `_sqlx_migrations`，改一字节即校验失败、
  所有老库启动报「数据库初始化失败」（2026-10 更名清扫改了 0015~0017 注释，
  dev/正式版升级双双踩雷，已实证）。品牌更名等批量重构必须排除 migrations 目录；
  变更只能以新迁移文件（递增编号）追加。
- **dev 干净启动契约**：`reset_dev_database` 会删当前名与旧品牌名（`rustfox.db`）
  的库文件；给库文件名/数据目录改名时，两个删除清单都要覆盖，否则旧文件被
  兼容迁移改名回来，dev 不再从种子数据启动。
- **改默认值必须配套清存量行的迁移**：`RequestSpec.timeout_ms` 默认从 30000 改
  300_000 时只改了新建路径，存量接口仍显式存着 30000，覆盖了新的全局设置
  （前端按 30s 提前取消，与设置页展示的 300s 不符，2026-10 用户实测）。
  改「隐式默认」这类值时，确认存量行里有没有旧值残留，需要时追加一条清理
  迁移（如 `0019`）把「恰好等于旧默认」的值改回 NULL/新默认。
- **前端超时解析需与后端 `resolve_timeout_ms` 对齐**：后端链路是
  `spec.timeout_ms → 全局设置 → 300s` 三重兜底；前端 `send()` 只取前两段会漏
  掉全局（`globalTimeoutMs` 曾被当成纯占位符文案用），接口留空时前端不设
  timer、后端才兜底，两侧行为不一致。

## 前端 UI 规范（实证踩坑）

- **禁止 `:global(html[data-theme=…])` 做主题覆盖**：该写法经 Vite 构建后丢失
  （产物无对应规则，已实证：侧边栏/右键菜单浅色主题因此静默全黑）。
  主题差异一律用 `style.css` 的 CSS 变量表达（深/浅各一套），回归测试
  `frontend/src/themePattern.spec.ts` 会扫全仓 `.vue` 拦截。
- **深/浅验证看构建产物，不只看源码**：`npm run build` 后 grep `dist/assets/*.css`
  确认规则存在（如 `.miyo-sidebar` 是否跟变量），dev 与构建行为可能分叉。
- **flex `<button>` 的子元素禁止用 `display: block`**：WebKit（Tauri WKWebView /
  Safari）对 flex 按钮内的 block 子元素不做伸展，宽度塌缩为 0（主题预览卡实证：
  只剩边框塌成的一条细竖线）；Chromium 会正常 blockify，只在 WKWebView 里坏，
  极易漏检。按钮内子元素统一 `display: flex` + 显式 `width: 100%`，视觉回归
  须在 Safari/WKWebView 复验，不能只看 Chromium 内核浏览器。
- **共享类禁止只定义在某组件 scoped 内**：`m-select-*` 曾只在 EndpointEditor 局部，
  另 4 处引用实际裸奔。跨组件复用的类一律放 `style.css` 全局。
- **方法色 single source**：`@theme` 的 `method-*` 令牌（色值）+ `utils/methodTone.ts`
  （组合：徽章式/纯文本式）。**禁止拼接构造类名**（如 `` `text-method-${x}` ``，
  Tailwind 扫描不到）；新增方法先补映射表，未知方法兜底中性灰。
- **Tailwind 是 v4 不是 v3**：CSS-first，无 config 文件；跳过 preflight；
  `dark:` 变体绑定 `<html data-theme>`（跟主题 Store，不跟系统）。
- **浅色可见性**：`rgba(255,255,255,…)` 边框/底色在浅色下隐形，一律用变量
  （`--border`/`--bg-hover` 等）；彩色底上的白色高光、CodeMirror JS 主题字面量除外。
- **圆角/字号走 token**：`--radius-sm/md/lg/xl`（4/6/8/10/16），碎值（7/5/3px）能收则收；
  饱和底（紫 pill）上的徽章/文字要提亮底色保可读。
- **`prefers-reduced-motion`**：`style.css` 有全局兜底一刀切；JS 计时器类动画
  各组件自理。新增持续动画时检查该媒体查询。
- **EndpointEditor 单实例常驻**：在途/响应/计时状态必须按接口 id 分桶，
  否则切 Tab 会串响应；全局 `sending` 布尔会阻塞多标签并发。

## 前端 i18n 规范

- **入口唯一**：`stores/locale.ts`——组件用 `const t = useLocaleStore().t`，pinia store
  在 setup 里同样取 `t`；纯 utils 抛用户可见错误用 `tFallback()`（无 Pinia 时兜底中文）。
  字典 `i18n/zh.ts`（键名真源，`ns.semantic` 扁平键）+ `i18n/en.ts`，键集必须 1:1
  （`stores/locale.spec.ts` 强制），插值用 `{name}` 占位。
- **禁止硬编码用户可见中文**：`src/i18nPattern.spec.ts` 扫全仓（去注释后）拦截；
  允许的例外只有存库数据值（测试用例分类 `'正向'` 等，展示经 `caseCategoryLabel`
  映射翻译）、`'简体中文'` 语言自称、`module_name: '默认'`（有按名匹配逻辑）、
  正则输入匹配与 console 日志——新增特例先改该 spec 的白名单再写代码。
- **英文硬编码（placeholder / 列头 / 页签 label）同样禁止**，一律走 `t()`；
  `src/i18nPattern.spec.ts` 除中文外另扫 `.vue` 模板（静态 `placeholder` 属性 +
  纯文本节点）拦截英文。白名单只放行两种语言都显示原文的专有名词（cURL / JSON /
  GraphQL 术语 / HTTP 方法与状态短语 / 语言名）与示例内容（路径 / URL / JSON 片段），
  可译 UI 词（Key / Value / Body …）不放行——新增特例先改该 spec 再写代码。
- **组件 spec 必须固定语言**：组件 setup 已调 `useLocaleStore`，spec 需要
  `setActivePinia(createPinia()) + useLocaleStore().setMode('zh')`（模式见
  AuthPanel.spec.ts），否则断言文案随环境语言漂移或直接缺 Pinia 报错。
- **`defineProps` 默认值禁止调 `t()`**：会被提升到 setup 之外编译报错
  （ParamDefineTable 踩过）；占位文案在模板里 `prop ?? t('key')` 兜底。
- **随语言刷新的常量数组**（下拉选项、页签）必须包 `computed`，setup 时一次性
  求值的数组切语言后不更新。

