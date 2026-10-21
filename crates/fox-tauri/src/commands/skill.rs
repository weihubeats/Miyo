//! Skill 管理 Command：真源扫描、SKILL.md 读写、跨工具启停。
//!
//! 作用域由 `scope` + `projectDir` 两个参数表达：
//! - `scope = "global"`（默认）→ `~/.agents/skills`；
//! - `scope = "project"` → `<projectDir>/.agents/skills`，`projectDir` 必填。
//!
//! 所有写操作串行化（进程级互斥），避免并发建链 / 写文件互相踩踏；
//! `fox-skill` 的 [`SkillError::code`] 直接映射为前端 `err.code`。
//!
//! **为什么全是 `async fn`**：Tauri 里没有 `async` 关键字的命令跑在**主线程**上
//! （官方文档 *Calling Rust from the Frontend → Async Commands*：
//! “Commands without the *async* keyword are executed on the main thread”），
//! 而本模块每个命令都要做真磁盘 IO（扫描一次 ~10ms 可见工作 + 38×5 次状态探测）。
//! 同步执行会把 IPC 事件循环整段占住，表现为「首次进 skill 页面卡一下」。
//! 声明为 `async` 后自动落到 `sync_threadpool` 执行，UI 不再被阻塞。
//! 参数均为 owned 类型，因此不会踩到「async 命令不能借引用」的限制。

use std::path::PathBuf;
use std::sync::Mutex;

use fox_skill::{Scope, SkillDoc, SkillIndex, SkillInput, SkillManager, SkillSummary, ToolInfo};
use tauri::command;

use crate::error::{CommandError, CommandResult};

/// 写操作串行化（单写入者）。
static WRITE_LOCK: Mutex<()> = Mutex::new(());

/// 前端不感知内部错误码：`OutsideRoot` 映射为 NOT_FOUND，避免 toast 顶上
/// 一个大写 `FORBIDDEN`（链接布局下 expanded 目标落在真源之外是正常的）。
fn to_command_error(err: fox_skill::SkillError) -> CommandError {
    use fox_skill::SkillError;
    let code = match err {
        SkillError::OutsideRoot(_) => "NOT_FOUND",
        ref other => other.code(),
    };
    CommandError::with_code(code, err.to_string())
}

fn manager(scope: Option<String>, project_dir: Option<String>) -> CommandResult<SkillManager> {
    match scope.as_deref().unwrap_or("global") {
        "global" => SkillManager::global()
            .map(SkillManager::with_probe)
            .map_err(to_command_error),
        "project" => {
            let dir = project_dir
                .map(|dir| dir.trim().to_string())
                .filter(|dir| !dir.is_empty())
                .ok_or_else(|| CommandError::validation("项目级作用域需要 projectDir"))?;
            let path = PathBuf::from(&dir);
            if !path.is_dir() {
                return Err(CommandError::validation(format!("项目目录不存在：{dir}")));
            }
            SkillManager::for_scope(Scope::Project(path))
                .map(SkillManager::with_probe)
                .map_err(to_command_error)
        }
        other => Err(CommandError::validation(format!(
            "未知作用域：{other}（可选 global / project）"
        ))),
    }
}

fn with_write_lock<T>(run: impl FnOnce() -> Result<T, fox_skill::SkillError>) -> CommandResult<T> {
    let _guard = WRITE_LOCK
        .lock()
        .map_err(|_| CommandError::with_code("INTERNAL", "skill 写锁不可用"))?;
    run().map_err(to_command_error)
}

/// 扫描 skill：真源列表 + 各工具状态 + 外部（foreign）条目。
#[command(rename_all = "camelCase")]
pub async fn skills_scan(
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<SkillIndex> {
    Ok(manager(scope, project_dir)?.scan())
}

/// 工具清单（含根目录、链接方式、是否已实测确认）。
#[command(rename_all = "camelCase")]
pub async fn skills_tools(
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<Vec<ToolInfo>> {
    Ok(manager(scope, project_dir)?.tools())
}

/// 读取单个 skill（frontmatter + 正文 + 原文）。
#[command(rename_all = "camelCase")]
pub async fn skill_read(
    name: String,
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<SkillDoc> {
    manager(scope, project_dir)?
        .read(&name)
        .map_err(to_command_error)
}

/// 新建 skill（只写真源，不自动启用）。
#[command(rename_all = "camelCase")]
pub async fn skill_create(
    input: SkillInput,
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<SkillSummary> {
    let manager = manager(scope, project_dir)?;
    with_write_lock(|| manager.create(input))
}

/// 保存 skill（`input.rawExpected` 与磁盘不一致时返回 `CONFLICT`）。
#[command(rename_all = "camelCase")]
pub async fn skill_save(
    name: String,
    input: SkillInput,
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<SkillSummary> {
    let manager = manager(scope, project_dir)?;
    with_write_lock(|| manager.save(&name, input))
}

/// 删除 skill：断链 + 真源移入 `skill-trash/`。
#[command(rename_all = "camelCase")]
pub async fn skill_delete(
    name: String,
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<()> {
    let manager = manager(scope, project_dir)?;
    with_write_lock(|| manager.delete(&name))
}

/// 启停某工具下的 skill（建链 / 断链；`Drifted` 返回 `CONFLICT`）。
#[command(rename_all = "camelCase")]
pub async fn skill_set_enabled(
    name: String,
    tool_id: String,
    enabled: bool,
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<SkillSummary> {
    let manager = manager(scope, project_dir)?;
    with_write_lock(|| manager.set_enabled(&name, &tool_id, enabled))
}

/// 校验 skill 名称（前端新建表单实时校验用）。
#[command(rename_all = "camelCase")]
pub async fn skill_validate_name(name: String) -> CommandResult<String> {
    fox_skill::validate_name(&name).map_err(to_command_error)
}

/// 打开目录用的绝对路径（前端交给 opener 插件）。
#[command(rename_all = "camelCase")]
pub async fn skill_dir_path(
    name: String,
    scope: Option<String>,
    project_dir: Option<String>,
) -> CommandResult<String> {
    let manager = manager(scope, project_dir)?;
    let summary = manager.read(&name).map_err(to_command_error)?.summary;
    Ok(summary.dir.to_string_lossy().to_string())
}
