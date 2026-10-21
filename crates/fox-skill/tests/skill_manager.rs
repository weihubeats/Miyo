//! fox-skill 集成测试：真源扫描 / 链接启停 / 冲突保护 / 删除回退。

use std::fs;
use std::path::{Path, PathBuf};

use fox_skill::tools::TOOLS;
use fox_skill::{
    validate_name, LinkState, Scope, SkillError, SkillInput, SkillManager, SkillSummary,
};

fn input(name: &str, description: &str, body: &str) -> SkillInput {
    SkillInput {
        name: name.into(),
        description: description.into(),
        body: body.into(),
        ..Default::default()
    }
}

/// 测试布局：真源 + Claude Code + Codex 两个目标。
fn layout(tmp: &Path) -> (SkillManager, PathBuf, PathBuf, PathBuf) {
    let source = tmp.join(".agents/skills");
    let trash = tmp.join(".agents/skill-trash");
    let claude = tmp.join(".claude/skills");
    let codex = tmp.join(".codex/skills");
    let manager = SkillManager::with_layout(
        Scope::Global,
        source.clone(),
        trash.clone(),
        vec![(&TOOLS[0], claude.clone()), (&TOOLS[2], codex.clone())],
    );
    (manager, source, claude, codex)
}

fn state_of(summary: &SkillSummary, tool_id: &str) -> LinkState {
    summary
        .states
        .iter()
        .find(|state| state.tool_id == tool_id)
        .unwrap_or_else(|| panic!("缺少工具 {tool_id} 的状态"))
        .state
}

#[test]
fn create_writes_frontmatter_and_body() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, _, _) = layout(tmp.path());

    let summary = manager
        .create(input(
            "pr-review",
            "按团队规范审查 PR",
            "# PR Review\n\n步骤一\n",
        ))
        .unwrap();

    assert_eq!(summary.name, "pr-review");
    assert_eq!(summary.description, "按团队规范审查 PR");
    let raw = fs::read_to_string(source.join("pr-review/SKILL.md")).unwrap();
    assert!(raw.starts_with("---\n"));
    assert!(raw.contains("name: pr-review"));
    assert!(raw.contains("# PR Review"));

    let doc = manager.read("pr-review").unwrap();
    assert_eq!(doc.frontmatter.name, "pr-review");
    assert_eq!(doc.body.trim(), "# PR Review\n\n步骤一");

    let index = manager.scan();
    assert_eq!(index.skills.len(), 1);
    assert_eq!(index.skills[0].name, "pr-review");
    // 新建不自动启用
    assert_eq!(
        state_of(&index.skills[0], "claude-code"),
        LinkState::Missing
    );
}

#[test]
fn rejects_unsafe_names() {
    for name in [
        "",
        "../evil",
        "a/b",
        ".hidden",
        "ends.",
        "a..b",
        "空格 name",
        "x".repeat(65).as_str(),
    ] {
        assert!(validate_name(name).is_err(), "名称 {name:?} 应被拒绝");
    }
    assert_eq!(validate_name("ok-name_1.2").unwrap(), "ok-name_1.2");
}

#[test]
fn duplicate_create_is_conflict() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, _, _) = layout(tmp.path());
    manager.create(input("dup", "d", "b")).unwrap();
    let err = manager.create(input("dup", "d", "b")).unwrap_err();
    assert!(matches!(err, SkillError::AlreadyExists(_)));
}

#[test]
fn empty_description_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, _, _) = layout(tmp.path());
    let err = manager.create(input("no-desc", "  ", "b")).unwrap_err();
    assert!(matches!(err, SkillError::Frontmatter(_)));
}

#[test]
fn enable_and_disable_link() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, claude, _) = layout(tmp.path());
    manager.create(input("demo", "演示", "正文")).unwrap();

    let summary = manager.set_enabled("demo", "claude-code", true).unwrap();
    assert_eq!(state_of(&summary, "claude-code"), LinkState::Linked);

    let link = claude.join("demo");
    assert!(fox_skill::link::is_link(&link));
    assert_eq!(
        fs::canonicalize(&link).unwrap(),
        fs::canonicalize(source.join("demo")).unwrap()
    );

    // 幂等：重复启用不报错
    let again = manager.set_enabled("demo", "claude-code", true).unwrap();
    assert_eq!(state_of(&again, "claude-code"), LinkState::Linked);

    let disabled = manager.set_enabled("demo", "claude-code", false).unwrap();
    assert_eq!(state_of(&disabled, "claude-code"), LinkState::Missing);
    assert!(!claude.join("demo").exists());
    // 真源内容不受影响
    assert!(source.join("demo/SKILL.md").exists());
}

#[test]
fn drifted_directory_is_never_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, claude, _) = layout(tmp.path());
    manager.create(input("demo", "演示", "真源内容")).unwrap();

    let drifted = claude.join("demo");
    fs::create_dir_all(&drifted).unwrap();
    fs::write(
        drifted.join("SKILL.md"),
        "---\nname: demo\ndescription: 分叉副本\n---\n",
    )
    .unwrap();

    let index = manager.scan();
    assert_eq!(
        state_of(&index.skills[0], "claude-code"),
        LinkState::Drifted
    );

    let err = manager
        .set_enabled("demo", "claude-code", true)
        .unwrap_err();
    assert!(matches!(err, SkillError::Conflict(_)));
    let err = manager
        .set_enabled("demo", "claude-code", false)
        .unwrap_err();
    assert!(matches!(err, SkillError::Conflict(_)));

    // 分叉副本原封不动
    assert!(fs::read_to_string(drifted.join("SKILL.md"))
        .unwrap()
        .contains("分叉副本"));
}

#[cfg(unix)]
#[test]
fn broken_link_is_reported_and_repaired() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, claude, _) = layout(tmp.path());
    manager.create(input("demo", "演示", "正文")).unwrap();
    fs::create_dir_all(&claude).unwrap();

    // 指向不存在的目录 → Broken
    symlink(tmp.path().join("ghost"), claude.join("demo")).unwrap();
    let index = manager.scan();
    assert_eq!(state_of(&index.skills[0], "claude-code"), LinkState::Broken);

    let repaired = manager.set_enabled("demo", "claude-code", true).unwrap();
    assert_eq!(state_of(&repaired, "claude-code"), LinkState::Linked);
    assert!(fs::canonicalize(claude.join("demo")).is_ok());
}

/// 在 `at` 建一个指向 `target` 的目录链接（Unix symlink / Windows junction）。
fn link_dir(target: &Path, at: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, at).unwrap();
    #[cfg(windows)]
    junction::create(target, at).unwrap();
}

/// 真源条目可以是指向别处的链接（skill-hub 布局）：读、改都必须落到真实内容上。
#[test]
fn symlinked_source_entry_is_editable_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, claude, _) = layout(tmp.path());

    let hub = tmp.path().join(".agents/skill-hub/skills/demo");
    fs::create_dir_all(&hub).unwrap();
    fs::write(
        hub.join("SKILL.md"),
        "---\nname: demo\ndescription: 旧描述\n---\n\n旧正文\n",
    )
    .unwrap();
    fs::create_dir_all(&source).unwrap();
    link_dir(&hub, &source.join("demo"));

    // 读：跟随链接拿到真实内容，并暴露真实目录。
    let doc = manager.read("demo").unwrap();
    assert_eq!(doc.frontmatter.description, "旧描述");
    assert_eq!(doc.summary.dir, fs::canonicalize(&hub).unwrap());

    // 写：写穿链接，改的是 hub 里的真身，而不是在真源里另建副本。
    manager
        .save(
            "demo",
            SkillInput {
                name: "demo".into(),
                description: "新描述".into(),
                body: "新正文\n".into(),
                raw_expected: Some(doc.raw),
                ..Default::default()
            },
        )
        .unwrap();
    let raw = fs::read_to_string(hub.join("SKILL.md")).unwrap();
    assert!(raw.contains("新描述"));
    // 入口仍是链接（没有被改写成真源里的实体副本）。
    assert_eq!(
        fs::canonicalize(source.join("demo")).unwrap(),
        fs::canonicalize(&hub).unwrap()
    );

    // 启停：目标工具链接指向真实目录，而不是真源里的入口链接。
    manager.set_enabled("demo", "claude-code", true).unwrap();
    assert_eq!(
        fs::canonicalize(claude.join("demo")).unwrap(),
        fs::canonicalize(&hub).unwrap()
    );
}

/// 删除链接条目时移动真实内容进 trash，并清掉入口链接（不留悬空链接）。
#[test]
fn delete_symlinked_entry_moves_real_source_to_trash() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, claude, _) = layout(tmp.path());

    let hub = tmp.path().join(".agents/skill-hub/skills/demo");
    fs::create_dir_all(&hub).unwrap();
    fs::write(hub.join("SKILL.md"), "---\nname: demo\n---\n\n正文\n").unwrap();
    fs::create_dir_all(&source).unwrap();
    link_dir(&hub, &source.join("demo"));
    manager.set_enabled("demo", "claude-code", true).unwrap();

    manager.delete("demo").unwrap();

    assert!(!source.join("demo").exists());
    assert!(!hub.exists());
    assert!(!claude.join("demo").exists());
    let trash: Vec<_> = fs::read_dir(tmp.path().join(".agents/skill-trash"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(trash.len(), 1);
    assert!(trash[0].path().join("SKILL.md").exists());
}

#[test]
fn save_rejects_external_modification() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, _, _) = layout(tmp.path());
    manager.create(input("demo", "演示", "旧正文")).unwrap();
    let doc = manager.read("demo").unwrap();

    // 模拟外部编辑
    fs::write(
        source.join("demo/SKILL.md"),
        "---\nname: demo\ndescription: 外部改过\n---\n\n新正文\n",
    )
    .unwrap();

    let err = manager
        .save(
            "demo",
            SkillInput {
                name: "demo".into(),
                description: "我的修改".into(),
                body: "我的正文".into(),
                raw_expected: Some(doc.raw),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, SkillError::Conflict(_)));

    // 拿到最新原文后可以保存
    let fresh = manager.read("demo").unwrap();
    manager
        .save(
            "demo",
            SkillInput {
                name: "demo".into(),
                description: "我的修改".into(),
                body: "我的正文".into(),
                raw_expected: Some(fresh.raw),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        manager.read("demo").unwrap().frontmatter.description,
        "我的修改"
    );
}

#[test]
fn save_rejects_rename() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, _, _) = layout(tmp.path());
    manager.create(input("demo", "演示", "正文")).unwrap();
    let err = manager
        .save(
            "demo",
            SkillInput {
                name: "other".into(),
                description: "d".into(),
                body: "b".into(),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidName(_)));
}

#[test]
fn delete_moves_source_to_trash_and_unlinks() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, claude, codex) = layout(tmp.path());
    manager.create(input("demo", "演示", "正文")).unwrap();
    manager.set_enabled("demo", "claude-code", true).unwrap();
    manager.set_enabled("demo", "codex", true).unwrap();

    manager.delete("demo").unwrap();

    assert!(!source.join("demo").exists());
    assert!(!claude.join("demo").exists());
    assert!(!codex.join("demo").exists());

    let trash = tmp.path().join(".agents/skill-trash");
    let entries: Vec<_> = fs::read_dir(&trash).unwrap().flatten().collect();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].path().join("SKILL.md").exists());
}

#[test]
fn delete_leaves_drifted_copy_untouched() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, claude, _) = layout(tmp.path());
    manager.create(input("demo", "演示", "正文")).unwrap();

    let drifted = claude.join("demo");
    fs::create_dir_all(&drifted).unwrap();
    fs::write(drifted.join("SKILL.md"), "copied").unwrap();

    manager.delete("demo").unwrap();
    assert!(drifted.join("SKILL.md").exists());
    assert_eq!(
        fs::read_to_string(drifted.join("SKILL.md")).unwrap(),
        "copied"
    );
}

#[test]
fn foreign_skills_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, claude, _) = layout(tmp.path());
    fs::create_dir_all(claude.join("caveman")).unwrap();
    fs::write(claude.join("caveman/SKILL.md"), "x").unwrap();

    let index = manager.scan();
    assert!(index.skills.is_empty());
    assert_eq!(index.foreign.len(), 1);
    assert_eq!(index.foreign[0].name, "caveman");
    assert_eq!(index.foreign[0].tool_id, "claude-code");
}

#[test]
fn name_mismatch_produces_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, source, _, _) = layout(tmp.path());
    let dir = source.join("dir-name");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        "---\nname: other-name\ndescription: d\n---\n\nbody\n",
    )
    .unwrap();

    let index = manager.scan();
    let warnings = &index.skills[0].warnings;
    assert!(warnings.iter().any(|w| w.code == "nameMismatch"));
}

#[test]
fn missing_source_root_only_warns() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, _, _) = layout(tmp.path());
    let index = manager.scan();
    assert!(index.skills.is_empty());
    assert!(index.warnings.iter().any(|w| w.code == "sourceMissing"));
    // 工具目录不存在也只是 present=false
    assert!(index.tools.iter().all(|tool| !tool.present));
}

#[test]
fn extra_frontmatter_keys_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, _, _) = layout(tmp.path());
    let mut extra = std::collections::BTreeMap::new();
    extra.insert(
        "allowed-tools".to_string(),
        serde_json::json!("Read, Write"),
    );

    manager
        .create(SkillInput {
            name: "meta".into(),
            description: "带额外字段".into(),
            body: "正文".into(),
            extra,
            raw_expected: None,
        })
        .unwrap();

    let doc = manager.read("meta").unwrap();
    assert_eq!(
        doc.frontmatter.extra.get("allowed-tools"),
        Some(&serde_json::json!("Read, Write"))
    );
}

#[test]
fn project_scope_uses_project_directories() {
    let tmp = tempfile::tempdir().unwrap();
    let manager = SkillManager::for_scope(Scope::Project(tmp.path().to_path_buf())).unwrap();

    assert_eq!(manager.source_root(), tmp.path().join(".agents/skills"));
    let roots: Vec<PathBuf> = manager.tools().into_iter().map(|tool| tool.root).collect();
    assert!(roots.contains(&tmp.path().join(".claude/skills")));
    assert!(roots.contains(&tmp.path().join(".opencode/skills")));
}

#[test]
fn rejects_path_traversal_via_tool_call() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, _, _) = layout(tmp.path());
    let err = manager
        .set_enabled("../etc", "claude-code", true)
        .unwrap_err();
    assert!(matches!(err, SkillError::InvalidName(_)));
}

/// 目录存在即视为可用：内置表默认未实测的工具，探测到实处时放宽 verified。
#[test]
fn probe_marks_present_tool_roots_as_verified() {
    let tmp = tempfile::tempdir().unwrap();
    let (manager, _, claude, _codex) = layout(tmp.path());
    fs::create_dir_all(&claude).unwrap();

    let probed = manager.clone().with_probe();
    let tools = probed.tools();
    let claude_info = tools.iter().find(|tool| tool.id == "claude-code").unwrap();
    let codex_info = tools.iter().find(|tool| tool.id == "codex").unwrap();

    assert!(claude_info.present && claude_info.verified);
    assert!(!codex_info.present);
    assert!(!codex_info.verified);
}
