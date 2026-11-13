//! 运行失败分析（启发式规则，输出标注为推测）。
//!
//! 从 `server.rs` 的 analyze_run_handler 抽出的纯函数 + 增强：
//! - 新增错误类型 `VARIABLE_EXTRACT_FAILED`（下游因缺变量被阻断时，回溯
//!   找到「响应可能包含该字段但未配置提取」的上游步骤并标注根因）与
//!   `NETWORK_UNREACHABLE`（连接失败 / DNS / 超时，区别于一般请求错误）；
//! - 输出增加 `rootCauseSummary`（根因链概述）与 `rerunAdvice`
//!   （是否建议直接重跑 + 理由）。
//!
//! 调用方（server handler / Tauri 命令）负责加载运行历史、脱敏与历史对比，
//! 本模块只做纯分析。

use std::collections::HashSet;

use serde::Serialize;
use uuid::Uuid;

use miyo_test::runner::EndpointResult;

/// 历史对比条目：一次更早运行的失败断言键集合（键 = `{步位次}::{断言描述}`）。
#[derive(Debug, Clone)]
pub struct RunHistoryEntry {
    pub finished_at: String,
    pub failed_assertion_keys: HashSet<String>,
}

/// 运行级元信息（scenario_runs 行的摘要部分）。
#[derive(Debug, Clone)]
pub struct RunMeta {
    pub ok: bool,
    pub total_steps: i64,
    pub passed: i64,
    pub failed: i64,
    pub skipped: i64,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    pub finished_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecurringFailure {
    pub consecutive_runs: i64,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedStepAnalysis {
    pub index: usize,
    pub name: String,
    pub method: String,
    /// 历史对比：同一断言在最近 N 次运行中连续失败（环境漂移 / 接口行为
    /// 变更的强信号）。None = 首次失败或无历史可比。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurring: Option<RecurringFailure>,
    /// 错误类型（REQUEST_ERROR / NETWORK_UNREACHABLE / ASSERTION_FAILED /
    /// DEPENDENCY_BLOCKED / RUN_CANCELLED / LOOP_TIMEOUT / URL_SCHEME_INVALID /
    /// VARIABLE_EXTRACT_FAILED…）。
    pub error_type: String,
    pub error: Option<String>,
    pub status: Option<u16>,
    /// 可能原因（推测，非确定结论）。
    pub possible_causes: Vec<String>,
    /// 建议处理方式。
    pub suggestions: Vec<String>,
    /// 失败断言明细。
    pub assertion_failures: Vec<serde_json::Value>,
    /// 本步是否为本次失败的根因步骤（链路上最早失败 / 唯一非 blocked 失败）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_cause: Option<bool>,
    /// 执行阶段（setup / main / teardown；缺省视为 main，兼容旧数据）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
}

/// 重跑建议。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RerunAdvice {
    pub should_rerun: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureAnalysis {
    pub run_id: Uuid,
    pub ok: bool,
    pub total_steps: i64,
    pub passed: i64,
    pub failed: i64,
    pub skipped: i64,
    pub cancelled: bool,
    pub stopped_on_failure: bool,
    pub failed_steps: Vec<FailedStepAnalysis>,
    /// 根因链概述（最早失败步骤 + 被阻断的下游数量）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_cause_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerun_advice: Option<RerunAdvice>,
    /// 提示：分析为规则推测，修复前请用 get_step_result 核对原始响应。
    pub disclaimer: String,
}

/// 纯分析入口：`results` 为本次运行的全部步骤结果（已脱敏），
/// `history` 为更早运行的失败断言键（新→旧）。
pub fn analyze_run_results(
    run_id: Uuid,
    meta: &RunMeta,
    results: &[EndpointResult],
    history: &[RunHistoryEntry],
) -> FailureAnalysis {
    // ---- 根因链：最早的非 blocked 失败主步骤；其后 blocked 的步骤视为连锁 ----
    let active: Vec<&EndpointResult> = results
        .iter()
        .filter(|r| r.effective_status() != "success")
        .collect();
    let root_index = results
        .iter()
        .position(|r| {
            let s = r.effective_status();
            (s == "failed" || s == "cancelled") && r.phase.as_deref().unwrap_or("main") == "main"
        })
        .map(|i| results[i].step_index.unwrap_or(i));
    let blocked_count = active
        .iter()
        .filter(|r| r.effective_status() == "blocked")
        .count();
    let root_cause_summary = root_index.map(|idx| {
        let name = results
            .iter()
            .find(|r| r.step_index == Some(idx))
            .map(|r| r.endpoint_name.clone())
            .unwrap_or_default();
        if blocked_count > 0 {
            format!(
                "根因在步骤 {}「{name}」；下游 {} 个步骤因依赖其产出变量被连锁阻断",
                idx + 1,
                blocked_count
            )
        } else {
            format!("根因在步骤 {}「{name}」", idx + 1)
        }
    });

    let mut failed_steps = Vec::new();
    for r in &active {
        let status = r.effective_status();
        let mut causes: Vec<String> = Vec::new();
        let mut suggestions: Vec<String> = Vec::new();
        let mut error = r.request_error.clone();
        let is_root = root_index.is_some_and(|idx| r.step_index == Some(idx));

        let mut error_type: String = match status {
            "blocked" => {
                causes.push(
                    r.blocked_reason
                        .clone()
                        .unwrap_or_else(|| "前序步骤失败，本步依赖的变量未产出".into()),
                );
                suggestions.push("先修复失败的前序步骤，再整体重跑（或用从该步重跑能力）".into());
                "DEPENDENCY_BLOCKED"
            }
            "cancelled" => {
                causes.push("运行被用户或系统取消".into());
                suggestions.push("如需继续请重新运行场景".into());
                "RUN_CANCELLED"
            }
            "skipped" => {
                causes.push("执行条件未满足（condition 不成立），本步被跳过".into());
                "SKIPPED"
            }
            _ if r
                .request_error
                .as_deref()
                .is_some_and(|e| e.contains("循环条件")) =>
            {
                causes.push("循环条件在最大轮数内未满足（轮询超时）".into());
                suggestions
                    .push("检查轮询目标服务的状态变更速度；适当调大 maxLoops / intervalMs".into());
                "LOOP_TIMEOUT"
            }
            _ if r
                .request_error
                .as_deref()
                .is_some_and(|e| e.contains("http/https")) =>
            {
                causes
                    .push("URL 协议非法（仅允许 http/https）——可能因变量替换产生了意外地址".into());
                suggestions.push("检查 base_url 与 URL 模板的变量替换结果".into());
                "URL_SCHEME_INVALID"
            }
            _ if r.request_error.as_deref().is_some_and(is_network_error) => {
                let msg = r.request_error.clone().unwrap_or_default();
                causes.push(format!("网络不可达：{msg}"));
                causes.push(
                    "目标服务未启动 / 端口错误 / 环境选错（本地调试先确认 Mock 服务已启动）".into(),
                );
                suggestions
                    .push("确认环境 Base URL 指向的服务可达后重跑（此类错误重跑有意义）".into());
                "NETWORK_UNREACHABLE"
            }
            _ if r.request_error.is_some() => {
                let msg = r.request_error.clone().unwrap_or_default();
                causes.push(format!("请求未成功发送或连接失败：{msg}"));
                causes.push("目标服务不可达 / 域名或端口错误 / 环境选择错误".into());
                suggestions
                    .push("用 send_endpoint 单独探测该接口；确认环境的 Base URL 是否正确".into());
                "REQUEST_ERROR"
            }
            _ => {
                let st = r.status.unwrap_or(0);
                match st {
                    401 | 403 => {
                        causes.push("认证失败：Token 缺失 / 过期 / 未被变量替换".into());
                        suggestions
                            .push("检查认证配置与 {{token}} 类变量是否来自前序登录步骤".into());
                    }
                    404 | 405 => {
                        causes.push("路径或方法不存在：URL 拼接 / 环境指向错误".into());
                        suggestions.push("核对实际请求 URL 与接口定义；确认环境 Base URL".into());
                    }
                    500..=599 => {
                        causes.push("服务端错误：可能由请求参数（变量值）不合法引发".into());
                        suggestions.push("检查请求 Body / Query 中变量替换后的实际值".into());
                    }
                    _ => {}
                }
                error = Some(format!(
                    "HTTP {st}：{} 条断言未通过",
                    r.outcomes.iter().filter(|o| !o.passed).count()
                ));
                suggestions
                    .push("对比断言的预期值与实际值；确认上游返回结构是否与提取假设一致".into());
                "ASSERTION_FAILED"
            }
        }
        .to_string();
        // 变量残留检查：URL / 提取值中含未解析 {{ }} → 变量未定义。
        if let Some(url) = &r.url {
            if url.contains("{{") {
                if error_type == "REQUEST_ERROR" || error_type == "ASSERTION_FAILED" {
                    error_type = "VARIABLE_EXTRACT_FAILED".into();
                }
                causes.push("请求 URL 仍含 {{占位}}——变量未定义或前序提取失败".into());
                suggestions
                    .push("运行前用 validate_scenario 做变量分析；补齐提取规则或环境变量".into());
                // 回溯提示：哪个上游步骤可能产出该变量。
                let idx = r.step_index.unwrap_or_default();
                if let Some(producer) = find_missing_var_producer(idx, r, results) {
                    causes.push(format!(
                        "上游步骤 {}「{}」的响应可能包含该字段但未配置提取规则",
                        producer.0 + 1,
                        producer.1
                    ));
                }
            }
        }
        let assertion_failures: Vec<serde_json::Value> = r
            .outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| {
                serde_json::json!({
                    "description": o.description,
                    "expected": o.expected,
                    "actual": o.actual,
                    "reason": o.reason,
                })
            })
            .collect();
        // 连续失败计数：按当前失败断言键在前史中逐次回溯。
        let idx = r.step_index.unwrap_or_default();
        let current_keys: HashSet<String> = r
            .outcomes
            .iter()
            .filter(|o| !o.passed)
            .map(|o| format!("{idx}::{}", o.description))
            .collect();
        let mut consecutive = 1i64;
        let mut last_seen = meta.finished_at.clone();
        for entry in history.iter().take(9) {
            if !current_keys.is_empty() && current_keys.is_subset(&entry.failed_assertion_keys) {
                consecutive += 1;
                last_seen = entry.finished_at.clone();
            } else {
                break;
            }
        }
        let recurring = if consecutive >= 2 && !assertion_failures.is_empty() {
            Some(RecurringFailure {
                consecutive_runs: consecutive,
                last_seen_at: last_seen,
            })
        } else {
            None
        };
        if let Some(rec) = &recurring {
            suggestions.push(format!(
                "该断言已连续 {} 次运行失败（最近 {}）——优先怀疑环境漂移或接口行为变更，而非脚本问题",
                rec.consecutive_runs, rec.last_seen_at
            ));
        }
        if is_root {
            suggestions.push("这是本次失败的根因步骤：修复它即可解除下游连锁阻断".into());
        }
        failed_steps.push(FailedStepAnalysis {
            index: r.step_index.unwrap_or_default(),
            name: r.endpoint_name.clone(),
            method: r.method.clone(),
            recurring,
            error_type,
            error,
            status: r.status,
            possible_causes: causes,
            suggestions,
            assertion_failures,
            root_cause: if is_root { Some(true) } else { None },
            phase: r.phase.clone(),
        });
    }

    // ---- 重跑建议：瞬时类错误（网络 / 超时 / 取消）重跑有意义；断言 / 参数类先修 ----
    let transient = failed_steps.iter().any(|s| {
        matches!(
            s.error_type.as_str(),
            "NETWORK_UNREACHABLE" | "REQUEST_ERROR" | "RUN_CANCELLED"
        )
    }) || meta.cancelled;
    let rerun_advice = if meta.ok {
        None
    } else if transient {
        Some(RerunAdvice {
            should_rerun: true,
            reason: "失败类型为网络 / 连接类（或运行被取消），目标服务恢复后直接重跑即可".into(),
        })
    } else {
        Some(RerunAdvice {
            should_rerun: false,
            reason: "失败类型为断言 / 参数 / 变量类，先按修复建议调整场景或环境再重跑".into(),
        })
    };

    FailureAnalysis {
        run_id,
        ok: meta.ok,
        total_steps: meta.total_steps,
        passed: meta.passed,
        failed: meta.failed,
        skipped: meta.skipped,
        cancelled: meta.cancelled,
        stopped_on_failure: meta.stopped_on_failure,
        failed_steps,
        root_cause_summary,
        rerun_advice,
        disclaimer:
            "以上原因为规则推测，非确定结论；修复前请用 get_step_result 核对原始请求与响应。".into(),
    }
}

/// 从数据库加载并分析一次运行（server handler 与 Tauri 命令共用）：
/// 读运行 → 脱敏（兜底旧明文历史）→ 历史对比 → 纯分析。
pub async fn analyze_run(
    db: &sqlx::SqlitePool,
    run_id: Uuid,
) -> Result<FailureAnalysis, miyo_core::AppError> {
    use miyo_storage::repository as repo;
    let Some(run) = repo::get_scenario_run(db, run_id).await? else {
        return Err(miyo_core::AppError::NotFound(format!(
            "运行（{run_id}）不存在"
        )));
    };
    let mut results: Vec<EndpointResult> =
        serde_json::from_str(&run.results_json).unwrap_or_default();
    for r in &mut results {
        crate::redact::redact_endpoint_result(r);
    }
    let previous_runs = repo::list_scenario_runs(db, run.scenario_id, 10)
        .await
        .unwrap_or_default();
    let history: Vec<RunHistoryEntry> = previous_runs
        .iter()
        .filter(|r| r.id != run.id)
        .map(|prev| {
            let prev_results: Vec<EndpointResult> =
                serde_json::from_str(&prev.results_json).unwrap_or_default();
            let failed_assertion_keys: HashSet<String> = prev_results
                .iter()
                .filter(|pr| !pr.ok && !pr.cancelled)
                .flat_map(|pr| {
                    let idx = pr.step_index.unwrap_or_default();
                    pr.outcomes
                        .iter()
                        .filter(|o| !o.passed)
                        .map(move |o| format!("{idx}::{}", o.description))
                })
                .collect();
            RunHistoryEntry {
                finished_at: prev.finished_at.to_rfc3339(),
                failed_assertion_keys,
            }
        })
        .collect();
    let meta = RunMeta {
        ok: run.fail_count == 0 && !run.cancelled,
        total_steps: run.total_steps,
        passed: run.ok_count,
        failed: run.fail_count,
        skipped: run.skipped_count,
        cancelled: run.cancelled,
        stopped_on_failure: run.stopped_on_failure,
        finished_at: run.finished_at.to_rfc3339(),
    };
    Ok(analyze_run_results(run.id, &meta, &results, &history))
}

/// 网络类错误关键词。
fn is_network_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("connection refused")
        || m.contains("connection closed")
        || m.contains("timed out")
        || m.contains("timeout")
        || m.contains("dns error")
        || m.contains("failed to lookup")
        || m.contains("connect error")
        || m.contains("network")
}

/// 为 blocked 步骤回溯「响应可能包含缺失变量但未提取」的上游步骤：
/// 返回（步位次，步名）。
fn find_missing_var_producer(
    blocked_index: usize,
    blocked: &EndpointResult,
    results: &[EndpointResult],
) -> Option<(usize, String)> {
    // 从 blocked_reason / URL 中提取缺失变量名。
    let text = format!(
        "{} {}",
        blocked.blocked_reason.clone().unwrap_or_default(),
        blocked.url.clone().unwrap_or_default()
    );
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
        regex::Regex::new(r"\{\{\s*([A-Za-z_][A-Za-z0-9_.]*)\s*\}\}").unwrap()
    });
    let missing: Vec<String> = RE.captures_iter(&text).map(|c| c[1].to_string()).collect();
    if missing.is_empty() {
        return None;
    }
    for r in results.iter().take(blocked_index) {
        if r.effective_status() != "success" {
            continue;
        }
        if let Some(resp) = &r.response {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&resp.body) {
                let keys = collect_object_keys(&v, 3);
                if missing.iter().any(|m| keys.contains(m)) {
                    return Some((r.step_index.unwrap_or_default(), r.endpoint_name.clone()));
                }
            }
        }
    }
    None
}

fn collect_object_keys(value: &serde_json::Value, depth: u8) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(v: &serde_json::Value, depth: u8, out: &mut Vec<String>) {
        if depth == 0 {
            return;
        }
        match v {
            serde_json::Value::Object(map) => {
                for (k, inner) in map {
                    out.push(k.clone());
                    walk(inner, depth - 1, out);
                }
            }
            serde_json::Value::Array(list) => {
                if let Some(first) = list.first() {
                    walk(first, depth - 1, out);
                }
            }
            _ => {}
        }
    }
    walk(value, depth, &mut out);
    out
}
