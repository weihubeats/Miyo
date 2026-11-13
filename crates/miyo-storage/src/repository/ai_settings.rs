//! AI 提供方设置（应用内 AI 测试中心的模型接入配置）。
//!
//! 整体 JSON 存于 settings 表（`ai_provider` 键），其中 `api_key` 字段
//! 经 AES-256-GCM 加密（与环境变量 / 全局变量同通道）——密文整体落库，
//! 读取时解密回内存结构。未配置返回 `None`。

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use miyo_core::Result;

use super::rows::{decrypt_env_json, encrypt_blob};

/// settings 表存储键。
const KEY_AI_PROVIDER: &str = "ai_provider";

/// AI 提供方配置（与 miyo-ai 的 AiProviderConfig 字段一致；存储层不依赖
/// miyo-ai，独立声明同形结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiProviderSettings {
    pub base_url: String,
    /// API Key（落库前整体加密；本结构体内存中为明文）。
    #[serde(default)]
    pub api_key: String,
    pub model: String,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
}

/// 读取 AI 提供方配置；未配置返回 None，解密失败按未配置处理（由调用方
/// 引导用户重新保存，避免密钥丢失导致整个设置页打不开）。
pub async fn get_ai_settings(db: &SqlitePool) -> Result<Option<AiProviderSettings>> {
    let Some(raw) = super::get_setting(db, KEY_AI_PROVIDER).await? else {
        return Ok(None);
    };
    let Ok(value) = decrypt_env_json(&raw) else {
        tracing::warn!("AI 提供方配置解密失败，按未配置处理（请重新保存）");
        return Ok(None);
    };
    let cfg: AiProviderSettings = serde_json::from_value(value)
        .map_err(|e| miyo_core::AppError::Decryption(e.to_string()))?;
    Ok(Some(cfg))
}

/// 保存 AI 提供方配置（api_key 加密；整体覆盖写）。
pub async fn save_ai_settings(db: &SqlitePool, cfg: &AiProviderSettings) -> Result<()> {
    let value = serde_json::to_value(cfg)
        .map_err(|e| miyo_core::AppError::Validation(format!("AI 配置序列化失败：{e}")))?;
    let blob = encrypt_blob(&value.to_string())?;
    super::set_setting(db, KEY_AI_PROVIDER, &blob).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::memory_pool;

    #[tokio::test]
    async fn ai_settings_roundtrip_encrypted() {
        let db = memory_pool().await.unwrap();
        assert!(get_ai_settings(&db).await.unwrap().is_none());

        let cfg = AiProviderSettings {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-secret-123".into(),
            model: "deepseek-chat".into(),
            temperature: Some(0.3),
            max_tokens: Some(4096),
        };
        save_ai_settings(&db, &cfg).await.unwrap();

        // 密文落库：明文 key 不出现在 settings 表。
        let raw: (String,) =
            sqlx::query_as("SELECT value_json FROM settings WHERE key = 'ai_provider'")
                .fetch_one(&db)
                .await
                .unwrap();
        assert!(!raw.0.contains("sk-secret-123"), "API Key 应加密存储");

        let loaded = get_ai_settings(&db).await.unwrap().unwrap();
        assert_eq!(loaded.base_url, cfg.base_url);
        assert_eq!(loaded.api_key, "sk-secret-123");
        assert_eq!(loaded.model, "deepseek-chat");
    }
}
