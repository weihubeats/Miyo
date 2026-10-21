# Skills Manager（RustFox 内 Skills 面板）设计方案

> 状态：已实现（M1–M4；M5 仅剩界面人工验收）｜作者：AI 设计稿｜日期：2026-10-20
> 目标版本：v0.1（发现 / 列表 / 启停 / CRUD）

## 1. 背景与现状

同一台机器上，skill 文件散落在多个工具目录，真源不统一：

| 路径 | 实测状态 | 说明 |
| --- | --- | --- |
| `~/.agents/skill-hub/skills/` | 33 个真目录 | **实际真源** |
| `~/.claude/skills/` | 全部是指向 skill-hub 的软链接 | 结构正确 |
| `~/.config/opencode/skills/` | 真目录（caveman 系列） | **已分叉**，与真源内容无关联 |
| `~/.codex/skills/` | 只有 `.system` | 未安装任何 skill |
| `~/.gemini/extensions/` | 扩展格式（caveman） | 格式与 SKILL.md 不同 |
| `.agents/skills/`（本仓库） | push / release | 项目级真源，`.claude/skills` 为软链 |

痛点：

1. 没有统一视图：不知道某个 skill 在哪些工具生效、哪些已分叉；
2. 启停靠手工 `ln -s` / `rm`，易手滑删真源；
3. CRUD 必须 `cd` + 编辑器，frontmatter 写错只有等工具不识别才发现；
4. 分叉（opencode 真目录）静默存在，无人知晓。

## 2. 目标与非目标

**目标（v1）**

- 一个面板看清：所有 skill × 所有工具的生效状态；
- 启停 = 建链 / 断链，真源内容永不因启停移动；
- 新建 / 编辑 / 删除 SKILL.md（frontmatter 结构化表单 + Markdown 正文）；
- 状态以文件系统为准，不维护影子数据库。

**非目标（v2+）**

- 收编 / 导入已有分叉 skill（v2）；
- 通过 fox-agent HTTP / MCP 向 AI 暴露 skill 读写（v2）；
- 项目级 skill（`.agents/skills`）视图（v2，接口先按 scope 预留）；
- 远程仓库 / 市场 / 版本管理。

## 3. 核心概念

| 概念 | 定义 |
| --- | --- |
| **Skill** | 一个目录，含 `SKILL.md`（YAML frontmatter + 正文），可带 `scripts/ reference/ assets/` |
| **SourceRoot** | 真源根目录，有序（优先级从高到低），默认 `~/.agents/skills` |
| **Target** | 某个 agent 工具的 skill 目录 + 链接方式（adapter） |
| **Link** | SourceRoot/skill → Target/skill 的目录链接（Unix symlink / Windows junction） |
| **State** | 每个 (skill, target) 对的状态，见下 |

状态机（推导得出，不落库）：

```
Missing   目标目录里没有该条目
Linked    链接存在且 realpath == 真源
Drifted   目标里是实目录（自己一份内容）→ 冲突，不自动覆盖
Broken    链接存在但目标不存在（真源被删/移动）
Foreign   目标目录里有真源没有的 skill（只展示，v1 不处理）
```

**设计原则：文件系统即真相。** 面板每次打开 / 每次操作后重新扫描，不缓存数据库。
唯一持久化的是「SourceRoot 配置」和「工具目录覆盖」，落在 `settings` 键值里。

### 3.1 真源条目是链接（skill-hub 布局）

真源目录里允许存在**指向别处的符号链接**，例如：

```
~/.agents/skills/business-flow-common -> ~/.agents/skill-hub/skills/business-flow-common
```

这类条目与实体目录同等对待：

- `scan()` / `read()` 跟随链接读取真实内容，`summary.dir` 返回**解析后的真实目录**
  （UI「打开目录」直接落到内容所在处，编辑器顶部显示真实源目录）；
- `save()` 写穿链接，改的是链接目标里的真身，不会在真源里另建副本；
- `set_enabled()` 把工具链接指向真实目录，避免「链接指向链接」的两跳；
- `delete()` 移动**真实目录**进 trash，再清掉入口链接，不留悬空链接；
  若真身是主目录或真源的上层路径则拒绝删除（防止把整棵树搬走）；
- `existing_dir()` 的 realpath 越界校验只针对**实体目录**：链接条目跳过，
  因为写操作最终落在用户自己指定的真实目录上，拒绝它等于「链接布局下无法编辑」。

## 4. 目录与适配（adapter）

adapter 是静态表，编译期内置，运行期可被 settings 覆盖路径：

```rust
pub struct ToolSpec {
    pub id: &'static str,          // "claude-code"
    pub display_name: &'static str,
    pub kind: LinkKind,            // Symlink | Junction | CopyOnly
    pub verified: bool,            // 路径是否已实测确认
    pub global_roots: fn() -> Vec<PathBuf>,
    pub project_roots: fn(&Path) -> Vec<PathBuf>, // v2 使用
}

pub enum LinkKind { Symlink, Junction, CopyOnly }
```

| 工具 | global 根 | 链接方式 | 状态 |
| --- | --- | --- | --- |
| Claude Code | `~/.claude/skills` | Symlink | ✅ 已验证 |
| OpenCode | `~/.config/opencode/skills` | Symlink | ✅ 已验证（但常为实目录 → Drifted） |
| Codex | `~/.codex/skills` | Symlink | ⚠️ 目录存在，语义待验证 |
| Cursor | `~/.cursor/skills`（待确认） | 待定 | ❓ 需 spike |
| Gemini CLI | `~/.gemini/skills`（待确认） | 待定 | ❓ 需 spike（现有格式是 extension） |

- **Windows**：junction 不需要管理员权限，目录才支持（skill 是目录，正好）；
  `std::os::windows::fs::symlink_dir` 需要开发者模式，故 Windows 一律 junction。
- **链接失败**：v1 不静默降级为拷贝（避免制造新的分叉），直接返回
  `LINK_FAILED` 并展示原因；`CopyOnly` 工具单独标记为「拷贝模式」，UI 上明示。
- **相对/绝对**：一律写绝对路径，扫描时 `canonicalize` 后比较，容忍 `\\?\` 前缀差异。

## 5. 核心接口

### 5.1 新 crate `crates/fox-skill`（不依赖 tauri，可纯单测）

```rust
pub struct SkillIndex {
    pub skills: Vec<SkillSummary>,
    pub foreign: Vec<ForeignSkill>,   // 目标目录里有、真源没有
    pub tools: Vec<ToolInfo>,
    pub warnings: Vec<String>,
}

pub struct SkillSummary {
    pub name: String,
    pub description: String,          // frontmatter
    pub source_root: PathBuf,
    pub dir: PathBuf,
    pub updated_at: Option<i64>,
    pub states: Vec<TargetState>,     // 每个 target 一条
    pub extra_entries: Vec<String>,   // scripts/ 等
    pub warnings: Vec<SkillWarning>,  // name 与目录不一致、缺 description…
}

pub struct TargetState {
    pub tool_id: String,
    pub root: PathBuf,
    pub state: LinkState,             // Linked/Missing/Drifted/Broken/Foreign/CopyOnly
    pub detail: Option<String>,
}

pub struct SkillDoc {
    pub summary: SkillSummary,
    pub frontmatter: SkillFrontmatter, // name, description, 其他键原样保留
    pub body: String,                  // 正文（不含 frontmatter）
    pub raw: String,                   // 原文，保存时用于冲突检测
}

pub struct SkillInput {
    pub name: String,
    pub description: String,
    pub body: String,
    pub extra_frontmatter: serde_json::Map<String, Value>,
    pub raw_expected: Option<String>,  // 乐观锁：与当前文件不一致则 CONFLICT
}

pub trait SkillService {
    fn scan(&self) -> Result<SkillIndex, SkillError>;
    fn read(&self, name: &str) -> Result<SkillDoc, SkillError>;
    fn create(&self, input: SkillInput) -> Result<SkillSummary, SkillError>;
    fn save(&self, name: &str, input: SkillInput) -> Result<SkillSummary, SkillError>;
    fn delete(&self, name: &str) -> Result<(), SkillError>;
    fn set_enabled(&self, name: &str, tool_id: &str, enabled: bool) -> Result<SkillSummary, SkillError>;
    fn copy_mode(&self, name: &str, tool_id: &str) -> Result<(), SkillError>; // CopyOnly 工具用
}
```

安全约束（写路径前强制）：

- `name` 必须匹配 `^[a-z0-9][a-z0-9._-]{0,63}$`，且等于 frontmatter 的 `name`；
- 所有路径 `canonicalize` 后必须 `starts_with(source_root)`，拒绝 `..` / 绝对路径 / 符号链接逃逸；
- 原子写：写 `SKILL.md.tmp` → `fsync` → `rename`；正文大小上限 1 MiB；
- 删除 = 移动到 `{data_dir}/RustFox/skills-trash/<name>-<ts>/`（可手动找回）+ 断链；
- 所有写操作经 `AppState` 内 `Mutex` 串行化（单写入者原则，与 fox-agent 一致）。

### 5.2 Tauri Command（`crates/fox-tauri/src/commands/skill.rs`）

| Command | 入参 | 出参 | 错误码 |
| --- | --- | --- | --- |
| `skills_scan` | `scope?, projectDir?` | `SkillIndex` | `IO` |
| `skills_tools` | `scope?, projectDir?` | `ToolInfo[]` | `IO` |
| `skill_read` | `name, scope?, projectDir?` | `SkillDoc` | `NOT_FOUND` `IO` |
| `skill_create` | `input, scope?, projectDir?` | `SkillSummary` | `VALIDATION` `CONFLICT` |
| `skill_save` | `name, input, scope?, projectDir?` | `SkillSummary` | `VALIDATION` `CONFLICT` `IO` |
| `skill_delete` | `name, scope?, projectDir?` | `()` | `NOT_FOUND` `IO` |
| `skill_set_enabled` | `name, toolId, enabled, scope?, projectDir?` | `SkillSummary` | `NOT_FOUND` `CONFLICT`(Drifted) `LINK_FAILED` |
| `skill_validate_name` | `name` | `String` | `VALIDATION` |
| `skill_dir_path` | `name, scope?, projectDir?` | `String` | `NOT_FOUND` |

`scope` 缺省为 `global`；`project` 时必须给 `projectDir`（绝对路径）。
权限需同步登记到 `crates/fox-tauri/permissions/default.toml` 的 `allow-*` 列表。

### 5.3 前端

- 路由：`/skills` → `views/SkillsView.vue`；侧边栏加入口；
- store：`stores/skills.ts`（`index / selected / loading / filter`，操作后局部 rescan）；
- 列表列：名称 · 描述 · 真源 · 每个工具一个状态徽章（Linked 绿 / Missing 灰 / Drifted 黄 / Broken 红），
  点徽章直接切换该工具启停；
- 编辑弹窗 `components/skills/SkillEditorDialog.vue`：名称（编辑态只读，名称即目录名）+
  描述 + 正文编辑器（复用 `JsonCodeMirror` 的 `language="markdown"` 模式）+ 未知 frontmatter
  键原样透传；保存带原文乐观锁，`CONFLICT` 时提供「重新加载」动作；
- 删除走 `DeleteConfirmModal`，文案提示真源已进 `skill-trash`；
- i18n：`skills.*` 键，`zh.ts` / `en.ts` 1:1；组件 spec 固定 `zh` 语言；
- 禁止硬编码中文（`i18nPattern.spec.ts` 会扫）。

## 6. 交互时序

```
打开面板 / 手动刷新
  UI ──invoke skills_scan──> fox-skill::scan()
                              ├─ 枚举 SourceRoot 下的目录
                              ├─ 解析 frontmatter（serde_norway）
                              ├─ 枚举每个 ToolSpec 根，stat 条目
                              └─ 状态推导 → SkillIndex
  UI <────────────────────────┘

启停 skill × tool
  UI ──invoke skill_set_enabled(name, "claude-code", true)──>
        ├─ 校验 name / 路径
        ├─ 读目标条目：Linked 幂等返回；Drifted → CONFLICT
        ├─ 建链接（Unix symlink / Win junction）
        └─ rescan 单个 skill → SkillSummary
  UI <─────────────────────────┘ 只更新该行，不整表刷新

编辑
  UI ──invoke skill_read──> 填表单
  保存 ──invoke skill_save(name, input with raw_expected)──>
        ├─ 原文 != raw_expected → CONFLICT（外部被改，提示重载）
        ├─ 校验 frontmatter → 序列化 → 原子写
        └─ rescan → SkillSummary
```

外部改动（用户在终端删了目录）在窗口重新获得焦点时被动刷新一次；
不做 fs watcher（v1 避免新增平台依赖与性能抖动）。

## 7. 关键风险与最小验证

| # | 风险 | 一天内验证方式 |
| --- | --- | --- |
| R1 | Windows junction 行为（创建 / read_link / 删除）不符合预期 | CI `windows-skill` job 跑 `fox-skill` 全量测试；本机已用 `cargo check --target x86_64-pc-windows-msvc` 交叉验证编译 |
| R2 | Cursor / Codex / Gemini 的真实 skill 目录与格式不确定（表里 3 个 ❓） | Spike：每个工具放一个 probe skill（`name: probe`，正文打印），启动工具问「列出可用 skill」，记录结果后把 `verified` 置 true |
| R3 | 漂移误判（挂载点 / 大小写不敏感 / `\\?\` 前缀） | tempdir 单测：实目录、悬空链、指向别处的链、同名不同大小写 |
| R4 | 误删真源导致内容丢失 | 删除走 trash 目录 + 单测断言原目录已移入 trash |
| R5 | 路径逃逸写入任意位置 | 单测：name 含 `../`、目标根由用户配置成 `/` |
| R6 | mac 上 `~/.claude/skills` 已有 30 条软链，扫描性能 | 基准：1000 个 skill × 5 工具 < 300ms（一次 stat 即可，不做内容读取） |

## 8. 实现步骤

- [x] **M1 核心库**：`crates/fox-skill`（scan / 状态推导 / link 抽象 / frontmatter 解析 / 原子写）+ tempdir 单测（24 项全绿）；
- [x] **M2 命令层**：`commands/skill.rs` 9 个命令 + `invoke_handler` 注册 + `allow-*` 权限登记；
- [x] **M3 列表与启停**：`SkillsView` 列表 + 状态徽章 + 启停 + 作用域切换 + 路由/导航 + i18n；
- [x] **M4 CRUD**：`SkillEditorDialog`（frontmatter 表单 + Markdown 编辑器 + 乐观锁冲突提示）+ 删除确认；
- [x] **M5 收尾**：CI `windows-skill` job（`cargo test -p fox-skill`）+ 本机 `x86_64-pc-windows-msvc` 交叉 check；
  `is_link` 在 Windows 额外用 `junction::exists` 兜底，避免 junction 被误判为实体目录。
- [ ] **M5 剩余**：界面人工验收（真实 Tauri 运行时点一遍列表 / 启停 / 编辑 / 删除）。

依赖注入顺序：M1 可独立合并（无 UI 也绿），M2/M3 一组，M4 一组。

## 9. 已定案（2026-10-20）

1. **真源默认根**：标准 `~/.agents/skills`（不自动收编 `~/.agents/skill-hub`，避免隐式行为）；
2. **Windows 进 v1**：`junction` 为默认链接方式，`symlink_dir` 不作为默认路径；
3. **项目级 scope 进 v1**：`Scope::Project(dir)` → 真源 `<项目>/.agents/skills`，
   目标目录取各工具的项目级路径；
4. **编辑器复用**手写 CodeMirror 封装（M4 实现，增加 markdown 模式）。

仍需实测（不阻塞 M1，进入 M2 前跑）：

- Cursor / Codex / Gemini CLI 的真实 skill 目录与「是否跟随目录链接」；
- Windows junction 的 `is_symlink()` / `read_link` / 删除行为（M1 已有对应单测，需 CI windows job 验证）。

**已知限制**：

- v1 不支持在应用内改名（`save` 拒绝改名）；
- 真源条目是链接时，改名需要同时改「入口链接 + 链接目标」，暂未支持。

---

（附）**最小验证建议**：先砍 R1/R2 两棵树 —— 一天内把「junction 能不能用」和「三个工具的目录到底在哪」验证完，其余设计都是确定性工程。
