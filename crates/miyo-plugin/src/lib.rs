//! # miyo-plugin：Miyo 插件系统领域层（Core + Plugin 架构）
//!
//! 职责边界（本 crate 是**纯逻辑层**，不做任何 IPC / 数据库访问）：
//! - [`manifest`]：清单模型 + 严格校验（真源；前端展示层为 ts-rs 生成镜像）；
//! - [`compat`]：宿主版本 / SDK 协议兼容性检查；
//! - [`permission`]：权限目录（封闭集合）与校验；
//! - [`builtin`]：内置插件目录（编译期嵌入清单）；
//! - [`package`]：`.miyo` 插件包解包 / 完整性 / ed25519 签名（穿越防护）。
//!
//! 持久化（plugins / plugin_data / plugin_logs 表）在 `miyo-storage::repository::plugins`，
//! IPC 命令在 `miyo-tauri::commands::plugins`——本 crate 不依赖任何 IO 之外的设施。
//!
//! ## 架构总览
//!
//! ```text
//! 前端 PluginManager（WebView）
//!   │  IPC plugin_*（安装/启停/日志/存储）
//!   ▼
//! miyo-tauri commands/plugins.rs ──► miyo-plugin（本 crate：校验/包/权限）
//!                              └──► miyo-storage repository/plugins（表读写）
//! 内置插件（Java 调试）：miyo-plugin::builtin 目录 + 前端 builtinModules 懒加载模块；
//! 命令门禁：java_* 等 14 命令在执行前查询插件启用态（repository::is_plugin_enabled）。
//! ```

pub mod builtin;
pub mod compat;
pub mod deps;
pub mod manifest;
pub mod package;
pub mod permission;

pub use builtin::{builtin_manifest, builtin_manifests, JAVA_DEBUG_PLUGIN_ID};
pub mod market;
pub use compat::{ensure_compatible, ensure_sdk_supported, is_compatible};
pub use manifest::{
    PluginConfigField, PluginConfigFieldType, PluginConfigSchema, PluginKind, PluginManifest,
    PluginNavItem, PluginRuntimeDep, SUPPORTED_SDK_VERSION,
};
pub use package::{PackageInfo, TrustLevel, OFFICIAL_SIGNING_PUBKEYS};
pub use permission::{
    lookup as lookup_permission, validate_permissions, PermissionDef, PermissionRisk,
};

#[cfg(test)]
mod tests {
    use super::*;

    /// 内置目录在默认测试配置下必须自洽（清单合法 + id 一致 + 权限命中目录）。
    #[test]
    fn builtin_catalog_self_consistent() {
        builtin::ensure_builtin_consistency().unwrap();
    }
}
