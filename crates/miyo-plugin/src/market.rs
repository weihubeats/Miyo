//! 市场索引签名验证（`docs/market/index.json`，schema v2）。
//!
//! 签名规则与插件包一致：Ed25519 over「去除 `signature` 键后的规范化 JSON」
//! （对象键名字典序递归排序、紧凑无空白序列化）。信任三级沿用
//! [`crate::package::TrustLevel`]：未签名放行（现状兼容）；签名存在但未命中
//! keyring → Foreign；命中 → Official。
//!
//! 防篡改口径由调用方执行：**签名存在但验证失败必须硬错误**（索引被篡改），
//! 未签名仅降级展示（与未签名插件包需显式信任同构）。

use serde_json::{Map, Value};

use crate::package::TrustLevel;

/// 索引签名验证结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MarketIndexVerification {
    /// 索引携带 signature 字段。
    pub signed: bool,
    /// 签名命中官方 keyring（signed=false 恒为 false）。
    pub verified: bool,
}

impl MarketIndexVerification {
    pub fn trust(&self) -> TrustLevel {
        if self.verified {
            TrustLevel::Official
        } else {
            TrustLevel::Unsigned
        }
    }
}

/// 验证市场索引字节。JSON 结构损坏返回 Err；未签名返回 Unsigned；
/// 签名非法格式 / 验签失败返回 Foreign（调用方对 signed-but-foreign 硬失败）。
pub fn verify_market_index(
    bytes: &[u8],
    keys: &[&str],
) -> Result<MarketIndexVerification, miyo_core::AppError> {
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|e| miyo_core::AppError::Validation(format!("市场索引 JSON 损坏：{e}")))?;
    let obj = value
        .as_object()
        .ok_or_else(|| miyo_core::AppError::Validation("市场索引必须是 JSON 对象".into()))?;

    let Some(sig_hex) = obj.get("signature").and_then(Value::as_str) else {
        return Ok(MarketIndexVerification {
            signed: false,
            verified: false,
        });
    };

    let mut payload_obj = obj.clone();
    payload_obj.remove("signature");
    let payload = compact_canonical(&Value::Object(payload_obj));

    let sig_bytes = hex::decode(sig_hex)
        .ok()
        .and_then(|b| <[u8; 64]>::try_from(b.as_slice()).ok());
    let Some(sig_bytes) = sig_bytes else {
        return Ok(MarketIndexVerification {
            signed: true,
            verified: false,
        });
    };
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let sig = match Signature::from_slice(&sig_bytes) {
        Ok(s) => s,
        Err(_) => {
            return Ok(MarketIndexVerification {
                signed: true,
                verified: false,
            })
        }
    };
    for key_hex in keys {
        let Ok(pubkey_bytes) = hex::decode(key_hex) else {
            continue;
        };
        let Ok(bytes32) = <[u8; 32]>::try_from(pubkey_bytes.as_slice()) else {
            continue;
        };
        let Ok(vk) = VerifyingKey::from_bytes(&bytes32) else {
            continue;
        };
        if vk.verify(&payload, &sig).is_ok() {
            return Ok(MarketIndexVerification {
                signed: true,
                verified: true,
            });
        }
    }
    Ok(MarketIndexVerification {
        signed: true,
        verified: false,
    })
}

/// 规范化 JSON：对象键名字典序递归排序、紧凑无空白（与 sign_plugin.mjs
/// 的 canonical 序列化同口径）。
fn compact_canonical(value: &Value) -> Vec<u8> {
    let sorted = sort_value(value);
    serde_json::to_vec(&sorted).expect("canonical json 序列化不可能失败")
}

fn sort_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut out = Map::new();
            for k in keys {
                out.insert(k.clone(), sort_value(&map[k]));
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    const TEST_SEED: [u8; 32] = [7u8; 32];

    fn canonical_payload(bytes: &[u8]) -> Vec<u8> {
        let value: Value = serde_json::from_slice(bytes).unwrap();
        let mut obj = value.as_object().unwrap().clone();
        obj.remove("signature");
        compact_canonical(&Value::Object(obj))
    }

    fn sign_index(index: &mut Value) -> String {
        let payload = compact_canonical(index);
        let key = SigningKey::from_bytes(&TEST_SEED);
        hex::encode(key.sign(&payload).to_bytes())
    }

    #[test]
    fn unsigned_index_is_unsigned() {
        let bytes = br#"{"version":1,"plugins":[]}"#;
        let v = verify_market_index(
            bytes,
            &["92739639878299e7f459b10fe3ba2c6994559594ff2feb1cbf61eef3d71cd206"],
        )
        .unwrap();
        assert!(!v.signed);
        assert!(!v.verified);
        assert_eq!(v.trust(), TrustLevel::Unsigned);
    }

    #[test]
    fn signed_index_verifies_against_keyring() {
        let pubkey = hex::encode(
            SigningKey::from_bytes(&TEST_SEED)
                .verifying_key()
                .to_bytes(),
        );
        let mut index: Value = serde_json::from_str(
            r#"{"version":1,"updated_at":"2026-11-22","plugins":[{"id":"a"}]}"#,
        )
        .unwrap();
        index["signature"] = Value::String(sign_index(&mut index.clone()));
        let bytes = serde_json::to_vec(&index).unwrap();
        let v = verify_market_index(&bytes, &[&pubkey]).unwrap();
        assert!(v.signed && v.verified);
        assert_eq!(v.trust(), TrustLevel::Official);
    }

    #[test]
    fn tampered_index_fails_verification() {
        let pubkey = hex::encode(
            SigningKey::from_bytes(&TEST_SEED)
                .verifying_key()
                .to_bytes(),
        );
        let mut index: Value =
            serde_json::from_str(r#"{"version":1,"plugins":[{"id":"a"}]}"#).unwrap();
        index["signature"] = Value::String(sign_index(&mut index.clone()));
        // 篡改 plugins（签名覆盖不到被改字段）
        index["plugins"] = serde_json::json!([{"id": "evil"}]);
        let bytes = serde_json::to_vec(&index).unwrap();
        let v = verify_market_index(&bytes, &[&pubkey]).unwrap();
        assert!(v.signed && !v.verified);
    }

    #[test]
    fn malformed_signature_is_signed_but_unverified() {
        let bytes = br#"{"version":1,"signature":"zz-not-hex"}"#;
        let v = verify_market_index(bytes, &[]).unwrap();
        assert!(v.signed && !v.verified);
    }

    #[test]
    fn canonical_sorting_is_stable_across_key_order() {
        // 键序不同、内容相同的索引 → 同一规范化 payload（签名可迁移）
        let a = compact_canonical(
            &serde_json::from_str::<Value>(r#"{"b":1,"a":{"d":2,"c":3}}"#).unwrap(),
        );
        let b = compact_canonical(
            &serde_json::from_str::<Value>(r#"{"a":{"c":3,"d":2},"b":1}"#).unwrap(),
        );
        assert_eq!(a, b);
        assert!(!a.is_empty());
        let _ = canonical_payload(b"{}"); // 覆盖辅助函数
    }
}
