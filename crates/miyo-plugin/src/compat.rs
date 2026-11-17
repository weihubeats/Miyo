//! 版本兼容性检查：插件 `min_app_version` ↔ 当前 Miyo 版本的逐段比较。
//!
//! 版本号口径与 manifest 校验一致（严格 x.y.z），不引入 semver crate——
//! 插件体系只需要「宿主不低于最低要求」一个方向。

use miyo_core::AppError;

use crate::manifest::parse_version;

/// 检查当前宿主版本是否满足插件的最低要求（major/minor/patch 逐段比较）。
pub fn is_compatible(min_app_version: &str, current_app_version: &str) -> bool {
    match (
        parse_version(min_app_version),
        parse_version(current_app_version),
    ) {
        (Ok(min), Ok(cur)) => cur >= min,
        // 版本非法视作不兼容（校验层本应拦住，这里保守处理）。
        _ => false,
    }
}

/// 带错误信息的兼容性校验（启用 / 激活 / 更新前调用）。
pub fn ensure_compatible(
    plugin_id: &str,
    min_app_version: &str,
    current_app_version: &str,
) -> Result<(), AppError> {
    if is_compatible(min_app_version, current_app_version) {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "插件 {plugin_id} 需要 Miyo ≥ {min_app_version}，当前版本 {current_app_version} 不兼容"
    )))
}

/// SDK 协议版本检查（manifest 校验已挡住非法值，这里挡「未来协议」）。
pub fn ensure_sdk_supported(sdk_version: u32, supported: u32) -> Result<(), AppError> {
    if sdk_version == supported {
        return Ok(());
    }
    Err(AppError::Validation(format!(
        "插件 SDK 协议版本 {sdk_version} 与宿主支持的 {supported} 不一致，请升级 Miyo 或插件"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segmentwise_comparison() {
        assert!(is_compatible("0.0.60", "0.0.60"));
        assert!(is_compatible("0.0.60", "0.0.61"));
        assert!(is_compatible("0.0.60", "1.0.0"));
        assert!(!is_compatible("0.1.0", "0.0.60"));
        assert!(!is_compatible("1.0.0", "0.99.999"));
        // patch 位逐段比较，不做字符串序（避免 0.0.9 < 0.0.10 误判）。
        assert!(is_compatible("0.0.9", "0.0.10"));
        assert!(!is_compatible("0.0.10", "0.0.9"));
    }

    #[test]
    fn invalid_versions_are_incompatible() {
        assert!(!is_compatible("latest", "0.0.60"));
        assert!(!is_compatible("0.0.60", ""));
    }

    #[test]
    fn ensure_reports_plugin_id() {
        let err = ensure_compatible("com.example.old", "9.9.9", "0.0.60").unwrap_err();
        assert!(err.user_message().contains("com.example.old"));
        assert!(err.user_message().contains("9.9.9"));
        assert!(ensure_sdk_supported(1, 1).is_ok());
        assert!(ensure_sdk_supported(2, 1).is_err());
    }
}
