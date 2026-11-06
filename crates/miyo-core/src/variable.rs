//! 变量引擎：`{{name}}` 语法解析、内置变量、优先级合并。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::{SecondsFormat, Utc};
use uuid::Uuid;

/// 变量表。
pub type VariableMap = HashMap<String, String>;

/// 最大递归深度。
pub const MAX_VARIABLE_DEPTH: usize = 10;

const BUILTIN_UUID: &str = "$uuid";
const BUILTIN_TIMESTAMP: &str = "$timestamp";
const BUILTIN_ISO_TIMESTAMP: &str = "$isoTimestamp";
const BUILTIN_RANDOM_INT: &str = "$randomInt";
const BUILTIN_SEQ: &str = "$seq";

/// 自增计数器存储：key → 下一次输出值。key 为空字符串表示全局 `{{$seq}}`。
static SEQ_COUNTERS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
/// 脏标记：计数器被推进/设置/删除后置位；请求热路径仅变脏才回写磁盘，
///
/// 避免每次请求一次 settings 查询 + 一次写入（`{{$seq}}` 未使用时纯浪费）。
static SEQ_DIRTY: OnceLock<std::sync::atomic::AtomicBool> = OnceLock::new();

fn seq_dirty() -> &'static std::sync::atomic::AtomicBool {
    SEQ_DIRTY.get_or_init(|| std::sync::atomic::AtomicBool::new(false))
}

/// 惰性初始化计数器表（OnceLock 稳定于 1.70，不依赖较新的 `LazyLock`）。
fn seq_counters() -> &'static Mutex<HashMap<String, u64>> {
    SEQ_COUNTERS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 解析指内置变量是否可用。
#[derive(Debug, Clone, Copy)]
pub struct ResolveOptions {
    pub allow_builtin: bool,
}

impl Default for ResolveOptions {
    fn default() -> Self {
        ResolveOptions {
            allow_builtin: true,
        }
    }
}

/// 生成内置变量值；不认识的名字返回 None。
pub fn builtin_value(name: &str) -> Option<String> {
    match name {
        BUILTIN_UUID => Some(Uuid::new_v4().to_string()),
        BUILTIN_TIMESTAMP => Some(Utc::now().timestamp().to_string()),
        BUILTIN_ISO_TIMESTAMP => Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)),
        BUILTIN_RANDOM_INT => Some(rand::random_range(0..=1000).to_string()),
        _ => {
            if let Some(rest) = name.strip_prefix(BUILTIN_SEQ) {
                let key = rest.strip_prefix(':').unwrap_or("");
                return Some(seq_value(key));
            }
            None
        }
    }
}

/// 自增序号：返回当前值（即下一次输出），随后 +1。
/// `{{$seq}}` 全局计数，`{{$seq:名字}}` 各名字独立计数。未设置时从 1 开始。
fn seq_value(key: &str) -> String {
    let mut map = seq_counters().lock().unwrap_or_else(|p| p.into_inner());
    let cur = map.get(key).copied().unwrap_or(0);
    let out = if cur == 0 { 1 } else { cur };
    map.insert(key.to_string(), out + 1);
    seq_dirty().store(true, std::sync::atomic::Ordering::Relaxed);
    out.to_string()
}

/// 取走脏标记（true = 自上次取走后计数器变过，调用方据此决定是否落盘）。
pub fn take_seq_dirty() -> bool {
    seq_dirty().swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// 列出全部自增序列（key + 下一次输出值，按 key 排序；含全局 `$seq`，其 key 为空串）。
pub fn list_seq_counters() -> Vec<(String, u64)> {
    let map = seq_counters().lock().unwrap_or_else(|p| p.into_inner());
    let mut v: Vec<_> = map.iter().map(|(k, val)| (k.clone(), *val)).collect();
    v.sort_by(|a, b| a.0.cmp(&b.0));
    v
}

/// 设置自增序列的下一次输出值（key 为空 = 全局 `$seq`）。
pub fn set_seq_counter(key: &str, value: u64) {
    seq_counters()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(key.to_string(), value);
    seq_dirty().store(true, std::sync::atomic::Ordering::Relaxed);
}

/// 删除自增序列（删除后再使用从 1 重新开始）。
pub fn delete_seq_counter(key: &str) {
    seq_counters()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(key);
    seq_dirty().store(true, std::sync::atomic::Ordering::Relaxed);
}

/// 导出全部计数（用于持久化）。
pub fn dump_seq_counters() -> HashMap<String, u64> {
    seq_counters()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// 从持久化恢复计数（合并加载，启动时调用；同名以恢复值为准）。
pub fn load_seq_counters(map: HashMap<String, u64>) {
    seq_counters()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .extend(map);
}

/// 按优先级合并三张变量表：运行时 > 环境 > 项目。
pub fn merge_variables(
    runtime: &VariableMap,
    environment: &VariableMap,
    project: &VariableMap,
) -> VariableMap {
    let mut merged = HashMap::new();
    for base in [project, environment, runtime] {
        for (k, v) in base {
            merged.insert(k.clone(), v.clone());
        }
    }
    merged
}

/// 解析文本中的 `{{name}}` 变量，最大递归 MAX_VARIABLE_DEPTH 层。
pub fn resolve_variables(input: &str, vars: &VariableMap) -> String {
    resolve_variables_with(input, vars, MAX_VARIABLE_DEPTH, ResolveOptions::default())
}

/// 解析文本中的 `{{name}}` 变量，指定最大递归深度。
///
/// 单次扫描实现：一次遍历输入，定位全部 `{{...}}` token，
/// `with_capacity` 预分配后直接拼接最终结果；token 值内嵌套的
/// 变量逐层递归解析（上限 `max_depth`），不再整串反复扫描与重建。
pub fn resolve_variables_with(
    input: &str,
    vars: &VariableMap,
    max_depth: usize,
    options: ResolveOptions,
) -> String {
    let mut out = String::with_capacity(input.len() + 16);
    resolve_into(&mut out, input, vars, max_depth, options);
    out
}

/// 单次扫描：将 `s` 中所有可解析 token 替换后追加到 `out`。
///
/// - `depth` 为剩余解析层数，为 0 时整体按字面量输出；
/// - 未知变量、空 token、未闭合的 `{{` 均原样保留；
/// - 按字节扫描定位 `{` / `}`：二者均为 ASCII，不会与多字节 UTF-8 序列冲突。
fn resolve_into(
    out: &mut String,
    s: &str,
    vars: &VariableMap,
    depth: usize,
    options: ResolveOptions,
) {
    if depth == 0 {
        out.push_str(s);
        return;
    }
    let bytes = s.as_bytes();
    let mut copied = 0; // 已拷入 out 的字面量区间终点（相对 s 起点的字节偏移）
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            // 查找闭合的 }}。
            let mut close = i + 2;
            while close + 1 < bytes.len() {
                if bytes[close] == b'}' && bytes[close + 1] == b'}' {
                    break;
                }
                close += 1;
            }
            if close + 1 >= bytes.len() {
                break; // 未闭合：其余内容一律按字面量处理
            }
            out.push_str(&s[copied..i]);
            let name = s[i + 2..close].trim();
            if name.is_empty() {
                out.push_str(&s[i..close + 2]);
            } else if let Some(value) = lookup(name, vars, options) {
                // 值内可能仍有 {{...}}：递归解析，消耗一层深度。
                resolve_into(out, &value, vars, depth - 1, options);
            } else {
                out.push_str(&s[i..close + 2]);
            }
            copied = close + 2;
            i = close + 2;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[copied..]);
}

/// 查找变量值。优先级：用户变量 > 内置变量。
fn lookup(name: &str, vars: &VariableMap, options: ResolveOptions) -> Option<String> {
    if let Some(value) = vars.get(name) {
        return Some(value.clone());
    }
    if options.allow_builtin {
        // 内置变量带 $ 前缀，避免与用户变量混淆。
        if let Some(value) = builtin_value(name) {
            return Some(value);
        }
    }
    None
}

/// 收集文本中引用的全部变量名（`{{name}}`，去重、保序、剔除内置变量）。
///
/// 与 [`resolve_variables`] 同一套扫描规则（空 token / 未闭合不算引用）；
/// 场景执行前预检未定义变量、Blocked 判定（引用是否产自失败步骤）都靠它。
pub fn collect_var_refs(input: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let mut close = i + 2;
            while close + 1 < bytes.len() {
                if bytes[close] == b'}' && bytes[close + 1] == b'}' {
                    break;
                }
                close += 1;
            }
            if close + 1 >= bytes.len() {
                break; // 未闭合：与 resolve 同口径，不算引用
            }
            let name = input[i + 2..close].trim();
            if !name.is_empty() && !name.starts_with('$') && !out.iter().any(|v| v == name) {
                out.push(name.to_string());
            }
            i = close + 2;
        } else {
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> VariableMap {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// 序列状态串行锁：自增计数器是进程级全局表，并行测试会互相干扰——
    /// dump/load 的合并写回可复活刚删的 key（`seq_management_set_list_delete`
    /// 因此在 CI 偶发失败），裸 `{{$seq}}` 的计数也会被其他测试顶掉。
    /// 触碰序列状态的测试必须先持锁（中毒时取回继续，避免级联全红）。
    static SEQ_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn serial_seq_state() -> std::sync::MutexGuard<'static, ()> {
        SEQ_TEST_SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    #[test]
    fn basic_replace() {
        let v = vars(&[("base_url", "https://api.example.com"), ("id", "10")]);
        assert_eq!(
            resolve_variables("{{base_url}}/users/{{id}}", &v),
            "https://api.example.com/users/10"
        );
    }

    #[test]
    fn unknown_kept_as_is() {
        let v = VariableMap::new();
        assert_eq!(resolve_variables("{{missing}}/x", &v), "{{missing}}/x");
    }

    #[test]
    fn builtin_uuid() {
        let v = VariableMap::new();
        let out = resolve_variables("{{$uuid}}", &v);
        assert!(Uuid::parse_str(&out).is_ok());
    }

    #[test]
    fn builtin_iso_timestamp() {
        let v = VariableMap::new();
        let out = resolve_variables("{{$isoTimestamp}}", &v);
        assert!(chrono::DateTime::parse_from_rfc3339(&out).is_ok());
    }

    #[test]
    fn builtin_random_int_range() {
        let v = VariableMap::new();
        let out = resolve_variables("{{$randomInt}}", &v);
        let n: u64 = out.parse().unwrap();
        assert!((0..=1000).contains(&n));
    }

    #[test]
    fn builtin_seq_global_increments() {
        let _serial = serial_seq_state();
        let v = VariableMap::new();
        let a: u64 = resolve_variables("{{$seq}}", &v).parse().unwrap();
        let b: u64 = resolve_variables("{{$seq}}", &v).parse().unwrap();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn builtin_seq_named_is_independent() {
        let _serial = serial_seq_state();
        let v = VariableMap::new();
        let key = format!("t{}", Uuid::new_v4().simple());
        let a: u64 = resolve_variables(&format!("{{{{$seq:{key}}}}}"), &v)
            .parse()
            .unwrap();
        let b: u64 = resolve_variables(&format!("{{{{$seq:{key}}}}}"), &v)
            .parse()
            .unwrap();
        assert_eq!(b, a + 1);
    }

    #[test]
    fn builtin_seq_value_is_next_output() {
        let _serial = serial_seq_state();
        let key = format!("t{}", Uuid::new_v4().simple());
        set_seq_counter(&key, 100);
        let v = VariableMap::new();
        let first: u64 = resolve_variables(&format!("{{{{$seq:{key}}}}}"), &v)
            .parse()
            .unwrap();
        let second: u64 = resolve_variables(&format!("{{{{$seq:{key}}}}}"), &v)
            .parse()
            .unwrap();
        // 设置 value=100 → 下一次输出 100，随后 101。
        assert_eq!(first, 100);
        assert_eq!(second, 101);
        delete_seq_counter(&key);
    }

    #[test]
    fn seq_management_set_list_delete() {
        let _serial = serial_seq_state();
        let key = format!("t{}", Uuid::new_v4().simple());
        set_seq_counter(&key, 42);
        let listed = list_seq_counters();
        let found = listed.iter().find(|(k, _)| *k == key).expect("应在列表中");
        assert_eq!(found.1, 42);
        delete_seq_counter(&key);
        assert!(
            !list_seq_counters().iter().any(|(k, _)| *k == key),
            "删除后不应再出现"
        );
    }

    #[test]
    fn seq_management_dump_load_roundtrip() {
        let _serial = serial_seq_state();
        let mut map = dump_seq_counters();
        map.insert("roundtrip".to_string(), 7);
        load_seq_counters(map.clone());
        let after = dump_seq_counters();
        assert_eq!(after.get("roundtrip"), Some(&7));
    }

    #[test]
    fn builtin_seq_works_with_literal_prefix() {
        let _serial = serial_seq_state();
        let v = VariableMap::new();
        let out = resolve_variables("aaaa{{$seq}}", &v);
        assert!(out.starts_with("aaaa"));
        let n: u64 = out[4..].parse().unwrap();
        assert!(n >= 1);
    }

    #[test]
    fn user_var_overrides_builtin() {
        let v = vars(&[("$uuid", "custom")]);
        assert_eq!(resolve_variables("{{$uuid}}", &v), "custom");
    }

    #[test]
    fn nested_variables_recursion() {
        let v = vars(&[("a", "{{b}}"), ("b", "{{c}}"), ("c", "end")]);
        assert_eq!(resolve_variables("{{a}}", &v), "end");
    }

    #[test]
    fn nested_recursion_depth_capped() {
        let v = vars(&[("v0", "{{v1}}")]);
        let mut map = v;
        for i in 1..20 {
            map.insert(format!("v{i}"), format!("{{{{v{}}}}}", i + 1));
        }
        map.insert("v20".to_string(), "stop".to_string());
        // 超过 10 层后不再递归，保持未解析形态。
        let out = resolve_variables("{{v0}}", &map);
        assert!(out.len() > 2);
        assert!(out.contains("{{"));
    }

    #[test]
    fn merge_variables_priority() {
        let project = vars(&[("a", "project"), ("b", "project")]);
        let env = vars(&[("b", "env"), ("c", "env")]);
        let runtime = vars(&[("c", "runtime"), ("d", "runtime")]);
        let merged = merge_variables(&runtime, &env, &project);
        assert_eq!(merged["a"], "project");
        assert_eq!(merged["b"], "env");
        assert_eq!(merged["c"], "runtime");
        assert_eq!(merged["d"], "runtime");
    }

    #[test]
    fn empty_token_left_alone() {
        let v = VariableMap::new();
        assert_eq!(resolve_variables("a{{}}b", &v), "a{{}}b");
    }

    #[test]
    fn many_tokens_resolved_in_one_pass() {
        let v = vars(&[("id", "42")]);
        let input = "{{id}},".repeat(1000);
        let out = resolve_variables(&input, &v);
        assert_eq!(out, "42,".repeat(1000));
        assert!(!out.contains("{{"));
    }

    #[test]
    fn unicode_literals_preserved() {
        let v = vars(&[("name", "小狐狸"), ("id", "9")]);
        assert_eq!(
            resolve_variables("你好，{{name}}！#{{id}}号", &v),
            "你好，小狐狸！#9号"
        );
    }

    #[test]
    fn unclosed_token_kept_as_literal() {
        let v = vars(&[("a", "1")]);
        assert_eq!(
            resolve_variables("{{a}} and {{unclosed", &v),
            "1 and {{unclosed"
        );
    }

    #[test]
    fn collect_refs_dedup_ordered_no_builtin() {
        let refs = collect_var_refs("{{base_url}}/u/{{id}}?t={{id}}&x={{$uuid}}&{{}}&{{open");
        assert_eq!(refs, vec!["base_url".to_string(), "id".to_string()]);
    }

    #[test]
    fn collect_refs_empty_input() {
        assert!(collect_var_refs("").is_empty());
        assert!(collect_var_refs("no vars here").is_empty());
    }
}
