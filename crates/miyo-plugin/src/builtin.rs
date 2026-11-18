//! 内置插件目录：随应用打包、但走完整插件生命周期（未安装 = 不注册 UI、命令门禁拒绝）。
//!
//! 清单文件是唯一事实源（`builtins/<id>/manifest.json`，include_str! 编译期嵌入）：
//! - 前端市场页经 IPC `plugin_list_catalog` 拉取，不手写第二份；
//! - builtin loader（前端 `plugins/builtinModules.ts`）只按 ID 映射到仓内模块。
//!
//! 内置 ≠ 默认安装：装机后插件处于「市场未安装」态，用户显式安装并批准权限后才激活。

use std::sync::OnceLock;

use miyo_core::AppError;

use crate::manifest::PluginManifest;

/// Java 方法调试插件 ID（后端命令门禁 / 前端 loader 共用）。
pub const JAVA_DEBUG_PLUGIN_ID: &str = "com.miyo.java-debug";

/// 内置插件清单原文（编译期嵌入，运行时零 IO）。
const JAVA_DEBUG_MANIFEST: &str = include_str!("../builtins/java-debug/manifest.json");

/// 内置清单缓存（启动后进程内只解析一次）。
static BUILTINS: OnceLock<Vec<PluginManifest>> = OnceLock::new();

/// 全部内置插件清单（解析失败属于编译期缺陷，panic 快速暴露）。
pub fn builtin_manifests() -> &'static [PluginManifest] {
    BUILTINS.get_or_init(|| {
        vec![PluginManifest::parse(JAVA_DEBUG_MANIFEST.as_bytes())
            .expect("内置插件清单必须合法（编译期数据）")]
    })
}

/// 按 ID 取内置插件清单。
pub fn builtin_manifest(id: &str) -> Option<&'static PluginManifest> {
    builtin_manifests().iter().find(|m| m.id == id)
}

/// 内置插件 ID 与清单的强一致校验：清单里的 id 必须与常量一致。
pub fn ensure_builtin_consistency() -> Result<(), AppError> {
    let manifest = builtin_manifest(JAVA_DEBUG_PLUGIN_ID)
        .ok_or_else(|| AppError::Validation("内置 Java 插件清单缺失".into()))?;
    if manifest.id != JAVA_DEBUG_PLUGIN_ID {
        return Err(AppError::Validation(
            "内置 Java 插件清单 id 与常量不一致".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permission;

    #[test]
    fn java_builtin_manifest_is_valid_and_consistent() {
        ensure_builtin_consistency().unwrap();
        let m = builtin_manifest(JAVA_DEBUG_PLUGIN_ID).unwrap();
        assert_eq!(m.version, "0.1.0");
        assert_eq!(m.kind, crate::manifest::PluginKind::Ui);
        // 声明的权限必须全部命中目录；v2 权限细分：java.debug 聚合键拆为
        // 四个 java.* 子权限（清单不再声明聚合键）。
        permission::validate_permissions(&m.permissions).unwrap();
        assert!(!permission::granted(&m.permissions, "java.debug"));
        for sub in permission::JAVA_SUB_PERMISSIONS {
            assert!(
                permission::granted(&m.permissions, sub),
                "内置清单缺少子权限 {sub}"
            );
        }
        // 导航路由与前端历史路由保持一致（迁移期不换路径）。
        let nav = m.nav.as_ref().unwrap();
        assert_eq!(nav[0].route, "/java-debug");
    }

    #[test]
    fn builtin_list_matches_constants() {
        let all = builtin_manifests();
        assert!(all.iter().any(|m| m.id == JAVA_DEBUG_PLUGIN_ID));
    }
}
