//! 插件依赖解析（manifest.plugin_deps）：安装 / 启用前的依赖满足性检查。
//!
//! 检查项（对应插件系统 2.0 依赖管理要求）：
//! - **循环依赖**：DFS 检测（含目标自身的传递依赖图）；
//! - **缺少依赖**：声明了未安装的插件；
//! - **版本过低**：依赖已装但版本 < 要求（plugin_deps 值 = 最低版本 x.y.z）；
//! - **依赖停用**：已装但未启用（启用目标前依赖必须全部可用）；
//! - **依赖不兼容**：依赖自身 min_app_version 与宿主版本不符（由调用方传入
//!   宿主版本后逐层校验）。
//!
//! miyo-plugin 保持纯逻辑：通过闭包注入「按 id 查插件」——存储细节留在
//! miyo-tauri / miyo-storage。

use std::collections::BTreeSet;

use miyo_core::AppError;

use crate::manifest::PluginManifest;

/// 依赖条目的运行时状态（调用方从存储层填充）。
#[derive(Debug, Clone)]
pub struct DepInfo {
    pub manifest: PluginManifest,
    /// 是否已安装。
    pub installed: bool,
    /// 是否已启用。
    pub enabled: bool,
}

/// 启用前依赖检查。
///
/// * `target`：待启用的插件清单；
/// * `lookup`：按 id 取依赖状态（未知 id 返回 None = 未安装）；
/// * `app_version`：宿主版本（逐层兼容性检查基准）。
///
/// 返回启用前必须按序启用的依赖链（拓扑序，不含 target 自身）。
pub fn check_enable(
    target: &PluginManifest,
    lookup: &dyn Fn(&str) -> Option<DepInfo>,
    app_version: &str,
) -> Result<Vec<String>, AppError> {
    let mut order: Vec<String> = Vec::new();
    let mut visiting: BTreeSet<String> = BTreeSet::new();
    let mut visited: BTreeSet<String> = BTreeSet::new();
    visit(
        target,
        lookup,
        app_version,
        &mut visiting,
        &mut visited,
        &mut order,
    )?;
    Ok(order)
}

/// DFS：visit 目标的全部依赖（先依赖后自身——拓扑序）。
fn visit(
    target: &PluginManifest,
    lookup: &dyn Fn(&str) -> Option<DepInfo>,
    app_version: &str,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    order: &mut Vec<String>,
) -> Result<(), AppError> {
    let deps = target.plugin_deps.clone().unwrap_or_default();
    for (dep_id, required_version) in &deps {
        if visiting.contains(dep_id) {
            return Err(AppError::Validation(format!(
                "检测到循环依赖：{dep_id}（链路含 {}）",
                visiting.iter().cloned().collect::<Vec<_>>().join(" → ")
            )));
        }
        if visited.contains(dep_id) {
            continue; // 已验证过（钻石依赖合法）
        }
        let Some(info) = lookup(dep_id) else {
            return Err(AppError::Validation(format!(
                "缺少依赖插件：{dep_id}（要求 ≥ {required_version}）——请先在插件市场安装"
            )));
        };
        if !info.installed {
            return Err(AppError::Validation(format!(
                "缺少依赖插件：{dep_id}（要求 ≥ {required_version}）"
            )));
        }
        // 版本过低（逐段比较，与兼容性同口径）
        crate::compat::ensure_compatible(dep_id, required_version, &info.manifest.version)
            .map_err(|e| {
                AppError::Validation(format!(
                    "依赖版本过低：{dep_id} {}（要求 ≥ {required_version}）——{}",
                    info.manifest.version,
                    e.user_message()
                ))
            })?;
        // 依赖自身与宿主的兼容性
        crate::compat::ensure_compatible(dep_id, &info.manifest.min_app_version, app_version)
            .map_err(|e| {
                AppError::Validation(format!(
                    "依赖插件与宿主不兼容：{dep_id}（{}）",
                    e.user_message()
                ))
            })?;
        if !info.enabled {
            return Err(AppError::Validation(format!(
                "依赖插件已停用：{dep_id}——请先启用 {dep_id}"
            )));
        }
        visiting.insert(dep_id.clone());
        visit(
            &info.manifest,
            lookup,
            app_version,
            visiting,
            visited,
            order,
        )?;
        visiting.remove(dep_id);
        visited.insert(dep_id.clone());
        order.push(dep_id.clone());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, version: &str, deps: &[(&str, &str)]) -> PluginManifest {
        let deps_json: Vec<String> = deps
            .iter()
            .map(|(id, v)| format!("\"{id}\": \"{v}\""))
            .collect();
        let deps_field = if deps.is_empty() {
            String::new()
        } else {
            format!(r#", "plugin_deps": {{{}}}"#, deps_json.join(","))
        };
        let json = format!(
            r#"{{"id":"{id}","name":"N","description":"D","version":"{version}","kind":"ui",
                "min_app_version":"0.0.60","sdk_version":1,"permissions":["std.storage"]{deps_field}}}"#
        );
        PluginManifest::parse(json.as_bytes()).unwrap()
    }

    fn info(manifest: PluginManifest, installed: bool, enabled: bool) -> Option<DepInfo> {
        Some(DepInfo {
            manifest,
            installed,
            enabled,
        })
    }

    #[test]
    fn no_deps_passes() {
        let target = manifest("com.a.main", "1.0.0", &[]);
        let result = check_enable(&target, &|_| None, "0.0.60").unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn missing_dependency_rejected() {
        let target = manifest("com.a.main", "1.0.0", &[("com.a.lib", "1.0.0")]);
        let err = check_enable(&target, &|_| None, "0.0.60").unwrap_err();
        assert!(err.user_message().contains("缺少依赖"), "{err}");
    }

    #[test]
    fn version_too_low_rejected() {
        let target = manifest("com.a.main", "1.0.0", &[("com.a.lib", "2.0.0")]);
        let lib = manifest("com.a.lib", "1.0.0", &[]);
        let err = check_enable(
            &target,
            &|id| {
                (id == "com.a.lib")
                    .then(|| info(lib.clone(), true, true))
                    .flatten()
            },
            "0.0.60",
        )
        .unwrap_err();
        assert!(err.user_message().contains("版本过低"), "{err}");
    }

    #[test]
    fn disabled_dependency_rejected() {
        let target = manifest("com.a.main", "1.0.0", &[("com.a.lib", "1.0.0")]);
        let lib = manifest("com.a.lib", "1.0.0", &[]);
        let err = check_enable(
            &target,
            &|id| {
                (id == "com.a.lib")
                    .then(|| info(lib.clone(), true, false))
                    .flatten()
            },
            "0.0.60",
        )
        .unwrap_err();
        assert!(err.user_message().contains("已停用"), "{err}");
    }

    #[test]
    fn incompatible_dependency_rejected() {
        let target = manifest("com.a.main", "1.0.0", &[("com.a.lib", "1.0.0")]);
        let lib = manifest("com.a.lib", "1.0.0", &[]);
        let lib = PluginManifest {
            min_app_version: "99.0.0".into(),
            ..lib
        };
        let err = check_enable(
            &target,
            &|id| {
                (id == "com.a.lib")
                    .then(|| info(lib.clone(), true, true))
                    .flatten()
            },
            "0.0.60",
        )
        .unwrap_err();
        assert!(err.user_message().contains("不兼容"), "{err}");
    }

    #[test]
    fn cycle_detected() {
        // A 依赖 B，B 依赖 A（A 自身已安装——目标启用场景 lookup 也会返回它）
        let target = manifest("com.a.a", "1.0.0", &[("com.a.b", "1.0.0")]);
        let b = manifest("com.a.b", "1.0.0", &[("com.a.a", "1.0.0")]);
        let err = check_enable(
            &target,
            &|id| match id {
                "com.a.a" => info(target.clone(), true, true),
                "com.a.b" => info(b.clone(), true, true),
                _ => None,
            },
            "0.0.60",
        )
        .unwrap_err();
        assert!(err.user_message().contains("循环依赖"), "{err}");
    }

    #[test]
    fn diamond_dependency_topological_order() {
        // main → b → c；main → c（钻石）：c 只验证一次
        let target = manifest(
            "com.a.main",
            "1.0.0",
            &[("com.a.b", "1.0.0"), ("com.a.c", "1.0.0")],
        );
        let b = manifest("com.a.b", "1.0.0", &[("com.a.c", "1.0.0")]);
        let c = manifest("com.a.c", "1.0.0", &[]);
        let order = check_enable(
            &target,
            &|id| match id {
                "com.a.b" => info(b.clone(), true, true),
                "com.a.c" => info(c.clone(), true, true),
                _ => None,
            },
            "0.0.60",
        )
        .unwrap();
        assert_eq!(order, vec!["com.a.c".to_string(), "com.a.b".to_string()]);
    }
}
