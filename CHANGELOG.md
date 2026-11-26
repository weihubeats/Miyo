# CHANGELOG

版本变化速览（完整提交记录见 `git log`，发版流程见 `docs/RELEASE.md`）。

## Unreleased（集成测试场景目录）

功能：

- 场景库升级为「左侧业务目录树 + 右侧场景列表」：多级目录（新建 / 重命名 /
  拖拽移动 / 展开折叠 / 同级排序），目录行展示场景数与子树执行状态汇总；
- 场景支持所属目录 / 标签 / 收藏；列表新增执行次数、成功失败计数，
  支持搜索（名称 / 描述 / 标签 / 目录路径）、状态 / 分类 / 收藏 / 目录筛选、
  列表 / 卡片视图切换、按更新时间排序；
- 批量操作：执行（顺序执行、失败即停）、移动目录、追加标签、收藏 / 取消、
  删除（二次确认）；场景可拖入目录移动，目录可拖到其他目录 / 根级（防环）；
- 目录删除三策略：仅空目录 / 场景移到未分类 / 连场景与运行历史一起删除
  （级联需二次确认），任何策略都不静默丢数据；
- 备份 schema 升到 v6（携带场景目录，恢复时重映射目录与场景归属），
  同步覆盖一并保留目录结构；
- 新增迁移 `0035_scenario_folders.sql`（`scenario_folders` 表 + 场景目录 /
  标签 / 收藏 / 全量执行计数字段与索引）；说明见 `docs/SCENARIO-FOLDERS.md`。

## Unreleased（黑屏诊断，issue #192 复盘）

修复：

- 渲染活性守卫（`frontend/src-tauri/src/shell_guard.rs`）：页面加载成功、
  `frontend-ready` 准时到达但合成器输出永不呈现时（Windows Server + 虚拟机实测），
  旧兜底完全失效。现在前端在 `requestAnimationFrame` 里周期心跳
  （`render_heartbeat`），壳侧检测到计数停滞 15s 即按
  「强制重绘 → 软件合成重载 → 原生提示 + 诊断落盘」三级降级
- WebView2 渲染进程监控：挂 `ICoreWebView2_2::ProcessFailed`，
  把渲染进程退出 / 无响应 / 浏览器进程退出连同 reason / exit code / 进程描述
  写进日志（此前完全无记录，只能靠用户偶然杀掉进程才看得见现象）
- 误报修复（macOS 实测）：窗口切到后台 / 被遮挡 / 最小化时系统会暂停
  `requestAnimationFrame`，心跳归零被当成「画面冻结」，正常使用中突然弹
  「界面无法刷新」。判定前现在先确认窗口在前台（可见 / 未最小化 / 有焦点），
  不满足只重新起算；心跳从未上报（计数恒为 0）单独归因，不再触发无效重建
- **根因修复**：心跳调用漏了插件命名空间（`invoke('render_heartbeat')`，
  而命令注册在 `plugin:miyo|` 下），Promise 被 `.catch` 吞掉、计数恒为 0，
  守卫从启动起就必然误判。改为 `plugin:miyo|render_heartbeat`；
  同类问题 `plugins/sdk.ts` 的 `ctx.paths.data()`（`plugin_dir_get`）一并修正
- 新增门禁 `pluginInvokePattern.spec.ts`：以 Rust 侧 `build.rs` 的命令清单为
  唯一真源扫描前端源码，裸命令名必须经 API 层封装（`call` / `quiet` /
  `callIpc` / `callWithSession`）或带 `plugin:miyo|` 前缀，否则测试失败
- 重建窗口修复：`close()` 只是投递 `CloseRequested`，webview 条目要等
  `Destroyed` 派回事件循环才移除，紧接着建同 label 必撞
  `a webview with label main already exists`。改为 `destroy()` + 轮询等 label
  释放，并在重建期间拦下「最后一个窗口销毁 → 退出应用」；该路径收紧为
  Windows 专属（`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 只对 WebView2 生效，
  其它平台重建只会丢表单与滚动位置）
- 诊断文案按平台分流：macOS / Linux 不再给 Windows Server 的 `dism` 指引，
  改为「切焦点 / 重启进程 / 命令行导出」，诊断文件另附判定时的窗口状态
- 重建窗口修复：`close()` 只是投递 `CloseRequested`，webview 条目要等
  `Destroyed` 派回事件循环才移除，紧接着建同 label 必撞
  `a webview with label main already exists`。改为 `destroy()` + 轮询等 label
  释放，并在重建期间拦下「最后一个窗口销毁 → 退出应用」；该路径收紧为
  Windows 专属（`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` 只对 WebView2 生效，
  其它平台重建只会丢表单与滚动位置）
- 诊断文案按平台分流：macOS / Linux 不再给 Windows Server 的 `dism` 指引，
  改为「切焦点 / 重启进程 / 命令行导出」，诊断文件另附判定时的窗口状态

功能：

- `Miyo.exe --miyo-export-db=<路径>`：无界面导出当前数据库（`VACUUM INTO`
  一致性快照），窗口点不动时仍能把数据救出来，完成后弹原生框确认路径
- 判定画面冻结时写 `blank-screen-<时间>.txt` 到日志目录，含平台 / WebView2 版本 /
  心跳计数 / 降级阶段与三条自救步骤，用户可整目录打包反馈

## Unreleased（性能与功能优化合集）

> 以下条目为 0.0.13–0.0.25 期间陆续合入的累计清单，正式归档见各版本段。

功能（团队协作，未发版）：

- 团队数据模型：项目按团队分组（团队管理弹窗 / 新建项目选团队 / 拖拽排序）
- 团队同步：项目快照经 git 远端推/拉（远端绑定团队维度，多项目/多团队可共用同一仓库，
  仓库内按 `projects/{id}.json` 分文件）；推送前凭证自动脱敏，拉取先预览差异、
  确认后覆盖且覆盖前自动本地快照兜底
- 首页导入入口：浏览远端项目快照清单，远端新增项目一键导入为本地项目
- git 远端支持 SSH / HTTP(S) / 本地路径；空仓库首推、无全局 git 身份均可工作
- S3 / 数据库远端占位（界面可选，标"开发中"）

UI（浅色主题对比度重构）：

- 表面色阶：主背景升 slate-100，面板/抽屉/卡片统一纯白，层次改由
  slate-200/300 边框与阴影表达
- 输入控件（rf-input / CustomSelect / 数字输入）边框升 slate-300 档；
  聚焦态紫描边 + 2px 光环既有保留
- 方法徽章与状态色切 700 档（POST amber-700 / GET emerald-700 等），
  白底对比度全部 ≥4.5:1；三级文字升 gray-500
- 代码编辑器浅色主题显式 slate-50 底 + slate-400 行号；Modal 底部
  操作栏补 hairline 分隔线
- 修复幽灵 token：`--bg-1/--bg-2` 被 4 组件 17 处引用却从未定义，
  非性能 unset 透出遮罩层——深色"碰巧像"面板，浅色下露出浑浊灰底
  与硬边线（用例抽屉右侧亮线即此）；现收敛为语义别名（bg-1=井槽
  底走 --bg-code，bg-2=壳层走 --bg-panel）
- 抽屉投影 token 化：浅色用低透明冷灰柔影替代纯黑浓影（--shadow-drawer），
  边缘只留一道 slate-200 hairline；遮罩统一走 --mask 变量
- 顶栏减负：GraphQL/实时/快捷键/团队同步收进「⋯」溢出菜单（新增
  workspace.moreTools），右栏工具组由 6 键减为 3 键
- 未发送阶段布局反转：请求卡吃掉剩余纵向高度（设置条钉卡片底缘），
  响应空态由占 ~2/3 屏收为 240px 固定低条；空态可点击聚焦 URL 栏
- 表头/内嵌带恢复层次：键值表头、环境快览头尾、环境管理侧栏与表、
  设计面板行/胶囊/方法选择改走 --bg-code 井色（--bg-panel 改白后
  这些内嵌带与承载面同色、仅剩分隔线）
- req-settings 底色收敛到 --bg-code，移除组件级 html[data-theme] 覆写
  与 color-mix(black N%) 脏灰
- 环境浮层右缘夹取：CustomSelect measure() 按实测宽左移、不越出视口
  （触发器贴右缘时右侧选项/底栏曾被窗口截断），.cs-pop 另加
  max-width 兜底
- 主题单源收口：10 处组件级 html[data-theme] 覆写（CustomNumberInput /
  DesignPanel / ProjectCard / SettingsDialog / EndpointEditor / BodyPanel）
  全部收敛为 style.css 成对 token（--inset-input-* / --dialog-* /
  --update-card-bg / --check-bg / --danger-fg / --projcard-*），组件只
  消费 var()；themePattern 回归测试升级为拦截任何组件级
  html[data-theme] 选择器
- 更新弹窗浅色适配：ReleaseNotesModal 移除「双主题恒深底」覆写
 （弹窗底/标题/分隔线随主题令牌，遮罩走 --mask 深浅两套），更新日志
  框改主题化井底（--bg-code + --border + --text-2），Markdown 内文与
  次要按钮「稍后再说」全部走变量（原 hover 反白浅色下白字隐形）；
  底部按钮统一换 rf-btn / rf-btn-primary 按钮族（原硬编码紫 +
  深紫阴影 shadow-purple-950/60 在浅色下糊成脏影）
- 全仓浅色扫尾（同类病灶统一修复）：
  - DeleteConfirmModal 同款「恒深底」覆写全部移除——容器/标题/正文/
    代码卡/关闭/取消钮随主题令牌，遮罩回落 --mask，高危钮深红双层
    阴影改柔红晕
  - ExportDocsDialog 开关组与 CodeSnippetsPanel 代码井 `#0a0a0a`
    写死深底 → `--bg-code`；语法高亮 `.hl-n` 固定 `#c084fc` 浅底近
    不可读 → `--patch`
  - 主行动紫 token 化：新增 `--primary / --grad-primary(-hover) /
    --primary-glow(-hover)` 深浅成对令牌；rf-btn-primary、rf-btn-send、
    ProjectList（logo/CTA/新建钮）、EndpointTree 选中 pill、
    WorkspaceView 工具钮光晕全部消费——**浅色降一档刺眼**
   （violet-500→600→700 渐变 + 光晕 0.4→0.22）
  - SettingsDialog 主题/语言选中 chip、Tab 高亮条、更新卡按钮
    （bg-purple-600 字面）→ --primary 令牌 / rf-btn-primary 族
  - HeaderUpdateTag `shadow-purple-950/50` 脏影 → primary 光晕；
    ProjectList ghost 钮白 2% 底 → 主题文字 3% mix
  - themePattern spec 升级：黑名单拦截组件级恒深底 token 直写、
    `#0a0a0a` 代码井、亮紫填充/脏影字面量（注释剥离后判定）；
    stash 回退 6 组件实证恰 1 条失败
- 用例运行环境可见：用例面板工具栏挂 EnvBadge（语义色点 + 环境名 +
  Base URL）；用例详情抽屉在「用例基本信息」卡右上提供环境选择器
  （与顶栏同源 store.setEnvironment，可直接切换），底部操作栏只保留
  导出代码 / 立即运行 / 取消 / 保存核心动作——运行前即知且可改请求
  发往哪个环境（无激活环境回落「无环境」）

性能：

- 导出/备份/Mock 示例查询去 N+1（单次 IN 查询按接口分组）
- Mock 路由索引化（预编译 + (method, 段数) 分组 + 单次 query 解析）
- 压测改固定 worker 池（句柄数与 total 解耦）+ 超时透传 + 可取消 + 64KB body 上限
- 文件上传流式化（multipart/binary 边读边发，大文件不进内存）
- 备份恢复事务化（BEGIN IMMEDIATE，失败回滚）
- 请求热路径三查询同波次并发 + 历史裁剪节流 + seq 脏检查落盘 + 变量表单次合并
- 新增性能索引（test_runs、histories endpoint 过滤、mock_rules endpoint、folders parent）
- 响应渲染懒计算（树接管时跳过 stringify/切分）+ JsonTree O(1) 展开 + 查找 DOM 级激活
- 编辑器高亮防抖 + CodeMirror 回写防抖 + Body 纯文本镜像防抖
- 目录树共享子索引 + Tab 栏单遍求值 + 切 Tab 缓存命中跳过 IPC
- 压测图表 150ms 节流 + LTTB 抽样 + 首屏分包（vue/codemirror/chart 独立 chunk）

功能：

- 实时调试视图（WebSocket 收发/Ping/自动重连 + SSE 订阅/帧解析/续传 + 独立窗口弹出）
- 启动迁移前快照（保留 5 份）+ integrity_check 自检
- 环境导入导出（RustFox 原生 / Postman Environment）
- Cookie 管理（自管 Jar：侧栏查看/按域清理 + 单请求禁用）
- 历史搜索（关键字 + 状态筛选）+ Mock 热重载 + 故障注入（比例/状态码）
- 断言扩展（matches/regex、empty、graphql_errors、length）
- 删除撤销（Toast 8 秒）+ 目录树多选批量删除/移动
- 快捷键集中注册表 + 帮助面板（Ctrl+/）
- 设置页日志查看 + OpenAPI 3.1 导入（归一化为 3.0 子集）
- 备份扩域（全局设置快照 + 全局变量/参数保守合并）+ cURL 忽略参数预览 + Rust/PHP 代码导入
- 地址栏直粘 cURL 自动解析回填当前接口 + 测试用例列表关键字搜索（含 Body）+ 用例 Drawer 请求 Body 实时查找高亮
- 数据目录可配置（设置页更改/恢复默认/打开目录 + `RUSTFOX_DATA_DIR` 环境变量覆盖，重启生效）

修复：

- GraphQL 请求体 `mode` 序列化对齐前端：serde 对 `GraphQL` 变体的 snake_case 产物是 `graph_q_l`，
  与前端发送的 `graphql` 不一致，导致独立 GraphQL 视图 `execute_request` 直接报 unknown variant

## v0.0.26（2026-10-01）

- 实时调试视图补全：WebSocket 收发 / Ping / 自动重连，SSE 订阅与帧解析，独立窗口弹出
- 工作区功能缺口修复 + 核心导航可访问性、首屏骨架与定时器卫生
- 首屏性能：分包优化（vue / codemirror / chart 独立 chunk）与渲染热路径瘦身
- 发版链路：对齐 @tauri-apps/plugin-updater 2.12，消除 Tauri 构建版本校验失败

## v0.0.25（2026-09-24）

- 历史列表卡片化重构：方法 Tag、状态胶囊、慢请求耗时分级；恢复历史保留环境语义 URL
- AboutDialog 下载更新体验优化；Tab 计数徽章与侧栏激活态微调
- localStorage 测试垫片修复 jsdom 存储隔离；站点文档生成器 md → 静态 HTML

## v0.0.24（2026-09-23）

- 文档与官网默认英文；FieldDoc 结构化字段文档 sidecar
- 地址栏环境感知 Base URL 下拉；环境切换下拉重构（搜索/双行/快捷操作）
- MCP 启停端口与日志保留天数；cURL 环境前缀修复

## v0.0.23（2026-09-21）

- 标签管理菜单 + 新建按钮定位；新建空接口默认干净、关闭免二次确认
- 目录新建面板 + 菜单避让；目录树行间死区修复
- 仪表板导航去重 + 真实活跃时间 + 卡片去噪；移除请求栏模块选择器

## v0.0.22（2026-09-19）

- 数据目录可配置（设置页 + `RUSTFOX_DATA_DIR` 环境变量覆盖）
- 顶栏项目菜单分组整理；全仓 UI/字体一致性收敛

## v0.0.21（2026-09-15）

- 更新提醒优化 + dev 调试区
- 序列测试加串行锁消除并行互相干扰
- Apifox/Postman 迁移手册（中英）+ 官网入口；三 workspace 共用仓库根 target/

## v0.0.20（2026-09-12）

- 代码类 textarea Tab 缩进（焦点不跳出）

## v0.0.19（2026-09-11）

- 地址栏粘贴 cURL 自动识别回填当前接口
- 用例列表关键字搜索 + Drawer Body 实时查找
- 主密钥首调加互斥，消除并行解密失败

## v0.0.18（2026-09-05）

- 全量接入界面语言设置（i18n）；浅色主题变量驱动修复
- 定时检查更新与跳过此版本；发送请求交互反馈升级
- 设置页自定义全局快捷键；Body Tab 切换记住各类型内容
- 黑曜石主题换肤；方法色与硬编码收敛到设计变量

## v0.0.17（2026-09-04）

- 动态签名 config 缺失致崩屏修复（物化默认配置兜底）
- 响应正文复制无效与树接管空串修复

## v0.0.16（2026-09-04）

- 动态签名鉴权（App-Key/Secret/Timestamp/Sig）
- 冒烟文档导出（测试用例 → 冒烟测试文档）
- 接口请求结果按 id 隔离，切换接口不再串显示上个接口的响应

## v0.0.15（2026-09-02）

- 修复保存草稿时的对象浅克隆共享与 DOM 同步脱节问题

## v0.0.14（2026-09-02）

- 请求 Body 增加搜索功能与高亮修复；移除 10s 建连限制

## v0.0.13（2026-09-02）

- 设置页左侧导航极简风格与自增序列卡片化列表

## v0.0.12

- 主题三档：跟随系统 / 深色 / 浅色（含设置弹窗浅色适配，持久化 + 防闪烁）
- 请求超时可配置 + 自增序列管理 + 设置页重构
- 更新进度条累计修正；备份 / 文档导出改目录选择框
- dev 种子数据默认激活公网接口，开箱即可发送请求

## v0.0.11

- 环境全局化：多模块 Base URL + 全局变量/参数；默认模块随当前项目
- 环境管理器语义对齐（移除兜底单选列，修复保存双 toast / 行尾空条）

## v0.0.10

- 安装包内置 `rustfox-mcp` 侧载二进制（macOS/Windows/Linux）；MCP 文档更新为安装包内绝对路径
- 多项目标签页：顶栏快照切换，草稿/标签跨项目保留
- 响应 JSON 树默认全展开；请求区未发送时占满高度

## v0.0.9

- Agent 集成：本机控制面 HTTP API（4110 起探测）+ `rustfox-mcp` CLI + 前端事件刷新
- 新增 `docs/AGENT.md` 完整指南；官网落地页 + GitHub Pages 自动部署
- Mock 管理弹层渲染修复 + 回归测试

## v0.0.8

- 设计态 Schema 标注 + 多格式文档导出；全链路性能与健壮性优化
- 大响应/大文档渲染内存防护（GraphQL 上限、JsonTree 行数上限、编辑器大内容降级）
- 品牌图标资源更新

## v0.0.7

- 测试用例 Drawer 重构（Method 联动 / CodeMirror 6 / 拖拽分割）
- 项目卡片拖拽排序持久化；激活项目/环境持久化到 settings 表（重启恢复）
- URL 栏显示解析后域名；编辑器体验升级（历史侧栏化、多语言代码导入、binary 请求体）

## v0.0.6 及更早

- Cookie 自动回放 + 全局代理设置；请求历史按接口过滤
- 大响应渲染保护；开发/正式版数据目录隔离
- v0.0.3 起应用内自动更新（关于 → Check for Updates）
