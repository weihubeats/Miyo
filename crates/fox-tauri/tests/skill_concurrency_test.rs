//! Skill 命令并发契约：**命令改为 `async` 之后**，写操作仍必须串行。
//!
//! 背景：为消除「首次进 skill 页面卡顿」，9 个 skill 命令从同步 `pub fn` 改为
//! `async fn`（同步命令跑在主线程上，见 `commands/skill.rs` 模块注释）。
//! `async` 命令由线程池执行，因此**并发调用从此成为可能**——原先靠「主线程单线程
//! 执行」顺带获得的那层串行保护没有了，安全性现在完全依赖 `WRITE_LOCK`。
//!
//! 本测试锁住两件事，避免这层保护被无意破掉：
//! 1. `with_write_lock` 在**持有锁期间不能跨 `.await`**（跨了就会死锁线程池 /
//!    并在并发下失去互斥）；
//! 2. 那 4 个写命令确实都包在 `with_write_lock` 里（不能有人漏掉）。

use std::sync::Mutex;

const COMMANDS_RS: &str = include_str!("../src/commands/skill.rs");

/// 写命令名（会改磁盘的）。
const WRITE_COMMANDS: &[&str] = &[
    "skill_create",
    "skill_save",
    "skill_delete",
    "skill_set_enabled",
];

/// 取一个函数体（从 `fn name` 到下一个顶层 `}`），用于检查其内部写法。
fn function_body(source: &str, name: &str) -> String {
    let marker = format!("fn {name}");
    let start = source
        .find(&marker)
        .unwrap_or_else(|| panic!("未找到函数 {name}"));
    // 从签名后的第一个 `{` 起做花括号配平，取完整函数体。
    let after = &source[start..];
    let open = after.find('{').expect("函数体未开始");
    let bytes = after.as_bytes();
    let mut depth = 0usize;
    for (i, ch) in bytes.iter().enumerate().skip(open) {
        match ch {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return after[..=i].to_string();
                }
            }
            _ => {}
        }
    }
    panic!("函数 {name} 花括号未配平");
}

/// 提取 `fn with_write_lock` 的函数体。
fn write_lock_body() -> String {
    function_body(COMMANDS_RS, "with_write_lock")
}

#[test]
fn write_lock_is_held_synchronously_without_await() {
    let body = write_lock_body();
    assert!(
        body.contains("WRITE_LOCK"),
        "with_write_lock 必须真的去抢 WRITE_LOCK"
    );
    assert!(
        body.contains(".lock()"),
        "with_write_lock 必须真的去抢 WRITE_LOCK"
    );
    assert!(
        !body.contains(".await"),
        "with_write_lock 持有锁期间不得跨 .await：\n  · async 命令在线程池执行，跨 await 持锁会在并发写时长时间阻塞线程\n  \
         · 正确做法是「锁内跑同步闭包、跑完即释放」（当前实现即如此）\n函数体：\n{body}"
    );
    // 闭包必须是同步的：出现 `async` 关键字或返回 Future 都意味着锁会被跨 await 持有。
    assert!(
        !body.contains("async"),
        "with_write_lock 的闭包必须是同步 FnOnce（不能在 async 闭包里持锁）：\n{body}"
    );
}

#[test]
fn every_write_command_goes_through_the_lock() {
    for cmd in WRITE_COMMANDS {
        let body = function_body(COMMANDS_RS, cmd);
        assert!(
            body.contains("with_write_lock"),
            "写命令 {cmd} 必须包在 with_write_lock 里，否则 async 化后并发写会互相踩踏。\n函数体：\n{body}"
        );
    }
}

/// 反向对照：确认 `with_write_lock` 之外**没有**别的路径在改磁盘。
///
/// 只允许 4 个写命令调用 `with_write_lock`；若新增写命令忘了用它，这里会漏，
/// 所以额外断言「出现了 with_write_lock 的命令集合」与预期完全一致。
#[test]
fn lock_users_match_expected_write_commands() {
    let users: Vec<&str> = COMMANDS_RS
        .match_indices("with_write_lock(")
        .map(|(at, _)| at)
        .filter_map(|at| {
            // 向前找最近的 `pub async fn <name>`
            let head = &COMMANDS_RS[..at];
            let name_at = head.rfind("pub async fn ")?;
            Some(
                head[name_at + "pub async fn ".len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                    .collect::<String>(),
            )
        })
        .map(|name| Box::leak(name.into_boxed_str()) as &str)
        .collect();

    let mut sorted = users.clone();
    sorted.sort_unstable();
    let mut expected: Vec<&str> = WRITE_COMMANDS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        sorted, expected,
        "使用 with_write_lock 的命令集合与预期写命令不一致（新增/删除了写命令？请同步更新本测试）"
    );
}

/// 4 个写命令必须都是 `async`（若非 async 会在主线程跑，回到卡顿问题）。
#[test]
fn write_commands_are_async() {
    for cmd in WRITE_COMMANDS {
        assert!(
            COMMANDS_RS.contains(&format!("pub async fn {cmd}")),
            "写命令 {cmd} 必须是 async fn（同步命令在主线程执行，会阻塞 UI）"
        );
    }
}

/// 用一个真实互斥量验证「同步闭包 + 锁」确实提供互斥语义：
/// 多线程并发调用时必须严格串行（不重叠）。
#[test]
fn sync_closure_lock_serializes_concurrent_callers() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    static LOCK: Mutex<()> = Mutex::new(());
    let inside = Arc::new(AtomicUsize::new(0));
    let max_seen = Arc::new(AtomicUsize::new(0));

    let handles: Vec<_> = (0..8)
        .map(|_| {
            let inside = Arc::clone(&inside);
            let max_seen = Arc::clone(&max_seen);
            std::thread::spawn(move || {
                // 复刻 with_write_lock：同步抢锁 → 同步跑完 → 释放
                let _guard = LOCK.lock().unwrap();
                let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                max_seen.fetch_max(now, Ordering::SeqCst);
                std::thread::yield_now();
                inside.fetch_sub(1, Ordering::SeqCst);
            })
        })
        .collect();

    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(
        max_seen.load(Ordering::SeqCst),
        1,
        "同一个锁保护的临界区不得并发进入"
    );
}
