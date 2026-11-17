//! 插件权限目录：唯一事实源（前端 `plugins/permissions.ts` 与之镜像，仅做展示文案）。
//!
//! 原则：
//! - 权限 ID 是封闭集合——manifest 声明未知名即校验失败（防声明绕过）；
//! - 后端在命令映射处强制检查（如 `java.*` 命令 ↔ `java.debug` 权限 + 插件启用态）；
//! - 新增能力必须先在这里登记，再写命令映射，最后补前端文案键。

use miyo_core::AppError;

/// 风险等级（管理中心按级着色；High 需在启用确认里单独勾选）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionRisk {
    Low,
    Medium,
    High,
}

/// 权限定义。
#[derive(Debug, Clone, Copy)]
pub struct PermissionDef {
    /// 权限 ID（manifest.permissions 中的字面量）。
    pub id: &'static str,
    /// 风险等级。
    pub risk: PermissionRisk,
    /// 中文摘要（错误信息 / 日志用；UI 文案走前端 i18n 键）。
    pub summary: &'static str,
}

/// 权限目录（封闭集合，按 ID 排序便于二分）。
pub const PERMISSIONS: &[PermissionDef] = &[
    PermissionDef {
        id: "std.clipboard.write",
        risk: PermissionRisk::Low,
        summary: "写入系统剪贴板",
    },
    PermissionDef {
        id: "std.env.read",
        risk: PermissionRisk::Medium,
        summary: "读取环境变量合并表（可能包含敏感变量值）",
    },
    PermissionDef {
        id: "std.events",
        risk: PermissionRisk::Low,
        summary: "订阅插件事件总线",
    },
    PermissionDef {
        id: "std.log",
        risk: PermissionRisk::Low,
        summary: "写入插件日志（受脱敏约束，敏感信息不得入日志）",
    },
    PermissionDef {
        id: "std.notify",
        risk: PermissionRisk::Low,
        summary: "发送应用内通知（Toast）",
    },
    PermissionDef {
        id: "std.storage",
        risk: PermissionRisk::Low,
        summary: "读写插件私有 KV 存储（按插件隔离）",
    },
    PermissionDef {
        id: "java.debug",
        risk: PermissionRisk::High,
        summary: "连接并调用 Java Debug Runtime（方法发现与执行，高危）",
    },
];

/// 按 ID 查权限定义。
pub fn lookup(id: &str) -> Option<&'static PermissionDef> {
    PERMISSIONS.iter().find(|p| p.id == id)
}

/// 校验一批权限声明：未知名 / 重复 / 超量即拒绝。
pub fn validate_permissions(claimed: &[String]) -> Result<(), AppError> {
    if claimed.len() > 16 {
        return Err(AppError::Validation("权限声明数量超过 16".into()));
    }
    let mut seen = std::collections::BTreeSet::new();
    for id in claimed {
        if lookup(id).is_none() {
            let known: Vec<&str> = PERMISSIONS.iter().map(|p| p.id).collect();
            return Err(AppError::Validation(format!(
                "未知权限「{id}」（允许的权限：{}）",
                known.join(", ")
            )));
        }
        if !seen.insert(id.as_str()) {
            return Err(AppError::Validation(format!("权限重复声明：{id}")));
        }
    }
    Ok(())
}

/// 检查插件是否持有某权限（granted ⊆ manifest.permissions 在启用流程保证；
/// 这里只做已批准集合的成员检查）。
pub fn granted(granted: &[String], id: &str) -> bool {
    granted.iter().any(|g| g == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_ids_are_unique_and_valid_format() {
        let mut seen = std::collections::BTreeSet::new();
        for p in PERMISSIONS {
            assert!(
                crate::manifest::PluginManifest::validate_id(p.id).is_ok(),
                "{}",
                p.id
            );
            assert!(seen.insert(p.id), "权限重复：{}", p.id);
        }
    }

    #[test]
    fn validate_rejects_unknown_and_duplicates() {
        assert!(validate_permissions(&["std.storage".into()]).is_ok());
        assert!(validate_permissions(&["std.storage".into(), "java.debug".into()]).is_ok());
        assert!(validate_permissions(&["std.unknown".into()]).is_err());
        assert!(validate_permissions(&["std.storage".into(), "std.storage".into()]).is_err());
    }

    #[test]
    fn granted_checks_membership() {
        let g = vec!["std.storage".to_string()];
        assert!(granted(&g, "std.storage"));
        assert!(!granted(&g, "java.debug"));
    }
}
