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
        id: "database.connect",
        risk: PermissionRisk::High,
        summary: "建立数据库连接（能力尚未开放；声明恒拒，登记以显式化风险）",
    },
    PermissionDef {
        id: "database.execute",
        risk: PermissionRisk::High,
        summary: "执行数据库语句（能力尚未开放；声明恒拒，登记以显式化风险）",
    },
    PermissionDef {
        id: "java.class.inspect",
        risk: PermissionRisk::High,
        summary: "探测目标 JVM 已加载类 / 方法 / Spring Bean（元信息读取，高危）",
    },
    PermissionDef {
        id: "java.debug",
        risk: PermissionRisk::High,
        summary: "Java 调试聚合权限（等价 java.* 四个子权限；v2.0 拆分前的兼容项）",
    },
    PermissionDef {
        id: "java.method.invoke",
        risk: PermissionRisk::High,
        summary: "在目标 JVM 内执行方法（最高危：可触发任意业务逻辑）",
    },
    PermissionDef {
        id: "java.process.discover",
        risk: PermissionRisk::Medium,
        summary: "扫描本机 Java 进程（launcher sidecar 枚举 + OS 扫描）",
    },
    PermissionDef {
        id: "java.runtime.attach",
        risk: PermissionRisk::High,
        summary: "向目标 JVM 注入调试 Agent / 断开会话（动态注入，高危）",
    },
    PermissionDef {
        id: "runtime.process.start",
        risk: PermissionRisk::High,
        summary: "启动插件 Sidecar 进程（宿主托管的受控子进程）",
    },
    PermissionDef {
        id: "runtime.process.stop",
        risk: PermissionRisk::Medium,
        summary: "停止插件自己的 Sidecar 进程",
    },
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
        id: "std.files.read",
        risk: PermissionRisk::Medium,
        summary: "读取用户显式选择的文件（能力尚未开放；声明恒拒，登记以显式化风险）",
    },
    PermissionDef {
        id: "std.files.write",
        risk: PermissionRisk::Medium,
        summary: "写入用户指定位置（能力尚未开放；声明恒拒，登记以显式化风险）",
    },
    PermissionDef {
        id: "std.log",
        risk: PermissionRisk::Low,
        summary: "写入插件日志（受脱敏约束，敏感信息不得入日志）",
    },
    PermissionDef {
        id: "std.network",
        risk: PermissionRisk::Medium,
        summary: "经宿主代理发起网络请求（能力尚未开放；声明恒拒，登记以显式化风险）",
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
];

/// java 聚合权限（`java.debug`）展开的子权限集合：
/// 命令门禁按子权限检查；granted 含 `java.debug`（含 0033 之前的存量行）
/// 等价于持有全部子权限。
pub const JAVA_SUB_PERMISSIONS: &[&str] = &[
    "java.class.inspect",
    "java.method.invoke",
    "java.process.discover",
    "java.runtime.attach",
];

/// 展开聚合权限：granted 集合中把 `java.debug` 视为全部 java 子权限。
/// 返回去重后的有效权限集合。
pub fn expand_granted(granted: &[String]) -> Vec<&str> {
    let mut out: Vec<&str> = Vec::new();
    let has_umbrella = granted.iter().any(|g| g == "java.debug");
    for g in granted {
        if g == "java.debug" {
            continue;
        }
        out.push(g.as_str());
    }
    if has_umbrella {
        for sub in JAVA_SUB_PERMISSIONS {
            if !out.contains(sub) {
                out.push(sub);
            }
        }
    }
    out
}

/// 检查展开后的 granted 是否持有某权限（含聚合展开）。
pub fn granted_effective(granted: &[String], id: &str) -> bool {
    expand_granted(granted).contains(&id)
}

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

    #[test]
    fn java_umbrella_expands_to_sub_permissions() {
        // 存量行（0033 前）：仅 java.debug → 展开为全部子权限
        let g = vec!["java.debug".to_string()];
        for sub in JAVA_SUB_PERMISSIONS {
            assert!(granted_effective(&g, sub), "{sub}");
        }
        // 0033 后的新行：显式子权限，无聚合键
        let g = vec![
            "java.process.discover".to_string(),
            "java.method.invoke".to_string(),
        ];
        assert!(granted_effective(&g, "java.process.discover"));
        assert!(granted_effective(&g, "java.method.invoke"));
        assert!(!granted_effective(&g, "java.runtime.attach"));
        assert!(!granted_effective(&g, "java.class.inspect"));
    }

    #[test]
    fn capability_catalog_is_closed_and_risk_annotated() {
        // 高危权限显式存在；无能力权限（database.* / std.network / std.files.*）
        // 登记为恒拒——文档口径一致
        for id in [
            "java.method.invoke",
            "database.execute",
            "std.network",
            "std.files.read",
        ] {
            assert!(lookup(id).is_some(), "{id} 应在目录中");
        }
    }
}
