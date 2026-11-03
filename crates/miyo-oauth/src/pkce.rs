//! PKCE（RFC 7636）S256 支持：授权码流的防截获扩展。
//!
//! 流程：本地生成随机 `code_verifier`（43~128 个非保留字符），授权 URL 携带
//! `code_challenge = BASE64URL(SHA256(verifier))` + `code_challenge_method=S256`；
//! 换 token 时回传 `code_verifier`，授权服务器校验哈希一致后才发令牌。
//! 授权码即使被回调链路截获，没有 verifier 也换不出 token。

use base64::Engine;
use rand::Rng as _;
use sha2::{Digest, Sha256};

/// 生成 `code_verifier`：32 随机字节 base64url 无填充 = 86 字符
/// （RFC 7636 要求 43~128，字符集 [A-Za-z0-9-._~]，base64url 字符集是其子集）。
pub fn code_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// `code_challenge = BASE64URL-ENCODE(SHA256(ASCII(code_verifier)))`（无填充）。
pub fn code_challenge_s256(verifier: &str) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 7636 附录 B 官方测试向量。
    #[test]
    fn rfc7636_appendix_b_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            code_challenge_s256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_length_and_charset() {
        for _ in 0..32 {
            let v = code_verifier();
            assert!((43..=128).contains(&v.len()), "长度越界: {}", v.len());
            assert!(
                v.chars().all(|c| c.is_ascii_alphanumeric()
                    || c == '-'
                    || c == '_'
                    || c == '.'
                    || c == '~'),
                "非法字符: {v}"
            );
        }
    }

    #[test]
    fn verifiers_are_unique() {
        let a = code_verifier();
        let b = code_verifier();
        assert_ne!(a, b, "两次生成不应相同（32 字节随机）");
    }
}
