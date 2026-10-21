//! 前端插件调用的能力契约测试：前端 import 的每个插件 API，必须在
//! `capabilities/*.json` 里有对应权限，否则运行时被拒
//! （`opener.open_path not allowed. Permissions associated with this command: …`）。
//!
//! 背景：`permissions_consistency.rs` / `permissions_contract_test.rs` 只校验
//! **fox 插件自己**的命令清单；而核心与第三方插件（opener / dialog / process /
//! updater）的权限写在 capabilities 里，两边都没有测试覆盖。SkillsView 因此
//! 踩坑：用了 `openPath()`，但 `opener:default` 只放行 `open-url` 与
//! `reveal-item-in-dir`，不含 `open-path`，点「打开目录」必然报 not allowed。
//!
//! 本测试把「前端调用」与「能力声明」对起来，新增插件 API 忘了配权限即刻红灯。

use std::collections::BTreeSet;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // crates/fox-tauri → 仓库根；capabilities 由 app 壳 frontend/src-tauri 持有。
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("定位仓库根目录")
        .to_path_buf()
}

/// app 壳目录（capabilities 的实际位置）：`frontend/src-tauri`。
fn app_dir() -> PathBuf {
    repo_root().join("frontend/src-tauri")
}

/// capabilities 目录不存在时跳过（例如在独立的 app 工作区里单独跑）。
fn capabilities_dir() -> Option<PathBuf> {
    let dir = app_dir().join("capabilities");
    dir.is_dir().then_some(dir)
}

/// 递归收集前端 src 下的 .vue / .ts（跳过单测与生成产物）。
fn frontend_sources(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if matches!(name.as_str(), "node_modules" | "dist" | "i18n") {
                continue;
            }
            frontend_sources(&path, out);
        } else if (name.ends_with(".vue") || name.ends_with(".ts")) && !name.ends_with(".spec.ts") {
            out.push(path);
        }
    }
}

/// 前端实际调用的插件 API：`(插件名, 方法名)`。
///
/// 只认值导入（`import { x } from '@tauri-apps/plugin-y'`）；
/// `import type` 是纯类型，不产生 IPC 调用，跳过。
fn frontend_plugin_calls() -> BTreeSet<(String, String)> {
    let src = repo_root().join("frontend/src");
    let mut files = Vec::new();
    frontend_sources(&src, &mut files);

    // opener 的插件名是 `tauri-plugin-opener`，但权限前缀是 `opener`。
    let mut calls = BTreeSet::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for line in text.lines() {
            let line = line.trim();
            // 跳过 `import type { … }` 与注释行
            if !line.starts_with("import ") || line.starts_with("import type") {
                continue;
            }
            let Some(at) = line.find("@tauri-apps/plugin-") else {
                continue;
            };
            // 取 `@tauri-apps/plugin-` 之后的插件名
            let after = &line[at + "@tauri-apps/plugin-".len()..];
            let plugin: String = after
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect();
            if plugin.is_empty() {
                continue;
            }
            // 取花括号里的具名导入
            let Some(open) = line.find('{') else { continue };
            let Some(close) = line[open..].find('}') else {
                continue;
            };
            let names = &line[open + 1..open + close];
            for raw in names.split(',') {
                // `open as openDialog` → open；去掉类型缀与空白
                let ident = raw
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if ident.is_empty() || ident == "type" {
                    continue;
                }
                calls.insert((plugin.clone(), ident));
            }
        }
    }
    calls
}

/// 读取 capabilities 下所有权限标识。
fn declared_permissions() -> Option<BTreeSet<String>> {
    let dir = capabilities_dir()?;
    let mut out = BTreeSet::new();
    for entry in std::fs::read_dir(&dir)
        .expect("读取 capabilities 目录")
        .flatten()
    {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        // 朴素提取 JSON 字符串字面量即可：权限标识都在 permissions 数组里。
        let mut rest = text.as_str();
        while let Some(start) = rest.find('"') {
            let after = &rest[start + 1..];
            let Some(end) = after.find('"') else { break };
            out.insert(after[..end].to_string());
            rest = &after[end + 1..];
        }
    }
    Some(out)
}

/// 插件方法名 → 权限标识（kebab-case）：`openPath` → `allow-open-path`。
fn permission_for(plugin: &str, method: &str) -> String {
    format!("{plugin}:allow-{}", kebab(method))
}

/// camelCase / PascalCase → kebab-case。
fn kebab(method: &str) -> String {
    let mut out = String::new();
    for (i, ch) in method.char_indices() {
        if ch.is_ascii_uppercase() {
            if i != 0 {
                out.push('-');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// 少数 JS API 名与底层 IPC 命令名不同：按源码里的 `invoke('plugin:x|y')` 映射。
///
/// 例：`@tauri-apps/plugin-process` 的 `relaunch()` 实际调用 `plugin:process|restart`，
/// 因此权限是 `allow-restart` 而不是 `allow-relaunch`（该插件根本没有 relaunch 命令）。
fn command_alias(plugin: &str, method: &str) -> Option<&'static str> {
    match (plugin, method) {
        ("process", "relaunch") => Some("restart"),
        _ => None,
    }
}

/// 该方法对应的候选权限标识（含别名解析）。
fn candidate_permissions(plugin: &str, method: &str) -> Vec<String> {
    let mut out = vec![permission_for(plugin, method)];
    if let Some(alias) = command_alias(plugin, method) {
        out.push(permission_for(plugin, alias));
    }
    out
}

/// 从 cargo registry 里读取指定插件当前版本的 `permissions/default.toml`，
/// 解析其 `permissions = [...]` 列表。
fn default_set_permissions(plugin: &str) -> Vec<String> {
    let Some(toml) = find_plugin_default_toml(plugin) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(toml) else {
        return Vec::new();
    };
    // 只取 [default] 段之后的权限列表：值为形如 "allow-xxx" 的裸标识。
    let body = match text.find("[default]") {
        Some(at) => &text[at..],
        None => return Vec::new(),
    };
    extract_quoted(body)
        .into_iter()
        .filter(|s| s.starts_with("allow-"))
        .map(|s| format!("{plugin}:{s}"))
        .collect()
}

/// 在 cargo registry 中定位插件的 `permissions/default.toml`（取最高版本）。
fn find_plugin_default_toml(plugin: &str) -> Option<PathBuf> {
    // dart-style 名称：opener 插件的 crate 名是 tauri-plugin-opener。
    let crate_name = format!("tauri-plugin-{plugin}");
    let home = std::env::var("HOME").ok()?;
    let registry = PathBuf::from(home).join(".cargo/registry/src");
    let mut best: Option<(Vec<u64>, PathBuf)> = None;
    for root in std::fs::read_dir(&registry).ok()?.flatten() {
        let base = root.path();
        if !base.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&base) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            // 形如 tauri-plugin-opener-2.7.0
            let Some(rest) = name.strip_prefix(&format!("{crate_name}-")) else {
                continue;
            };
            let version: Vec<u64> = rest
                .split('.')
                .filter_map(|part| part.parse::<u64>().ok())
                .collect();
            let candidate = entry.path().join("permissions/default.toml");
            if !candidate.is_file() {
                continue;
            }
            if best.as_ref().is_none_or(|(v, _)| version > *v) {
                best = Some((version, candidate));
            }
        }
    }
    best.map(|(_, path)| path)
}

/// 提取文本中所有 `"..."` 字符串字面量。
fn extract_quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            let start = i + 1;
            if let Some(rel) = text[start..].find('"') {
                out.push(text[start..start + rel].to_string());
                i = start + rel + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// 命令虽然放行了，但还需 scope 才真正可用：`open_path` 会做 fs scope 校验，
/// 裸标识（无 `allow` 列表）等于「什么路径都不许」。
fn requires_path_scope(plugin: &str, method: &str) -> bool {
    plugin == "opener" && method == "openPath"
}

#[test]
fn every_frontend_plugin_call_is_permitted() {
    let Some(declared) = declared_permissions() else {
        eprintln!("跳过：未找到 frontend/src-tauri/capabilities（独立工作区运行）");
        return;
    };

    let calls = frontend_plugin_calls();
    assert!(
        calls.len() >= 5,
        "前端插件调用解析异常，命中过少：{calls:?}"
    );

    let mut missing = Vec::new();
    for (plugin, method) in &calls {
        let candidates = candidate_permissions(plugin, method);
        // 同名 allow 标识（如 opener:allow-open-path），或别名命令的标识
        if candidates.iter().any(|perm| declared.contains(perm)) {
            // 放行了命令还不算数：需要 scope 的命令必须真的带 scope。
            if requires_path_scope(plugin, method)
                && !candidates
                    .iter()
                    .any(|perm| permission_has_scope(perm.as_str()))
            {
                missing.push(format!(
                    "{plugin}.{method} → 已放行但缺 path scope（任何路径都会被拒）"
                ));
            }
            continue;
        }
        // 或者被插件 default 权限集覆盖（读插件真实 default.toml 判定）
        if declared.contains(&format!("{plugin}:default"))
            && candidates.iter().any(|perm| {
                default_set_permissions(plugin)
                    .iter()
                    .any(|covered| covered == perm)
            })
        {
            continue;
        }
        missing.push(format!("{plugin}.{method} → 缺少 {}", candidates[0]));
    }

    assert!(
        missing.is_empty(),
        "以下前端插件调用没有任何能力声明覆盖，运行时会被拒（not allowed）：\n  {}\n\
         请在 frontend/src-tauri/capabilities/default.json 的 permissions 中补充对应权限。",
        missing.join("\n  ")
    );
}

#[test]
fn opener_open_path_has_path_scope() {
    // 回归：`opener:allow-open-path` 只放行命令，open_path 还会做 fs scope 校验。
    // 早期只写了裸标识（无 scope），导致任何路径都被拒：
    //   Not allowed to open path /Users/…/.agents/skill-hub/skills/blueprint-check
    let Some(text) = read_capability_containing("opener:allow-open-path") else {
        panic!("capabilities 必须显式放行 opener:allow-open-path");
    };
    assert!(
        text.contains("\"allow\"") && text.contains("$HOME"),
        "opener:allow-open-path 必须带 path scope（仅裸标识会拒绝所有路径）；\
         且需覆盖 $HOME 下的 skill 真源目录"
    );
}

/// 关键陷阱回归：Unix 下 opener 默认 `requireLiteralLeadingDot = true`，
/// 此时任何 glob 都匹配不到带 `.` 组件的路径 —— 而 skill 真源全在 `~/.agents`、
/// `~/.claude` 这类点目录下，scope 写了也等于没写。
#[test]
fn opener_allows_dot_directories() {
    let s = read_app_file("tauri.conf.json");
    // 必须显式关掉「点号必须字面匹配」，否则 $HOME/.agents/** 永远匹配不上。
    assert!(
        s.contains("\"requireLiteralLeadingDot\"") && s.contains("false"),
        "tauri.conf.json 的 plugins.opener.requireLiteralLeadingDot 必须为 false，\
         否则 open_path 的 scope 无法匹配 ~/.agents、~/.claude 等点目录（Unix 默认 true）"
    );
}

/// 该权限标识在 capabilities 里是否带了 scope（即条目是
/// `{ "identifier": …, "allow": [...] }` 对象形式，而不是裸字符串）。
fn permission_has_scope(perm: &str) -> bool {
    let Some(dir) = capabilities_dir() else {
        return false;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        // 先按 JSON 解析，避免字符串扫描把注释里的标识当成声明。
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(list) = value.get("permissions").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in list {
            let Some(obj) = item.as_object() else {
                continue;
            };
            if obj.get("identifier").and_then(|v| v.as_str()) != Some(perm) {
                continue;
            }
            // 对象形式：看 allow 是否非空
            let has_allow = obj
                .get("allow")
                .and_then(|v| v.as_array())
                .is_some_and(|a| !a.is_empty());
            if has_allow {
                return true;
            }
        }
    }
    false
}

/// 读取含指定标识的 capabilities 文件全文。
fn read_capability_containing(needle: &str) -> Option<String> {
    let dir = capabilities_dir()?;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let Ok(text) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        if text.contains(needle) {
            return Some(text);
        }
    }
    None
}

/// 读取 app 壳下的配置文件（如 tauri.conf.json）。
fn read_app_file(rel: &str) -> String {
    let path = app_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读取 {} 失败：{e}", path.display()))
}
