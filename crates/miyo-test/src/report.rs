//! 场景测试报告（运行结果增强 / 交付 §八）。
//!
//! 输入 = 一批运行（同一场景的多次 / 多环境 / 多数据组结果），输出：
//! - [`ReportStats`]：场景成功率 / 步骤成功率 / 平均耗时 / P50-P99 /
//!   失败步骤统计 / 断言失败统计 / 环境横向对比；
//! - [`junit_xml`]：JUnit XML（CI 消费，每步一个 testcase）；
//! - [`html_report`]：自包含 HTML（无外部资源，可直接归档 / 分享）。
//!
//! 统计口径：**主步骤**（`phase = None`）——setup / teardown 结果不计入
//! 成功率；每步请求耗时取 `response.duration_ms`，缺失（未发送）不计入分位数。

use serde::Serialize;
use std::collections::BTreeMap;

use crate::runner::EndpointResult;

/// 参与统计的一次运行。
#[derive(Debug, Clone)]
pub struct ReportRun {
    /// 运行 id（历史链接）。
    pub run_id: String,
    /// 运行结束时刻（RFC3339）。
    pub finished_at: String,
    /// 环境名（环境矩阵对比维度；None = 单环境历史运行）。
    pub environment: Option<String>,
    /// 数据组名（数据驱动维度；None = 普通运行）。
    pub data_set: Option<String>,
    /// 总耗时毫秒。
    pub duration_ms: Option<f64>,
    /// 运行是否整体通过（主步骤全部成功）。
    pub ok: bool,
    /// 主步骤结果（phase = None 的行）。
    pub results: Vec<EndpointResult>,
}

/// 百分位（毫秒）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Percentiles {
    pub p50: f64,
    pub p90: f64,
    pub p95: f64,
    pub p99: f64,
}

/// 步骤失败统计（跨运行聚合：同名步骤多次失败合并计数）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StepFailureStat {
    pub step_name: String,
    pub fail_count: u64,
    /// 最近一次失败原因（定位入口）。
    pub last_error: Option<String>,
}

/// 断言失败统计（按断言描述聚合）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AssertFailStat {
    pub description: String,
    pub fail_count: u64,
    pub last_reason: Option<String>,
}

/// 环境横向对比行。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct EnvStat {
    pub environment: String,
    pub runs: usize,
    pub scenario_ok: usize,
    pub step_ok: u64,
    pub step_total: u64,
    pub avg_duration_ms: f64,
}

/// 报告统计（交付 §八）。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ReportStats {
    pub scenario_name: String,
    /// 参与统计的运行数。
    pub run_count: usize,
    /// 场景成功率（整体通过的运行 / 总运行，0..1）。
    pub scenario_success_rate: f64,
    /// 步骤成功率（主步骤成功数 / 主步骤总数，0..1）。
    pub step_success_rate: f64,
    /// 平均步骤耗时毫秒。
    pub avg_step_duration_ms: f64,
    /// 步骤耗时分位数。
    pub percentiles: Percentiles,
    /// 失败步骤统计（按失败次数降序）。
    pub failed_steps: Vec<StepFailureStat>,
    /// 断言失败统计（按失败次数降序）。
    pub assertion_failures: Vec<AssertFailStat>,
    /// 环境执行对比（按环境名分组；单环境运行也有一行）。
    pub env_comparison: Vec<EnvStat>,
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// 计算报告统计。
pub fn compute_stats(scenario_name: &str, runs: &[ReportRun]) -> ReportStats {
    let run_count = runs.len();
    let scenario_ok = runs.iter().filter(|r| r.ok).count();

    let mut step_total = 0u64;
    let mut step_ok = 0u64;
    let mut durations: Vec<f64> = Vec::new();
    let mut failed_steps: BTreeMap<String, (u64, Option<String>)> = BTreeMap::new();
    let mut assert_failures: BTreeMap<String, (u64, Option<String>)> = BTreeMap::new();

    for run in runs {
        for r in &run.results {
            // 只统计主步骤（setup / teardown 不计入成功率）。
            if r.phase.is_some() {
                continue;
            }
            step_total += 1;
            if let Some(d) = r.duration_ms {
                durations.push(d);
            }
            if !r.ok && !r.cancelled {
                let entry = failed_steps
                    .entry(r.endpoint_name.clone())
                    .or_insert((0, None));
                entry.0 += 1;
                entry.1 = r.request_error.clone().or_else(|| {
                    r.outcomes
                        .iter()
                        .find(|o| !o.passed)
                        .and_then(|o| o.reason.clone())
                });
            }
            for o in r.outcomes.iter().filter(|o| !o.passed) {
                let entry = assert_failures
                    .entry(o.description.clone())
                    .or_insert((0, None));
                entry.0 += 1;
                entry.1 = o.reason.clone();
            }
            if r.ok {
                step_ok += 1;
            }
        }
    }
    durations.sort_by(|a, b| a.total_cmp(b));

    // 环境对比：无环境标记的运行归入「默认」。
    let mut env_map: BTreeMap<String, Vec<&ReportRun>> = BTreeMap::new();
    for run in runs {
        let key = run.environment.clone().unwrap_or_else(|| "默认".into());
        env_map.entry(key).or_default().push(run);
    }
    let env_comparison: Vec<EnvStat> = env_map
        .into_iter()
        .map(|(env, rs)| {
            let mut scenario_ok = 0usize;
            let mut step_ok = 0u64;
            let mut total = 0u64;
            let mut dur_sum = 0.0;
            let mut dur_n = 0usize;
            for run in &rs {
                if run.ok {
                    scenario_ok += 1;
                }
                for r in &run.results {
                    if r.phase.is_some() {
                        continue;
                    }
                    total += 1;
                    if r.ok {
                        step_ok += 1;
                    }
                    if let Some(d) = r.duration_ms {
                        dur_sum += d;
                        dur_n += 1;
                    }
                }
            }
            EnvStat {
                environment: env,
                runs: rs.len(),
                scenario_ok,
                step_ok,
                step_total: total,
                avg_duration_ms: if dur_n > 0 {
                    dur_sum / dur_n as f64
                } else {
                    0.0
                },
            }
        })
        .collect();

    ReportStats {
        scenario_name: scenario_name.to_string(),
        run_count,
        scenario_success_rate: if run_count > 0 {
            scenario_ok as f64 / run_count as f64
        } else {
            0.0
        },
        step_success_rate: if step_total > 0 {
            step_ok as f64 / step_total as f64
        } else {
            0.0
        },
        avg_step_duration_ms: if durations.is_empty() {
            0.0
        } else {
            durations.iter().sum::<f64>() / durations.len() as f64
        },
        percentiles: Percentiles {
            p50: percentile(&durations, 0.50),
            p90: percentile(&durations, 0.90),
            p95: percentile(&durations, 0.95),
            p99: percentile(&durations, 0.99),
        },
        failed_steps: {
            let mut v: Vec<StepFailureStat> = failed_steps
                .into_iter()
                .map(|(step_name, (c, e))| StepFailureStat {
                    step_name,
                    fail_count: c,
                    last_error: e,
                })
                .collect();
            v.sort_by_key(|f| std::cmp::Reverse(f.fail_count));
            v
        },
        assertion_failures: {
            let mut v: Vec<AssertFailStat> = assert_failures
                .into_iter()
                .map(|(description, (c, e))| AssertFailStat {
                    description,
                    fail_count: c,
                    last_reason: e,
                })
                .collect();
            v.sort_by_key(|f| std::cmp::Reverse(f.fail_count));
            v
        },
        env_comparison,
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn html_escape(s: &str) -> String {
    xml_escape(s)
}

/// JUnit XML：一次运行 = 一个 testcase 集合（场景为 suite，每步一个
/// testcase；失败步带 failure 节点）。CI（GitHub Actions / Jenkins）直接消费。
pub fn junit_xml(scenario_name: &str, runs: &[ReportRun]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let total_tests: usize = runs
        .iter()
        .map(|r| r.results.iter().filter(|x| x.phase.is_none()).count())
        .sum();
    let total_fail: usize = runs
        .iter()
        .map(|r| {
            r.results
                .iter()
                .filter(|x| x.phase.is_none() && !x.ok && !x.cancelled)
                .count()
        })
        .sum();
    out.push_str(&format!(
        "<testsuites name=\"{}\" tests=\"{}\" failures=\"{}\">\n",
        xml_escape(scenario_name),
        total_tests,
        total_fail
    ));
    for run in runs {
        let suite = match (&run.environment, &run.data_set) {
            (Some(e), Some(d)) => format!("{scenario_name} [{e}/{d}]"),
            (Some(e), None) => format!("{scenario_name} [{e}]"),
            (None, Some(d)) => format!("{scenario_name} [{d}]"),
            _ => scenario_name.to_string(),
        };
        out.push_str(&format!(
            "<testsuite name=\"{}\" tests=\"{}\" failures=\"{}\" time=\"{:.3}\">\n",
            xml_escape(&suite),
            run.results.iter().filter(|x| x.phase.is_none()).count(),
            run.results
                .iter()
                .filter(|x| x.phase.is_none() && !x.ok && !x.cancelled)
                .count(),
            run.duration_ms.unwrap_or(0.0) / 1000.0
        ));
        for r in run.results.iter().filter(|x| x.phase.is_none()) {
            let dur = r.duration_ms.unwrap_or(0.0) / 1000.0;
            let failed = !r.ok && !r.cancelled;
            out.push_str(&format!(
                "<testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\"",
                xml_escape(&r.endpoint_name),
                xml_escape(&run.run_id),
                dur
            ));
            if failed {
                out.push_str(">\n");
                let reason = r
                    .request_error
                    .clone()
                    .or_else(|| {
                        r.outcomes.iter().find(|o| !o.passed).map(|o| {
                            format!(
                                "{}: {}",
                                o.description,
                                o.reason.clone().unwrap_or_default()
                            )
                        })
                    })
                    .unwrap_or_else(|| "步骤失败".into());
                out.push_str(&format!(
                    "<failure message=\"{}\">{}</failure>\n",
                    xml_escape(reason.lines().next().unwrap_or("失败")),
                    xml_escape(&reason)
                ));
                out.push_str("</testcase>\n");
            } else if r.cancelled {
                out.push_str(">\n<skipped/>\n</testcase>\n");
            } else {
                out.push_str("/>\n");
            }
        }
        out.push_str("</testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}

/// 自包含 HTML 报告（无外部资源；统计 + 每运行步骤表 + 环境对比）。
pub fn html_report(scenario_name: &str, stats: &ReportStats, runs: &[ReportRun]) -> String {
    let pct = |v: f64| format!("{:.1}%", v * 100.0);
    let mut rows = String::new();
    for run in runs {
        for r in &run.results {
            let status = match (&r.step_status, r.status) {
                (Some(s), _) => s.clone(),
                (None, Some(c)) => c.to_string(),
                _ => "-".into(),
            };
            let phase = r.phase.clone().unwrap_or_else(|| "main".into());
            rows.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&run.run_id),
                html_escape(&phase),
                html_escape(&r.endpoint_name),
                html_escape(&status),
                r.duration_ms
                    .map(|d| format!("{d:.1}ms"))
                    .unwrap_or_default(),
                if r.ok { "✓" } else { "✗" }
            ));
        }
    }
    let failed_rows: String = stats
        .failed_steps
        .iter()
        .map(|f| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                html_escape(&f.step_name),
                f.fail_count,
                html_escape(f.last_error.as_deref().unwrap_or("-"))
            )
        })
        .collect();
    let env_rows: String = stats
        .env_comparison
        .iter()
        .map(|e| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{:.1}ms</td></tr>",
                html_escape(&e.environment),
                e.runs,
                if e.step_total > 0 {
                    format!("{}/{}", e.step_ok, e.step_total)
                } else {
                    "-".into()
                },
                pct(if e.step_total > 0 {
                    e.step_ok as f64 / e.step_total as f64
                } else {
                    0.0
                }),
                e.avg_duration_ms
            )
        })
        .collect();
    format!(
        r#"<!DOCTYPE html>
<html lang="zh"><head><meta charset="utf-8">
<title>{name} · 测试报告</title>
<style>
body {{ font-family: system-ui, sans-serif; margin: 24px; color: #1f2328; }}
h1 {{ font-size: 20px; }} h2 {{ font-size: 15px; margin-top: 24px; }}
table {{ border-collapse: collapse; width: 100%; font-size: 13px; }}
td, th {{ border: 1px solid #d0d7de; padding: 4px 8px; text-align: left; }}
th {{ background: #f6f8fa; }}
.kpi {{ display: flex; gap: 16px; flex-wrap: wrap; margin: 12px 0; }}
.kpi div {{ background: #f6f8fa; border: 1px solid #d0d7de; border-radius: 6px; padding: 8px 14px; }}
.kpi b {{ font-size: 16px; }}
</style></head><body>
<h1>{name} · 测试报告</h1>
<div class="kpi">
<div>运行数 <b>{run_count}</b></div>
<div>场景成功率 <b>{rate}</b></div>
<div>步骤成功率 <b>{step_rate}</b></div>
<div>平均耗时 <b>{avg:.1}ms</b></div>
<div>P50 <b>{p50:.1}ms</b></div>
<div>P90 <b>{p90:.1}ms</b></div>
<div>P95 <b>{p95:.1}ms</b></div>
<div>P99 <b>{p99:.1}ms</b></div>
</div>
<h2>环境对比</h2>
<table><tr><th>环境</th><th>运行数</th><th>步骤通过</th><th>成功率</th><th>平均耗时</th></tr>{env_rows}</table>
<h2>失败步骤</h2>
<table><tr><th>步骤</th><th>失败次数</th><th>最近原因</th></tr>{failed_rows}</table>
<h2>步骤明细</h2>
<table><tr><th>运行</th><th>阶段</th><th>步骤</th><th>状态</th><th>耗时</th><th>结果</th></tr>{rows}</table>
</body></html>
"#,
        name = html_escape(scenario_name),
        run_count = stats.run_count,
        rate = pct(stats.scenario_success_rate),
        step_rate = pct(stats.step_success_rate),
        avg = stats.avg_step_duration_ms,
        p50 = stats.percentiles.p50,
        p90 = stats.percentiles.p90,
        p95 = stats.percentiles.p95,
        p99 = stats.percentiles.p99,
        env_rows = env_rows,
        failed_rows = failed_rows,
        rows = rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assert::Outcome;
    use crate::runner::StepResponse;
    use std::collections::HashMap;

    fn step(
        name: &str,
        ok: bool,
        duration: f64,
        phase: Option<&str>,
        failed_assert: Option<&str>,
    ) -> EndpointResult {
        EndpointResult {
            endpoint_id: uuid::Uuid::new_v4(),
            endpoint_name: name.into(),
            method: "GET".into(),
            path: "/x".into(),
            ok,
            status: if ok { Some(200) } else { Some(500) },
            duration_ms: Some(duration),
            request_error: None,
            outcomes: match failed_assert {
                Some(desc) => vec![Outcome {
                    description: desc.into(),
                    passed: false,
                    reason: Some("值不一致".into()),
                    expected: None,
                    actual: None,
                }],
                None => vec![],
            },
            extracted: HashMap::new(),
            url: None,
            step_index: Some(0),
            cancelled: false,
            response: Some(StepResponse {
                status: 200,
                headers: vec![],
                body: "{}".into(),
                content_type: "application/json".into(),
                duration_ms: duration,
                size_bytes: 2,
                truncated: false,
                body_omitted: false,
                history_truncated: false,
                request: None,
            }),
            step_status: Some(if ok {
                "success".into()
            } else {
                "failed".into()
            }),
            blocked_reason: None,
            loop_runs: None,
            phase: phase.map(|p| p.into()),
            started_at: None,
            retry_log: Vec::new(),
        }
    }

    fn run(id: &str, ok: bool, env: Option<&str>, results: Vec<EndpointResult>) -> ReportRun {
        ReportRun {
            run_id: id.into(),
            finished_at: "2026-11-12T00:00:00Z".into(),
            environment: env.map(|e| e.into()),
            data_set: None,
            duration_ms: Some(results.iter().filter_map(|r| r.duration_ms).sum()),
            ok,
            results,
        }
    }

    #[test]
    fn stats_rates_percentiles_and_failures() {
        let runs = vec![
            run(
                "r1",
                true,
                Some("dev"),
                vec![
                    step("登录", true, 100.0, None, None),
                    step("下单", true, 200.0, None, None),
                ],
            ),
            run(
                "r2",
                false,
                Some("dev"),
                vec![
                    step("登录", true, 150.0, None, None),
                    step("下单", false, 300.0, None, Some("状态码 eq 200")),
                ],
            ),
            run(
                "r3",
                false,
                Some("prod"),
                vec![
                    step("登录", false, 90.0, None, Some("状态码 eq 200")),
                    step("下单", true, 180.0, None, None),
                ],
            ),
        ];
        let stats = compute_stats("下单链路", &runs);
        assert_eq!(stats.run_count, 3);
        assert!((stats.scenario_success_rate - 1.0 / 3.0).abs() < 1e-9);
        // 主步骤 6 个，成功 4。
        assert!((stats.step_success_rate - 4.0 / 6.0).abs() < 1e-9);
        let ds: Vec<f64> = vec![90.0, 100.0, 150.0, 180.0, 200.0, 300.0];
        assert!((stats.percentiles.p50 - ds[3]).abs() < 1e-9);
        assert!((stats.avg_step_duration_ms - ds.iter().sum::<f64>() / 6.0).abs() < 1e-9);
        // 失败步骤统计：下单 1 次、登录 1 次（并列时按序不定，查成员）。
        assert_eq!(stats.failed_steps.len(), 2);
        assert!(stats
            .failed_steps
            .iter()
            .any(|f| f.step_name == "登录" && f.fail_count == 1));
        assert!(stats
            .failed_steps
            .iter()
            .any(|f| f.step_name == "下单" && f.fail_count == 1));
        // 断言失败统计。
        assert_eq!(stats.assertion_failures.len(), 1);
        assert_eq!(stats.assertion_failures[0].fail_count, 2);
        // 环境对比：dev / prod 两行。
        assert_eq!(stats.env_comparison.len(), 2);
    }

    #[test]
    fn teardown_rows_are_excluded_from_rates() {
        let runs = vec![run(
            "r1",
            true,
            None,
            vec![
                step("登录", true, 100.0, None, None),
                step("清理", false, 10.0, Some("teardown"), None),
            ],
        )];
        let stats = compute_stats("x", &runs);
        assert!(
            (stats.step_success_rate - 1.0).abs() < 1e-9,
            "teardown 失败不计入主成功率"
        );
    }

    #[test]
    fn junit_xml_shape_and_failures() {
        let runs = vec![run(
            "r1",
            true,
            Some("dev"),
            vec![
                step("登录", true, 100.0, None, None),
                step("下单", false, 300.0, None, Some("状态码 eq 200")),
            ],
        )];
        let xml = junit_xml("下单链路", &runs);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("tests=\"2\"") && xml.contains("failures=\"1\""));
        assert!(xml.contains("<failure message="));
        assert!(xml.contains("[dev]"), "suite 带环境标注");
        // teardown 行不进 JUnit。
        let runs2 = vec![run(
            "r2",
            true,
            None,
            vec![
                step("登录", true, 100.0, None, None),
                step("清理", false, 10.0, Some("teardown"), None),
            ],
        )];
        let xml2 = junit_xml("x", &runs2);
        assert!(xml2.contains("tests=\"1\"") && xml2.contains("failures=\"0\""));
    }

    #[test]
    fn html_report_is_self_contained() {
        let runs = vec![run(
            "r1",
            true,
            Some("dev"),
            vec![
                step("登录", true, 100.0, None, None),
                step("下单", false, 300.0, None, Some("状态码 eq 200")),
            ],
        )];
        let stats = compute_stats("下单链路", &runs);
        let html = html_report("下单链路", &stats, &runs);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("P95"));
        assert!(html.contains("下单"));
        assert!(html.contains("环境对比") && html.contains("失败步骤"));
        // XML 转义：无裸 <>
        assert!(!html.contains("<script"));
    }
}
