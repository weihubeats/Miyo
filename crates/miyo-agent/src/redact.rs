//! Agent 层脱敏助手（单源 [`miyo_sync::redact`] 的薄封装）。
//!
//! 供 server handlers 与 `analysis` 模块（失败分析 / 修复草稿）共用：
//! 凡是返回给 AI / 前端 / 落历史的数据，出仓前统一过这里。

/// 变量池脱敏（单源 [`miyo_sync::redact::redact_vars`]）。
pub(crate) fn redact_vars(
    vars: &std::collections::HashMap<String, String>,
) -> std::collections::HashMap<String, String> {
    miyo_sync::redact::redact_vars(vars)
}

/// URL query 脱敏（单源 [`miyo_sync::redact::redact_url_query`]）：日志 /
/// 历史落库 / MCP 返回共用；凭据在 query 的 URL 不落明文。
pub(crate) fn redact_url(url: &str) -> String {
    miyo_sync::redact::redact_url_query(url)
}

/// 请求快照脱敏（headers / cookies / url query）。
pub(crate) fn redact_snapshot(snapshot: &mut miyo_http::client::RequestSnapshot) {
    let mask = |(k, v): &(String, String)| -> (String, String) {
        if miyo_sync::redact::is_sensitive_key(k) && !v.is_empty() {
            (k.clone(), miyo_sync::redact::PLACEHOLDER.to_string())
        } else {
            (k.clone(), v.clone())
        }
    };
    snapshot.headers = snapshot.headers.iter().map(&mask).collect();
    snapshot.cookies = snapshot.cookies.iter().map(&mask).collect();
    snapshot.url = redact_url(&snapshot.url);
}

/// 运行结果全面脱敏（MCP/Agent 返回与历史落库共用单点）：
/// URL query、提取值、响应头、响应 body（JSON 保守策略）、请求快照、
/// 断言 expected/actual 逐一处理。旧历史行（脱敏前落库的明文数据）在
/// 读取端重过一遍本函数即可兜底。
pub(crate) fn redact_endpoint_result(r: &mut miyo_test::runner::EndpointResult) {
    if let Some(url) = &mut r.url {
        *url = redact_url(url);
    }
    r.extracted = redact_vars(&r.extracted);
    if let Some(resp) = r.response.as_mut() {
        miyo_sync::redact::redact_header_pairs(&mut resp.headers);
        let (masked, changed) = miyo_sync::redact::redact_json_text(&resp.body);
        if changed {
            resp.body = masked;
        }
        if let Some(snapshot) = resp.request.as_mut() {
            redact_snapshot(snapshot);
        }
    }
    for o in &mut r.outcomes {
        // 断言 expected/actual 是文本：URL 凭据脱敏 + JWT 形态判定
        //（JSON 形态文本交给 redact_json_text 保守处理）。
        for slot in [&mut o.expected, &mut o.actual] {
            let Some(text) = slot.as_mut() else { continue };
            let (masked, changed) = miyo_sync::redact::redact_json_text(text);
            if changed {
                *text = masked;
            }
            let redacted = miyo_sync::redact::redact_message_urls(text);
            if redacted != *text {
                *text = redacted;
            }
            if miyo_core::sensitive::looks_like_secret(text) {
                *text = miyo_sync::redact::PLACEHOLDER.to_string();
            }
        }
    }
}

/// TestCase 返回脱敏（单源 [`miyo_sync::redact::redact_test_case`]）。
pub(crate) fn redact_test_case(case: &mut miyo_core::model::TestCase) {
    miyo_sync::redact::redact_test_case(case);
}
