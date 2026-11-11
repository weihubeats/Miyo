//! 文本工具：UTF-8 安全截断（原 miyo-tauri request.rs / grpc.rs /
//! miyo-agent server.rs 三份同逻辑实现收敛于此）。

/// 按字节上限截断到最近的字符边界（多字节字符不截半）。
/// 不超限时原样返回借用；超限时返回 `s[..end]`（end 回退到字符边界）。
pub fn byte_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn within_limit_returns_borrow_of_original() {
        let s = String::from("hello");
        let t = byte_truncate(&s, 10);
        assert!(std::ptr::eq(t, s.as_str()));
    }

    #[test]
    fn ascii_exact_boundary() {
        assert_eq!(byte_truncate("abcdef", 3), "abc");
    }

    #[test]
    fn multibyte_truncates_at_char_boundary() {
        // 中文字符 3 字节：上限 4 → 只能保留 1 个字符（3 字节）。
        assert_eq!(byte_truncate("中文测试", 4), "中");
        // 上限 3 恰好整字符。
        assert_eq!(byte_truncate("中文测试", 3), "中");
        // 上限 2：任何中文字符都放不下 → 空串。
        assert_eq!(byte_truncate("中文", 2), "");
    }

    #[test]
    fn mixed_content_boundary() {
        let s = "ab中文cd";
        // 6 字节 = ab + 中；5 字节恰好在「文」起始边界前，仍能保住「中」。
        assert_eq!(byte_truncate(s, 6), "ab中");
        assert_eq!(byte_truncate(s, 5), "ab中");
    }

    #[test]
    fn empty_and_zero() {
        assert_eq!(byte_truncate("", 10), "");
        assert_eq!(byte_truncate("abc", 0), "");
    }
}
