//! JS 脚本沙箱（SPEC §14.4）：执行用户 Pre-request / Post-response 脚本。
//!
//! 基于 rquickjs（QuickJS）实现，注入 Postman 风格 API（`pm.*`、`console.log`），
//! 支持 `async/await`（含顶层 await）、2 秒超时与运行时错误捕获。
//! 脚本在独立工作线程中执行，不会阻塞异步主线程。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::{Duration, Instant};

use miyo_core::model::{BodySpec, HttpMethod, RequestSpec};
use miyo_core::{script_error, AppError};
use rquickjs::Error as JsError;
use rquickjs::{Context, Ctx, Function, Promise, Runtime, Value};

use crate::client::HttpResponseData;

/// 单个脚本执行的超时时间。
pub const SCRIPT_TIMEOUT_MS: u64 = 2000;
/// 脚本超时中文提示。
const SCRIPT_TIMEOUT_MESSAGE: &str = "脚本执行超时（超过 2 秒）";
/// 调用侧等待的额外余量，避免排队导致的误报。
const CALLER_SLACK_MS: u64 = 1000;
/// 脚本空转时的轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_millis(1);
/// 单个沙箱内存上限（64 MiB），防止脚本无限分配内存。
const SCRIPT_MEMORY_LIMIT_BYTES: usize = 64 * 1024 * 1024;

/// 请求快照：脚本可读取的请求信息。
#[derive(Debug, Clone, Default)]
pub struct ScriptRequestData {
    pub method: String,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl ScriptRequestData {
    /// 从请求规格构建快照（method 需单独传入，规格本身不含方法）。
    pub fn from_spec(method: HttpMethod, url: &str, spec: &RequestSpec) -> Self {
        ScriptRequestData {
            method: method.as_str().to_string(),
            url: url.to_string(),
            headers: spec
                .headers
                .iter()
                .filter(|kv| kv.enabled)
                .map(|kv| (kv.key.trim().to_string(), kv.value.clone()))
                .collect(),
            body: body_text(spec),
        }
    }
}

/// 响应快照：Post-response 脚本可读取的响应信息。
#[derive(Debug, Clone)]
pub struct ScriptResponseData {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl ScriptResponseData {
    pub fn from_response(resp: &HttpResponseData) -> Self {
        ScriptResponseData {
            status: resp.status,
            headers: resp.headers.clone(),
            body: resp.body_text(),
        }
    }
}

/// 脚本执行输入：`response` 为 `Some` 时按 Post-response 执行，否则按 Pre-request。
#[derive(Debug, Clone)]
pub struct ScriptInput {
    pub code: String,
    pub environment: HashMap<String, String>,
    pub request: ScriptRequestData,
    pub response: Option<ScriptResponseData>,
}

/// 单个 `pm.test` 断言结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestResult {
    pub name: String,
    pub passed: bool,
    pub error: Option<String>,
}

/// 脚本执行结果：环境与请求头的变更会回传给调用方。
#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub environment: HashMap<String, String>,
    pub headers: Vec<(String, String)>,
    pub tests: Vec<TestResult>,
    pub logs: Vec<String>,
}

/// 脚本沙箱：持有专用工作线程，串行执行脚本，互不阻塞主线程。
#[derive(Debug)]
pub struct ScriptSandbox {
    tx: Option<Sender<Job>>,
    join: Option<std::thread::JoinHandle<()>>,
}

struct Job {
    input: ScriptInput,
    reply: tokio::sync::oneshot::Sender<Result<ScriptResult, AppError>>,
}

impl ScriptSandbox {
    /// 启动沙箱工作线程（阻塞等待引擎初始化完成）。
    /// 工作线程内只创建一次 QuickJS Runtime + Context，之后所有脚本复用。
    pub fn new() -> Result<Self, AppError> {
        let (tx, rx) = channel::<Job>();
        let (init_tx, init_rx) = channel::<Result<(), AppError>>();
        // native 栈 8MB（默认 2MB）：QuickJS 有 JS 层栈检查，但深递归可能在
        // 检查生效前冲爆 native 栈直接 abort 整个进程；加大线程栈给检查留余量。
        let join = std::thread::Builder::new()
            .name("miyo-script-sandbox".to_string())
            .stack_size(8 * 1024 * 1024)
            .spawn(move || sandbox_worker(rx, init_tx))
            .map_err(AppError::Io)?;
        match init_rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                let _ = join.join();
                return Err(e);
            }
            Err(_) => {
                let _ = join.join();
                return Err(script_error("脚本沙箱启动失败"));
            }
        }
        Ok(ScriptSandbox {
            tx: Some(tx),
            join: Some(join),
        })
    }

    /// 异步执行脚本，等待执行结果或超时。
    pub async fn run(&self, input: ScriptInput) -> Result<ScriptResult, AppError> {
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| script_error("脚本沙箱已停止"))?;
        let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
        tx.send(Job {
            input,
            reply: reply_tx,
        })
        .map_err(|_| script_error("脚本沙箱已停止"))?;
        tokio::time::timeout(
            Duration::from_millis(SCRIPT_TIMEOUT_MS + CALLER_SLACK_MS),
            reply_rx,
        )
        .await
        .map_err(|_| script_error(SCRIPT_TIMEOUT_MESSAGE))?
        .map_err(|_| script_error("脚本沙箱已停止"))?
    }
}

impl Drop for ScriptSandbox {
    fn drop(&mut self) {
        drop(self.tx.take());
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
    }
}

/// 工作线程主循环：串行处理脚本任务，任一脚本最多占用 2 秒。
/// Runtime / Context 只在此线程内初始化一次，供所有脚本复用。
fn sandbox_worker(rx: Receiver<Job>, init_tx: Sender<Result<(), AppError>>) {
    let mut runner = match SandboxRunner::init() {
        Ok(runner) => runner,
        Err(e) => {
            let _ = init_tx.send(Err(e));
            return;
        }
    };
    if init_tx.send(Ok(())).is_err() {
        return;
    }
    while let Ok(job) = rx.recv() {
        let result = runner.run_script(&job.input);
        let _ = job.reply.send(result);
    }
}

/// 复用型 QuickJS 执行单元：工作线程内初始化一次，脚本间通过清理全局状态隔离。
struct SandboxRunner {
    /// 注意字段顺序：Context 先于 Runtime 声明，析构时先释放 Context 再释放 Runtime。
    context: Context,
    runtime: Runtime,
    /// 初始化时的全局可枚举键快照：每次执行前删除运行期间新增的全局变量。
    pristine_globals: HashSet<String>,
}

impl SandboxRunner {
    /// 初始化引擎（Runtime + Context），记录全局快照。
    /// PRELUDE 依赖每次运行才注册的 `__*` 桥接函数，因此按次注入（见 [`execute`]）。
    fn init() -> Result<Self, AppError> {
        let runtime = Runtime::new().map_err(init_error)?;
        runtime.set_memory_limit(SCRIPT_MEMORY_LIMIT_BYTES);
        // JS 层栈上限 1MB：深递归（如 `function f(){f()} f()`）触发 QuickJS
        // 栈溢出异常而非冲爆 native 栈（进程 abort，2s 中断器救不了 abort）。
        runtime.set_max_stack_size(1024 * 1024);
        let context = Context::full(&runtime).map_err(init_error)?;
        let pristine_globals = context.with(|ctx| collect_global_keys(&ctx).map_err(init_error))?;
        Ok(SandboxRunner {
            context,
            runtime,
            pristine_globals,
        })
    }

    /// 执行单个脚本：复用引擎实例，每次执行前重置中断器 / 内存上限 / 全局状态。
    fn run_script(&mut self, input: &ScriptInput) -> Result<ScriptResult, AppError> {
        let deadline = Instant::now() + Duration::from_millis(SCRIPT_TIMEOUT_MS);

        // 中断处理器携带本次 deadline：QuickJS 在字节码执行期间周期性检查，兜底死循环。
        // 必须每次重置，否则上次的 deadline 过期后会误中断后续脚本。
        self.runtime
            .set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
        // 重置内存上限：解除上次脚本触顶后的锁定，恢复 64 MiB 限制。
        self.runtime.set_memory_limit(SCRIPT_MEMORY_LIMIT_BYTES);

        let script = build_script(&input.code);
        let result = self.context.with(|ctx| {
            reset_globals(&ctx, &self.pristine_globals)?;
            execute(ctx, input, &script, deadline)
        });

        // 脚本结束后主动 GC：回收循环引用等残留，为下一次执行释放内存。
        self.runtime.run_gc();
        result
    }
}

/// 收集全局对象的全部可枚举键（含 PRELUDE 与引擎内置项）。
fn collect_global_keys(ctx: &Ctx<'_>) -> Result<HashSet<String>, JsError> {
    ctx.globals()
        .keys::<String>()
        .collect::<Result<HashSet<_>, _>>()
}

/// 重置全局状态，恢复脚本间隔离：删除上次运行新增的全局变量
/// （`globalThis.*`、`__` 桥接函数、包装器变量等）。
/// PRELUDE 声明（`pm` / `console` / 工具函数）由每次执行重新求值覆盖，
/// 状态天然全新；且忽略删除失败（QuickJS 中 eval 出的函数声明不可删除）。
fn reset_globals(ctx: &Ctx<'_>, pristine: &HashSet<String>) -> Result<(), AppError> {
    let globals = ctx.globals();
    let keys: Vec<String> = globals
        .keys::<String>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(init_error)?;
    for key in keys {
        if !pristine.contains(&key) {
            // 不可配置属性（PRELUDE 函数声明等）删除会抛异常，跳过即可。
            let _ = globals.remove(key);
        }
    }
    Ok(())
}

/// 上下文初始化：注入状态、注册 Rust 桥接函数、执行并驱动脚本。
fn execute(
    ctx: Ctx<'_>,
    input: &ScriptInput,
    script: &str,
    deadline: Instant,
) -> Result<ScriptResult, AppError> {
    let state = Rc::new(RefCell::new(ScriptState::new(input)));
    register_bindings(&ctx, &state).map_err(init_error)?;

    let promise = ctx
        .eval_promise(script)
        .map_err(|e| classify_eval_error(&ctx, &e, deadline))?;
    drive_promise(&promise, deadline)?;

    let state = state.borrow();
    if let Some(error) = &state.error {
        return Err(script_error(error.clone()));
    }
    Ok(ScriptResult {
        environment: state.environment.clone(),
        headers: state.headers.clone(),
        tests: state.tests.clone(),
        logs: state.logs.clone(),
    })
}

/// 驱动 Promise 直到完成或超时：逐个推进微任务队列，队列为空时短暂休眠再试。
fn drive_promise(promise: &Promise<'_>, deadline: Instant) -> Result<(), AppError> {
    loop {
        if Instant::now() >= deadline {
            return Err(script_error(SCRIPT_TIMEOUT_MESSAGE));
        }
        match promise.finish::<Value>() {
            Ok(_) => return Ok(()),
            Err(JsError::WouldBlock) => std::thread::sleep(POLL_INTERVAL),
            Err(e) => {
                if Instant::now() >= deadline {
                    return Err(script_error(SCRIPT_TIMEOUT_MESSAGE));
                }
                return Err(script_error(format!(
                    "脚本执行失败：{}",
                    describe_js_error(promise.ctx(), &e)
                )));
            }
        }
    }
}

/// 将 JS 运行时异常翻译为 `AppError::ScriptError`（语法错误 / 运行时错误 / 超时）。
fn classify_eval_error(ctx: &Ctx<'_>, e: &JsError, deadline: Instant) -> AppError {
    if Instant::now() >= deadline {
        script_error(SCRIPT_TIMEOUT_MESSAGE)
    } else {
        script_error(format!("脚本执行失败：{}", describe_js_error(ctx, e)))
    }
}

/// 提取 JS 异常的可读文本（语法错误 / 运行时异常的 message）。
fn describe_js_error(ctx: &Ctx<'_>, e: &JsError) -> String {
    if e.is_exception() {
        let v = ctx.catch();
        v.as_object()
            .and_then(|obj| obj.get::<&str, String>("message").ok())
            .or_else(|| v.get::<String>().ok())
            .unwrap_or_else(|| e.to_string())
    } else {
        e.to_string()
    }
}

/// 初始化阶段的引擎错误（一般不会发生，防御性映射）。
fn init_error(e: JsError) -> AppError {
    script_error(format!("脚本环境初始化失败：{e}"))
}

/// 拼接预置 API 与用户代码：用户代码包裹在 async 函数中，支持顶层 await，
/// 运行时异常被捕获后经 `__sx_error` 回传；脚本末尾显式 await 包装器，
/// 保证返回的 Promise 在用户代码（含异步测试）全部完成后才 settle。
fn build_script(user_code: &str) -> String {
    format!(
        "{PRELUDE}\nvar __sandbox_promise = (async function () {{\n  try {{\n{user_code}\n    await Promise.all(pm.__pendingTests);\n  }} catch (e) {{\n    __sx_error(String(e && (e.message || e.stack) || e));\n  }}\n}})();\nawait __sandbox_promise;\n"
    )
}

/// 请求 body 的文本表示（与 client 的 payload 构建保持一致）。
fn body_text(spec: &RequestSpec) -> String {
    match &spec.body {
        BodySpec::Json { raw } | BodySpec::Text { raw } => raw.clone(),
        BodySpec::UrlEncoded { fields } => {
            let pairs: Vec<(String, String)> = fields
                .iter()
                .filter(|kv| kv.enabled)
                .map(|kv| (kv.key.clone(), kv.value.clone()))
                .collect();
            serde_urlencoded::to_string(pairs).unwrap_or_default()
        }
        BodySpec::Multipart { .. } | BodySpec::None | BodySpec::Binary { .. } => String::new(),
        // gRPC 不走 HTTP 脚本沙箱；兜底空串（与 client payload 口径一致）
        BodySpec::Grpc { .. } => String::new(),
        BodySpec::GraphQL { spec } => {
            crate::client::graphql_request_json(spec, &std::collections::HashMap::new())
                .unwrap_or_default()
        }
    }
}

/// 脚本内部状态，由 Rust 桥接闭包读写。
struct ScriptState {
    environment: HashMap<String, String>,
    headers: Vec<(String, String)>,
    request: ScriptRequestData,
    response: Option<ScriptResponseData>,
    tests: Vec<TestResult>,
    logs: Vec<String>,
    error: Option<String>,
}

impl ScriptState {
    fn new(input: &ScriptInput) -> Self {
        ScriptState {
            environment: input.environment.clone(),
            headers: input.request.headers.clone(),
            request: input.request.clone(),
            response: input.response.clone(),
            tests: Vec::new(),
            logs: Vec::new(),
            error: None,
        }
    }

    fn header_upsert(&mut self, key: &str, value: &str) {
        if let Some(pair) = self
            .headers
            .iter_mut()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
        {
            pair.1 = value.to_string();
        } else {
            self.headers.push((key.to_string(), value.to_string()));
        }
    }

    fn header_get(&self, key: &str) -> Option<String> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v.clone())
    }

    fn header_remove(&mut self, key: &str) {
        self.headers.retain(|(k, _)| !k.eq_ignore_ascii_case(key));
    }

    fn header_has(&self, key: &str) -> bool {
        self.headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(key))
    }
}

/// 向全局对象注册 `__*` 桥接函数（JS 预置 API 的唯一后端）。
fn register_bindings(ctx: &Ctx<'_>, state: &Rc<RefCell<ScriptState>>) -> Result<(), JsError> {
    let globals = ctx.globals();

    let s = Rc::clone(state);
    globals.set(
        "__log",
        Function::new(ctx.clone(), move |level: String, text: String| {
            s.borrow_mut().logs.push(format!("[{level}] {text}"));
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__test",
        Function::new(
            ctx.clone(),
            move |name: String, passed: bool, error: String| {
                let error = (!error.is_empty()).then_some(error);
                s.borrow_mut().tests.push(TestResult {
                    name,
                    passed,
                    error,
                });
            },
        )?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__sx_error",
        Function::new(ctx.clone(), move |msg: String| {
            s.borrow_mut().error = Some(msg);
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__env_get",
        Function::new(ctx.clone(), move |key: String| -> Option<String> {
            s.borrow().environment.get(&key).cloned()
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__env_set",
        Function::new(ctx.clone(), move |key: String, value: String| {
            s.borrow_mut().environment.insert(key, value);
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__env_unset",
        Function::new(ctx.clone(), move |key: String| {
            s.borrow_mut().environment.remove(&key);
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__env_has",
        Function::new(ctx.clone(), move |key: String| -> bool {
            s.borrow().environment.contains_key(&key)
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__env_all",
        Function::new(ctx.clone(), move || -> HashMap<String, String> {
            s.borrow().environment.clone()
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_method",
        Function::new(ctx.clone(), move || s.borrow().request.method.clone())?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_url",
        Function::new(ctx.clone(), move || s.borrow().request.url.clone())?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_body",
        Function::new(ctx.clone(), move || s.borrow().request.body.clone())?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_header_upsert",
        Function::new(ctx.clone(), move |key: String, value: String| {
            s.borrow_mut().header_upsert(&key, &value);
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_header_get",
        Function::new(ctx.clone(), move |key: String| -> Option<String> {
            s.borrow().header_get(&key)
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_header_remove",
        Function::new(ctx.clone(), move |key: String| {
            s.borrow_mut().header_remove(&key);
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_header_has",
        Function::new(ctx.clone(), move |key: String| -> bool {
            s.borrow().header_has(&key)
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__req_headers_all",
        Function::new(ctx.clone(), move || -> HashMap<String, String> {
            s.borrow()
                .request
                .headers
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__resp_text",
        Function::new(ctx.clone(), move || {
            s.borrow()
                .response
                .as_ref()
                .map(|r| r.body.clone())
                .unwrap_or_default()
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__resp_code",
        Function::new(ctx.clone(), move || {
            s.borrow()
                .response
                .as_ref()
                .map_or(0, |r| i32::from(r.status))
        })?,
    )?;

    let s = Rc::clone(state);
    globals.set(
        "__resp_headers_all",
        Function::new(ctx.clone(), move || -> HashMap<String, String> {
            s.borrow()
                .response
                .as_ref()
                .map(|r| {
                    r.headers
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default()
        })?,
    )?;

    Ok(())
}

/// JS 预置 API：Postman 风格 `pm.*` 与 `console.*`，全部经 `__*` 桥接函数访问 Rust 状态。
const PRELUDE: &str = r#"
function __sx_stringify(v) {
    if (v === null) return 'null';
    if (v === undefined) return 'undefined';
    if (typeof v === 'string') return v;
    if (typeof v === 'function') return '[Function]';
    try { return JSON.stringify(v); } catch (e) { return String(v); }
}

function __sx_deep_equal(a, b) {
    if (a === b) return true;
    if (typeof a !== 'object' || typeof b !== 'object' || a === null || b === null) return false;
    var aArr = a instanceof Array, bArr = b instanceof Array;
    if (aArr !== bArr) return false;
    if (aArr) {
        if (a.length !== b.length) return false;
        for (var i = 0; i < a.length; i++) {
            if (!__sx_deep_equal(a[i], b[i])) return false;
        }
        return true;
    }
    var ka = Object.keys(a), kb = Object.keys(b);
    if (ka.length !== kb.length) return false;
    for (var i = 0; i < ka.length; i++) {
        var k = ka[i];
        if (!Object.prototype.hasOwnProperty.call(b, k)) return false;
        if (!__sx_deep_equal(a[k], b[k])) return false;
    }
    return true;
}

function __sx_fail(expected, actual) {
    throw new Error('期望值为 ' + __sx_stringify(expected) + '，实际为 ' + __sx_stringify(actual));
}

function __sx_expect(actual) {
    var chain = {
        to: {
            eql: function (expected) {
                if (!__sx_deep_equal(actual, expected)) __sx_fail(expected, actual);
            },
            equal: function (expected) {
                if (actual !== expected) __sx_fail(expected, actual);
            },
            include: function (expected) {
                if (actual === undefined || actual === null ||
                    typeof actual.indexOf !== 'function' || actual.indexOf(expected) < 0) {
                    __sx_fail(expected, actual);
                }
            },
            have: {
                property: function (name) {
                    if (actual === null || typeof actual !== 'object' || !(name in actual)) {
                        __sx_fail('属性 ' + name, actual);
                    }
                }
            },
            be: {
                true: function () { if (actual !== true) __sx_fail(true, actual); },
                false: function () { if (actual !== false) __sx_fail(false, actual); },
                null: function () { if (actual !== null) __sx_fail(null, actual); },
                undefined: function () { if (actual !== undefined) __sx_fail(undefined, actual); }
            }
        }
    };
    return chain;
}

var console = {
    log: function () {
        var args = Array.prototype.slice.call(arguments);
        args.unshift('log');
        __sx_log_impl.apply(null, args);
    },
    info: function () {
        var args = Array.prototype.slice.call(arguments);
        args.unshift('info');
        __sx_log_impl.apply(null, args);
    },
    warn: function () {
        var args = Array.prototype.slice.call(arguments);
        args.unshift('warn');
        __sx_log_impl.apply(null, args);
    },
    error: function () {
        var args = Array.prototype.slice.call(arguments);
        args.unshift('error');
        __sx_log_impl.apply(null, args);
    }
};

function __sx_log_impl(level) {
    var parts = [];
    for (var i = 1; i < arguments.length; i++) { parts.push(__sx_stringify(arguments[i])); }
    __log(level, parts.join(' '));
}

var pm = {
    environment: {
        get: function (key) { return __env_get(key); },
        set: function (key, value) { __env_set(key, __sx_stringify(value)); },
        unset: function (key) { __env_unset(key); },
        has: function (key) { return __env_has(key); },
        all: function () { return __env_all(); }
    },
    request: {
        method: __req_method(),
        url: __req_url(),
        body: __req_body(),
        headers: {
            add: function (header) {
                if (!header || header.key === undefined) {
                    throw new Error('pm.request.headers.add 需要传入 { key, value } 对象');
                }
                __req_header_upsert(String(header.key), __sx_stringify(header.value));
            },
            upsert: function (key, value) { __req_header_upsert(String(key), __sx_stringify(value)); },
            get: function (key) { return __req_header_get(String(key)); },
            remove: function (key) { __req_header_remove(String(key)); },
            has: function (key) { return __req_header_has(String(key)); },
            all: function () { return __req_headers_all(); }
        }
    },
    response: {
        json: function () {
            var text = __resp_text();
            if (text === '') { throw new Error('响应体为空，无法解析 JSON'); }
            try { return JSON.parse(text); } catch (e) { throw new Error('响应体不是合法 JSON'); }
        },
        text: function () { return __resp_text(); },
        code: __resp_code(),
        headers: __resp_headers_all()
    },
    test: function (name, fn) {
        try {
            var result = fn();
            if (result && typeof result.then === 'function') {
                pm.__pendingTests.push(result);
                result.then(
                    function () { __test(name, true, ''); },
                    function (err) { __test(name, false, __sx_stringify(err && err.message || err)); }
                );
            } else {
                __test(name, true, '');
            }
        } catch (err) {
            __test(name, false, __sx_stringify(err && err.message || err));
        }
    },
    expect: __sx_expect,
    __pendingTests: []
};
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn base_input(code: &str) -> ScriptInput {
        ScriptInput {
            code: code.to_string(),
            environment: HashMap::from([("base_url".to_string(), "http://localhost".to_string())]),
            request: ScriptRequestData {
                method: "GET".to_string(),
                url: "http://localhost/api/users".to_string(),
                headers: vec![("Content-Type".to_string(), "application/json".to_string())],
                body: String::new(),
            },
            response: None,
        }
    }

    fn post_input(code: &str) -> ScriptInput {
        let mut input = base_input(code);
        input.response = Some(ScriptResponseData {
            status: 200,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: r#"{"code":0,"data":{"name":"miyo","age":2}}"#.to_string(),
        });
        input
    }

    #[tokio::test]
    async fn pre_request_script_mutates_environment_and_headers() {
        let sandbox = ScriptSandbox::new().unwrap();
        let input = base_input(
            r#"
            pm.environment.set('token', 'abc123');
            pm.request.headers.add({ key: 'X-Token', value: 'abc123' });
            pm.request.headers.add({ key: 'Content-Type', value: 'text/plain' });
            console.log('token =', pm.environment.get('token'));
            "#,
        );
        let result = sandbox.run(input).await.unwrap();
        assert_eq!(
            result.environment.get("token").map(String::as_str),
            Some("abc123")
        );
        assert_eq!(
            result.environment.get("base_url").map(String::as_str),
            Some("http://localhost")
        );
        assert!(result
            .headers
            .iter()
            .any(|(k, v)| k == "X-Token" && v == "abc123"));
        // 大小写不敏感 upsert：覆盖已有 Content-Type。
        assert_eq!(
            result
                .headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("Content-Type"))
                .count(),
            1
        );
        assert!(result
            .headers
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("Content-Type") && v == "text/plain"));
        assert!(result.logs.iter().any(|l| l.contains("token = abc123")));
        assert!(result.tests.is_empty());
    }

    #[tokio::test]
    async fn post_response_script_runs_pm_test_assertions() {
        let sandbox = ScriptSandbox::new().unwrap();
        let input = post_input(
            r#"
            const body = pm.response.json();
            pm.test('状态码为 200', () => pm.expect(pm.response.code).to.eql(200));
            pm.test('返回字段 name', () => pm.expect(body.data.name).to.equal('miyo'));
            pm.test('age 为数字类型', () => pm.expect(typeof body.data.age).to.eql('number'));
            pm.test('断言失败示例', () => pm.expect(body.data.name).to.equal('cat'));
            "#,
        );
        let result = sandbox.run(input).await.unwrap();
        assert_eq!(result.tests.len(), 4);
        let passed: Vec<&str> = result
            .tests
            .iter()
            .filter(|t| t.passed)
            .map(|t| t.name.as_str())
            .collect();
        assert_eq!(passed.len(), 3, "通过列表：{passed:?}");
        let failed = result.tests.iter().find(|t| !t.passed).unwrap();
        assert_eq!(failed.name, "断言失败示例");
        assert!(
            failed.error.as_deref().unwrap().contains("cat"),
            "错误信息：{:?}",
            failed.error
        );
    }

    #[tokio::test]
    async fn async_script_with_top_level_await_is_supported() {
        let sandbox = ScriptSandbox::new().unwrap();
        let input = base_input(
            r#"
            pm.environment.set('before', '1');
            await Promise.resolve(42);
            pm.environment.set('after', '2');
            pm.test('异步断言', async () => {
                await Promise.resolve(1);
                pm.expect(pm.environment.get('after')).to.equal('2');
            });
            "#,
        );
        let result = sandbox.run(input).await.unwrap();
        assert_eq!(
            result.environment.get("before").map(String::as_str),
            Some("1")
        );
        assert_eq!(
            result.environment.get("after").map(String::as_str),
            Some("2")
        );
        assert_eq!(result.tests.len(), 1);
        assert!(result.tests[0].passed, "异步断言应通过：{:?}", result.tests);
    }

    #[tokio::test]
    async fn console_and_pm_environment_helpers_work() {
        let sandbox = ScriptSandbox::new().unwrap();
        let input = base_input(
            r#"
            console.log('hello', 42, { a: 1 });
            pm.environment.unset('base_url');
            pm.environment.set('x', '1');
            pm.environment.set('x', '2');
            pm.environment.unset('x');
            console.warn('has =', pm.environment.has('x'));
            console.error('url =', pm.request.url, 'method =', pm.request.method);
            "#,
        );
        let result = sandbox.run(input).await.unwrap();
        assert!(result
            .logs
            .iter()
            .any(|l| l.starts_with("[log]") && l.contains("hello 42")));
        assert!(result
            .logs
            .iter()
            .any(|l| l.starts_with("[warn]") && l.contains("has = false")));
        assert!(result.logs.iter().any(|l| l.starts_with("[error]")
            && l.contains("http://localhost/api/users")
            && l.contains("GET")));
        assert!(!result.environment.contains_key("base_url"));
        assert!(!result.environment.contains_key("x"));
    }

    #[tokio::test]
    async fn globals_are_isolated_across_runs() {
        let sandbox = ScriptSandbox::new().unwrap();
        // 第一次运行：污染全局变量。
        sandbox
            .run(base_input(
                "globalThis.__leak = 'stale'; pm.environment.set('k', 'v');",
            ))
            .await
            .unwrap();
        // 第二次运行（复用同一引擎）：不应看到上次泄漏的全局变量，pm 仍可用。
        let result = sandbox
            .run(base_input(
                "pm.test('无泄漏', () => pm.expect(globalThis.__leak).to.eql(undefined));",
            ))
            .await
            .unwrap();
        assert_eq!(result.tests.len(), 1);
        assert!(result.tests[0].passed, "全局状态未隔离：{:?}", result.tests);
    }

    #[tokio::test]
    async fn deep_recursion_errors_without_aborting_process() {
        // 深递归：QuickJS JS 层栈上限（1MB）应触发栈溢出异常返回错误，
        // 而不是冲爆 native 栈直接 abort 进程（此前未设 max_stack_size）。
        let sandbox = ScriptSandbox::new().unwrap();
        let result = sandbox
            .run(base_input("function f() { return f(); } f();"))
            .await;
        assert!(result.is_err(), "深递归应返回脚本错误而非成功");
        // 同一引擎随后仍可正常执行（不是崩溃后重启的假象）。
        let ok = sandbox
            .run(base_input(
                "pm.test('alive', () => pm.expect(1).to.eql(1));",
            ))
            .await
            .unwrap();
        assert!(ok.tests.iter().all(|t| t.passed));
    }

    #[tokio::test]
    async fn sync_infinite_loop_times_out() {
        let sandbox = ScriptSandbox::new().unwrap();
        let start = Instant::now();
        let err = sandbox
            .run(base_input("while (true) {}"))
            .await
            .unwrap_err();
        let elapsed = start.elapsed();
        match err {
            AppError::ScriptError(msg) => assert!(msg.contains("超时"), "意外提示：{msg}"),
            other => panic!("期望 ScriptError，实际 {other}"),
        }
        assert!(
            elapsed >= Duration::from_millis(1500),
            "中断过早：{elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(6), "中断过晚：{elapsed:?}");
    }

    #[tokio::test]
    async fn async_infinite_loop_times_out() {
        let sandbox = ScriptSandbox::new().unwrap();
        let start = Instant::now();
        let err = sandbox
            .run(base_input("while (true) { await Promise.resolve(); }"))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, AppError::ScriptError(msg) if msg.contains("超时")),
            "意外错误：{err}"
        );
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(1500),
            "中断过早：{elapsed:?}"
        );
        assert!(elapsed < Duration::from_secs(6), "中断过晚：{elapsed:?}");
    }

    #[tokio::test]
    async fn runtime_error_is_mapped_to_script_error() {
        let sandbox = ScriptSandbox::new().unwrap();
        let err = sandbox
            .run(base_input("throw new Error('boom');"))
            .await
            .unwrap_err();
        match err {
            AppError::ScriptError(msg) => assert!(msg.contains("boom"), "错误信息丢失：{msg}"),
            other => panic!("期望 ScriptError，实际 {other}"),
        }
    }

    #[tokio::test]
    async fn syntax_error_is_mapped_to_script_error() {
        let sandbox = ScriptSandbox::new().unwrap();
        let err = sandbox.run(base_input("var x = ;")).await.unwrap_err();
        match err {
            AppError::ScriptError(msg) => {
                assert!(msg.contains("脚本执行失败"), "意外提示：{msg}");
                assert!(msg.len() > "脚本执行失败：".len(), "缺少错误详情：{msg}");
            }
            other => panic!("期望 ScriptError，实际 {other}"),
        }
    }

    #[test]
    fn from_spec_builds_request_snapshot() {
        let spec = RequestSpec {
            headers: vec![miyo_core::model::KeyValue::new("A", "1"), {
                let mut kv = miyo_core::model::KeyValue::new("B", "2");
                kv.enabled = false;
                kv
            }],
            body: BodySpec::Json {
                raw: r#"{"a":1}"#.to_string(),
            },
            ..Default::default()
        };
        let snapshot =
            ScriptRequestData::from_spec(HttpMethod::POST, "http://localhost/api", &spec);
        assert_eq!(snapshot.method, "POST");
        assert_eq!(snapshot.headers.len(), 1);
        assert_eq!(snapshot.body, r#"{"a":1}"#);
    }

    #[test]
    fn from_response_builds_response_snapshot() {
        let resp = HttpResponseData {
            status: 404,
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: bytes::Bytes::from_static(br#"{"error":"nope"}"#),
            duration_ms: 12.0,
            size_bytes: 15,
            cookies: Vec::new(),
            truncated: false,
            request: None,
        };
        let snapshot = ScriptResponseData::from_response(&resp);
        assert_eq!(snapshot.status, 404);
        assert_eq!(snapshot.body, r#"{"error":"nope"}"#);
    }
}
