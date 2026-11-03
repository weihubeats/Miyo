//! 全局变量（Global Variables）。
//!
//! 跨项目 / 跨环境共享的变量，优先级最低（运行时 > 环境 > 项目 > 全局），
//! 供 `{{name}}` 注入兜底。与环境变量一致支持「远程值 / 本地值 / 启用」，
//! 整体加密后存于 settings 表（`global_variables` 键），复用一个 JSON blob。

use sqlx::SqlitePool;

use miyo_core::model::EnvironmentVariable;
use miyo_core::Result;

use super::rows::{decrypt_env_json, encrypt_env_json, variables_from_value};

/// settings 表存储键。
const KEY_GLOBAL_VARIABLES: &str = "global_variables";

/// 读取全局变量；未配置 / 解密失败返回空表。
pub async fn get_global_variables(db: &SqlitePool) -> Result<Vec<EnvironmentVariable>> {
    let Some(blob) = super::get_setting(db, KEY_GLOBAL_VARIABLES).await? else {
        return Ok(Vec::new());
    };
    match decrypt_env_json(&blob) {
        Ok(value) => {
            let vars = variables_from_value(value)?;
            Ok(vars)
        }
        Err(e) => {
            tracing::warn!(error = %e, "全局变量解密失败，按空表返回");
            Ok(Vec::new())
        }
    }
}

/// 保存全局变量（整体加密覆盖写）。
pub async fn save_global_variables(
    db: &SqlitePool,
    variables: &[EnvironmentVariable],
) -> Result<()> {
    let blob = encrypt_env_json(variables)?;
    super::set_setting(db, KEY_GLOBAL_VARIABLES, &blob).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;

    #[tokio::test]
    async fn global_variables_roundtrip_encrypted() {
        let db = memory_pool().await.unwrap();
        assert!(get_global_variables(&db).await.unwrap().is_empty());

        let vars = vec![
            EnvironmentVariable {
                key: "domain".into(),
                remote_value: "example.com".into(),
                local_value: String::new(),
                enabled: true,
                description: Some("全局域名".into()),
            },
            EnvironmentVariable {
                key: "disabled_key".into(),
                remote_value: "x".into(),
                local_value: String::new(),
                enabled: false,
                description: None,
            },
        ];
        save_global_variables(&db, &vars).await.unwrap();

        let raw: (String,) =
            sqlx::query_as("SELECT value_json FROM settings WHERE key = 'global_variables'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(!raw.0.contains("example.com"), "全局变量应加密存储");

        let loaded = get_global_variables(&db).await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].key, "domain");
        assert_eq!(loaded[0].effective_value(), "example.com");
        assert!(!loaded[1].enabled);
    }
}
