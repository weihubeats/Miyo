//! 冒烟测试文档导出：将测试用例按分组渲染为 Markdown。

use std::collections::HashMap;

use chrono::Utc;
use miyo_core::model::{Endpoint, KeyValue, TestCase, TestCaseStatus};
use serde::{Deserialize, Serialize};

/// 用例分组固定展示顺序。
const CATEGORY_ORDER: [&str; 5] = ["正向", "负向", "边界值", "安全性", "其他"];

/// 单条用例的运行元信息（前端 `caseRunMeta` 传递，可选）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SmokeRunResult {
    /// HTTP 状态码。
    pub status: i32,
    /// 耗时（毫秒）。
    pub duration_ms: i64,
}

impl SmokeRunResult {
    /// 状态码 → 是否通过（2xx/3xx 视为成功，与前端 runTestCase 判断一致）。
    fn passed(self) -> bool {
        (200..400).contains(&self.status)
    }
}

/// 耗时格式化：<1s 取整毫秒，≥1s 转秒（2 位小数）。
fn format_duration(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.2}s", ms as f64 / 1000.0)
    }
}

/// 分组 → 预期结果文案（用例未存断言，按分组给泛化预期）。
fn expected_of(category: &str) -> &'static str {
    match category {
        "正向" => "请求成功，响应为预期结果（HTTP 2xx，业务返回成功）。",
        "负向" => "请求被正确拒绝（HTTP 4xx / 5xx，业务返回失败）。",
        "边界值" => "响应符合边界定义（成功或明确拒绝，不得异常/超时）。",
        "安全性" => "安全校验生效（未授权 / 非法参数访问被拒绝）。",
        _ => "响应符合业务预期。",
    }
}

/// 用例最近一次是否通过：优先取前端运行元信息（状态码），缺失回退持久化状态。
fn is_passed(c: &TestCase, run_results: &HashMap<uuid::Uuid, SmokeRunResult>) -> bool {
    match run_results.get(&c.id) {
        Some(r) => r.passed(),
        None => c.last_run_status == TestCaseStatus::Success,
    }
}

/// 用例是否未测试：无运行元信息且持久化状态为未测试。
fn is_untested(c: &TestCase, run_results: &HashMap<uuid::Uuid, SmokeRunResult>) -> bool {
    run_results.get(&c.id).is_none() && c.last_run_status == TestCaseStatus::Untested
}

/// 将一组接口及其测试用例渲染为冒烟测试 Markdown 文档。
///
/// - `include_results` 为 true 时在用例详情 / 验收清单中附带最近一次运行结果；
/// - `run_results` 为前端内存态运行元信息（caseId → 状态码/耗时），
///   存在时优先展示（附 HTTP 状态码与耗时），缺失则回退到 `TestCase.last_run_status`。
pub fn render_smoke(
    project_name: &str,
    endpoints: &[Endpoint],
    cases_by_endpoint: &HashMap<uuid::Uuid, Vec<TestCase>>,
    include_results: bool,
    run_results: &HashMap<uuid::Uuid, SmokeRunResult>,
) -> String {
    let mut cases: Vec<&TestCase> = Vec::new();
    for ep in endpoints {
        if let Some(list) = cases_by_endpoint.get(&ep.id) {
            cases.extend(list.iter());
        }
    }

    let mut out = String::with_capacity(4096);
    out.push_str(&format!("# {project_name} 冒烟测试文档\n\n"));
    out.push_str(&format!(
        "> 导出时间：{} · 接口：{} · 用例：{}\n\n",
        Utc::now().format("%Y-%m-%d %H:%M:%S"),
        endpoints.len(),
        cases.len()
    ));

    // 分组统计表
    out.push_str("## 一、测试范围\n\n");
    out.push_str("| 分组 | 用例数 |\n|---|---|\n");
    for cat in CATEGORY_ORDER {
        let n = cases.iter().filter(|c| c.category == cat).count();
        out.push_str(&format!("| {cat} | {n} |\n"));
    }
    if include_results {
        let pass = cases.iter().filter(|c| is_passed(c, run_results)).count();
        let fail = cases.iter().filter(|c| is_untested(c, run_results)).count();
        let total = cases.len();
        out.push_str(&format!(
            "\n运行结果：通过 {pass} · 失败 {un} · 未测试 {fail}\n",
            un = total - pass - fail
        ));
    }
    out.push('\n');
    out.push_str("覆盖接口：\n\n");
    for ep in endpoints {
        out.push_str(&format!(
            "- {}（{} {}）\n",
            ep.name,
            ep.method.as_str(),
            ep.path
        ));
    }
    out.push('\n');

    // 按分组渲染用例
    out.push_str("## 二、测试用例\n\n");
    let mut case_index = 0usize;
    for cat in CATEGORY_ORDER {
        let mut rendered = false;
        for ep in endpoints {
            let list = cases_by_endpoint.get(&ep.id);
            let group: Vec<&TestCase> = list
                .map(|v| v.iter().filter(|c| c.category == cat).collect())
                .unwrap_or_default();
            if group.is_empty() {
                continue;
            }
            if !rendered {
                out.push_str(&format!("### {cat}\n\n"));
                rendered = true;
            }
            out.push_str(&format!(
                "#### {method} {path}（{name}）\n\n",
                method = ep.method.as_str(),
                path = ep.path,
                name = ep.name
            ));
            for c in group {
                case_index += 1;
                render_case(&mut out, case_index, c, include_results, run_results);
            }
            out.push('\n');
        }
    }
    if case_index == 0 {
        out.push_str("（暂无测试用例，请先在接口「测试用例」标签中添加。）\n\n");
    }

    // 验收清单
    out.push_str("## 三、验收清单\n\n");
    if cases.is_empty() {
        out.push_str("（无用例可验收）\n");
    } else {
        for cat in CATEGORY_ORDER {
            let group: Vec<&TestCase> = cases
                .iter()
                .copied()
                .filter(|c| c.category == cat)
                .collect();
            if group.is_empty() {
                continue;
            }
            out.push_str(&format!("{cat}：\n\n"));
            for c in group {
                let status = if include_results {
                    match run_results.get(&c.id) {
                        Some(r) => {
                            if r.passed() {
                                format!(
                                    " ✅ HTTP {} · {}",
                                    r.status,
                                    format_duration(r.duration_ms)
                                )
                            } else {
                                format!(
                                    " ❌ HTTP {} · {}",
                                    r.status,
                                    format_duration(r.duration_ms)
                                )
                            }
                        }
                        None => match c.last_run_status {
                            TestCaseStatus::Success => " ✅".to_string(),
                            TestCaseStatus::Failed => " ❌".to_string(),
                            TestCaseStatus::Untested => " ⬜".to_string(),
                        },
                    }
                } else {
                    String::new()
                };
                out.push_str(&format!(
                    "- [ ] {}{}（{} {}）\n",
                    c.name,
                    status,
                    c.method.as_str(),
                    c.url_path
                ));
            }
            out.push('\n');
        }
    }
    out
}

/// 渲染单个用例：请求行 / 参数表 / Headers / Body / 预期（可选附带运行结果）。
fn render_case(
    out: &mut String,
    index: usize,
    c: &TestCase,
    include_results: bool,
    run_results: &HashMap<uuid::Uuid, SmokeRunResult>,
) {
    let badge = if include_results {
        match run_results.get(&c.id) {
            Some(r) => format!(
                "（{} · HTTP {} · {}）",
                if r.passed() {
                    "✅ 通过"
                } else {
                    "❌ 失败"
                },
                r.status,
                format_duration(r.duration_ms)
            ),
            None => match c.last_run_status {
                TestCaseStatus::Success => "（✅ 通过）".to_string(),
                TestCaseStatus::Failed => "（❌ 失败）".to_string(),
                TestCaseStatus::Untested => "（⬜ 未测试）".to_string(),
            },
        }
    } else {
        String::new()
    };
    out.push_str(&format!("**{index}. {name}**{badge}\n\n", name = c.name));
    out.push_str(&format!("- 请求：`{} {}`\n", c.method.as_str(), c.url_path));

    if !c.params.is_empty() {
        out.push_str("\nQuery 参数：\n\n");
        out.push_str("| 名称 | 值 | 启用 |\n|---|---|---|\n");
        for kv in &c.params {
            out.push_str(&kv_cell(kv));
        }
        out.push('\n');
    }
    if !c.headers.is_empty() {
        out.push_str("请求头：\n\n");
        out.push_str("| 名称 | 值 | 启用 |\n|---|---|---|\n");
        for kv in &c.headers {
            out.push_str(&kv_cell(kv));
        }
        out.push('\n');
    }
    if !c.body_content.trim().is_empty() {
        out.push_str("请求体（");
        out.push_str(&c.body_type);
        out.push_str("）：\n\n```\n");
        out.push_str(c.body_content.trim_end());
        out.push_str("\n```\n\n");
    }

    out.push_str("**预期**：");
    out.push_str(expected_of(&c.category));
    out.push_str("\n\n");
}

/// KeyValue → Markdown 表格行（值含 `|` 时转义）。
fn kv_cell(kv: &KeyValue) -> String {
    format!(
        "| {} | {} | {} |\n",
        kv.key.replace('|', "\\|"),
        kv.value.replace('|', "\\|"),
        if kv.enabled { "是" } else { "否" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use miyo_core::model::{HttpMethod, TestCaseStatus};
    use uuid::Uuid;

    fn ep(name: &str, method: HttpMethod, path: &str) -> Endpoint {
        Endpoint {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            folder_id: None,
            name: name.to_string(),
            method,
            path: path.to_string(),
            description: String::new(),
            status: miyo_core::model::EndpointStatus::Released,
            sort_order: 0,
            request: miyo_core::model::RequestSpec::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn case(ep_id: Uuid, name: &str, category: &str) -> TestCase {
        TestCase {
            id: Uuid::new_v4(),
            request_id: ep_id,
            name: name.to_string(),
            category: category.to_string(),
            folder: String::new(),
            method: HttpMethod::GET,
            url_path: "/api/users/{id}".to_string(),
            path_variables: vec![KeyValue::new("id", "1")],
            params: vec![KeyValue::new("id", "1")],
            headers: vec![KeyValue::new("X-Token", "abc")],
            body_type: "json".to_string(),
            body_content: "{\"a\":1}".to_string(),
            last_run_status: TestCaseStatus::Untested,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn smoke_basic_structure() {
        let e = ep("用户列表", HttpMethod::GET, "/api/users/{id}");
        let mut map = HashMap::new();
        map.insert(
            e.id,
            vec![
                case(e.id, "按 ID 查询", "正向"),
                case(e.id, "非法 ID", "负向"),
            ],
        );
        let md = render_smoke("测试项目", &[e], &map, false, &HashMap::new());

        assert!(md.starts_with("# 测试项目 冒烟测试文档\n"));
        assert!(md.contains("| 正向 | 1 |"));
        assert!(md.contains("| 负向 | 1 |"));
        assert!(md.contains("## 二、测试用例"));
        assert!(md.contains("### 正向"));
        assert!(md.contains("#### GET /api/users/{id}（用户列表）"));
        assert!(md.contains("**1. 按 ID 查询**"));
        assert!(md.contains("`GET /api/users/{id}`"));
        assert!(md.contains("| id | 1 | 是 |"));
        assert!(md.contains("| X-Token | abc | 是 |"));
        assert!(md.contains("```\n{\"a\":1}\n```"));
        assert!(md.contains("**预期**：请求成功"));
        assert!(md.contains("## 三、验收清单"));
        assert!(md.contains("- [ ] 按 ID 查询（GET /api/users/{id}）"));
    }

    #[test]
    fn smoke_empty_cases() {
        let e = ep("空", HttpMethod::GET, "/api/empty");
        let md = render_smoke("P", &[e], &HashMap::new(), false, &HashMap::new());
        assert!(md.contains("（暂无测试用例"));
        assert!(md.contains("（无用例可验收）"));
    }

    #[test]
    fn smoke_category_order() {
        let e = ep("X", HttpMethod::GET, "/x");
        let mut map = HashMap::new();
        map.insert(
            e.id,
            vec![
                case(e.id, "负", "负向"),
                case(e.id, "正", "正向"),
                case(e.id, "边界", "边界值"),
            ],
        );
        let md = render_smoke("P", &[e], &map, false, &HashMap::new());
        let pos = md.find("### 正向").unwrap();
        let neg = md.find("### 负向").unwrap();
        let edge = md.find("### 边界值").unwrap();
        assert!(pos < neg && neg < edge);
    }

    #[test]
    fn smoke_include_results_fallback_to_persisted_status() {
        let e = ep("用户列表", HttpMethod::GET, "/api/users/{id}");
        let mut map = HashMap::new();
        let mut c1 = case(e.id, "按 ID 查询", "正向");
        c1.last_run_status = TestCaseStatus::Success;
        let mut c2 = case(e.id, "非法 ID", "负向");
        c2.last_run_status = TestCaseStatus::Failed;
        map.insert(e.id, vec![c1, c2]);

        let md = render_smoke(
            "测试项目",
            std::slice::from_ref(&e),
            &map,
            true,
            &HashMap::new(),
        );
        assert!(md.contains("运行结果：通过 1 · 失败 1 · 未测试 0"));
        assert!(md.contains("**1. 按 ID 查询**（✅ 通过）"));
        assert!(md.contains("**2. 非法 ID**（❌ 失败）"));
        assert!(md.contains("- [ ] 按 ID 查询 ✅（GET /api/users/{id}）"));
        assert!(md.contains("- [ ] 非法 ID ❌（GET /api/users/{id}）"));

        let md_off = render_smoke("测试项目", &[e], &map, false, &HashMap::new());
        assert!(!md_off.contains("运行结果："));
        assert!(!md_off.contains("（✅ 通过）"));
        assert!(!md_off.contains("按 ID 查询 ✅"));
    }

    #[test]
    fn smoke_include_results_uses_run_meta() {
        let e = ep("用户列表", HttpMethod::GET, "/api/users/{id}");
        let mut map = HashMap::new();
        let c1 = case(e.id, "按 ID 查询", "正向");
        let c2 = case(e.id, "非法 ID", "负向");
        map.insert(e.id, vec![c1.clone(), c2.clone()]);

        let mut run = HashMap::new();
        run.insert(
            c1.id,
            SmokeRunResult {
                status: 200,
                duration_ms: 320,
            },
        );
        run.insert(
            c2.id,
            SmokeRunResult {
                status: 500,
                duration_ms: 1500,
            },
        );

        let md = render_smoke("测试项目", std::slice::from_ref(&e), &map, true, &run);
        // 运行元信息优先：展示状态码与耗时
        assert!(md.contains("**1. 按 ID 查询**（✅ 通过 · HTTP 200 · 320ms）"));
        assert!(md.contains("**2. 非法 ID**（❌ 失败 · HTTP 500 · 1.50s）"));
        assert!(md.contains("运行结果：通过 1 · 失败 1 · 未测试 0"));
        // 验收清单同样携带 HTTP 状态码与耗时
        assert!(md.contains("- [ ] 按 ID 查询 ✅ HTTP 200 · 320ms（GET /api/users/{id}）"));
        assert!(md.contains("- [ ] 非法 ID ❌ HTTP 500 · 1.50s（GET /api/users/{id}）"));
    }

    #[test]
    fn smoke_run_meta_untested_when_missing() {
        let e = ep("用户列表", HttpMethod::GET, "/api/users/{id}");
        let mut map = HashMap::new();
        let mut c1 = case(e.id, "仅持久化通过", "正向");
        c1.last_run_status = TestCaseStatus::Success;
        let c2 = case(e.id, "仅运行元信息通过", "正向");
        map.insert(e.id, vec![c1.clone(), c2.clone()]);

        let mut run = HashMap::new();
        run.insert(
            c2.id,
            SmokeRunResult {
                status: 204,
                duration_ms: 50,
            },
        );

        let md = render_smoke("测试项目", std::slice::from_ref(&e), &map, true, &run);
        assert!(md.contains("运行结果：通过 2 · 失败 0 · 未测试 0"));
    }
}
