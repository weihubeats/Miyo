//! 本机 Java 进程发现（真实进程扫描，非端口猜测）。
//!
//! 平台实现：
//! - macOS：`ps -axo pid=,uid=,etime=,lstart=,command=`（单次快扫）；
//! - Linux：`/proc/<pid>/cmdline` + `/proc/<pid>/exe` + `/proc/<pid>/status`；
//! - Windows：PowerShell `Get-CimInstance Win32_Process`（java.exe/javaw.exe）。
//!
//! 诚实边界：
//! - attach 可行性以 launcher `list`（Attach API 视角）为准——os_scan 只负责
//!   「这是一个 Java 进程」；不同 uid / 非 HotSpot 一律标注不可 attach 与原因；
//! - 启动参数敏感值脱敏（`redact_command_line`）后回传前端；
//! - 版本 / 厂商经 `<home>/bin/java -version` 探测并按 home 缓存（不重复起进程）。

use std::collections::HashSet;
use std::path::PathBuf;

use crate::jdk::ToolchainRegistry;
use crate::model::{redact_command_line, JavaProcessInfo};

/// 单机发现的进程清单（去重，按 pid 升序）。
#[cfg(target_os = "macos")]
pub async fn scan_java_processes() -> Vec<RawProcess> {
    scan_macos().await
}

#[cfg(target_os = "linux")]
pub async fn scan_java_processes() -> Vec<RawProcess> {
    scan_linux().await
}

#[cfg(target_os = "windows")]
pub async fn scan_java_processes() -> Vec<RawProcess> {
    scan_windows().await
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
pub async fn scan_java_processes() -> Vec<RawProcess> {
    Vec::new()
}

/// 发现前的原始进程行（平台无关形状）。
#[derive(Debug, Clone)]
pub struct RawProcess {
    pub pid: i64,
    pub uid: String,
    pub elapsed: String,
    pub started_at: String,
    pub command_line: String,
}

/// 完整发现：OS 扫描 → launcher attachable 交叉 → 版本/厂商富化（home 缓存）。
///
/// `attachable_pids` 来自 launcher `list`（Attach API 视角）；`attached_pids`
/// 是本 Miyo 会话已 attach 的 pid（前端展示用）。
pub async fn discover_processes(
    registry: &mut ToolchainRegistry,
    attachable_pids: &HashSet<i64>,
    attached_pids: &HashSet<i64>,
) -> Vec<JavaProcessInfo> {
    let raw = scan_java_processes().await;
    let mut out = Vec::with_capacity(raw.len());
    for proc in raw {
        if !looks_like_java(&proc.command_line) {
            continue;
        }
        let mut info = enrich(registry, proc).await;
        info.attached = attached_pids.contains(&info.pid);
        if info.attached {
            info.attachable = true;
            info.attach_block_reason = None;
        } else if attachable_pids.contains(&info.pid) {
            info.attachable = true;
        } else {
            info.attachable = false;
            info.attach_block_reason = Some(attach_block_reason(&info));
        }
        out.push(info);
    }
    out.sort_by_key(|p| p.pid);
    out
}

/// 手动输入 pid 的单进程探测（不存在 → None）。
pub async fn probe_pid(
    pid: i64,
    registry: &mut ToolchainRegistry,
    attachable_pids: &HashSet<i64>,
) -> Option<JavaProcessInfo> {
    let raw = probe_single(pid).await?;
    if !looks_like_java(&raw.command_line) {
        return Some(JavaProcessInfo {
            pid,
            display_name: raw.command_line.chars().take(80).collect(),
            command_line: redact_command_line(&raw.command_line),
            uid: raw.uid.clone(),
            attach_block_reason: Some("该 pid 不是 Java 进程".into()),
            ..Default::default()
        });
    }
    let mut info = enrich(registry, raw).await;
    info.source = "manual_pid".into();
    info.attachable = attachable_pids.contains(&pid);
    if !info.attachable {
        info.attach_block_reason = Some(attach_block_reason(&info));
    }
    Some(info)
}

/// 进程命令行是否像 Java（可执行名 java / 命令行含 java 路径或 main 类形态）。
fn looks_like_java(command_line: &str) -> bool {
    let lower = command_line.to_lowercase();
    lower.contains("/bin/java")
        || lower.contains("\\bin\\java")
        || lower.starts_with("java")
        || lower.contains("/jre/bin/")
        || lower.contains("org.gradle.launcher.daemon")
        || lower.contains("kotlin.compiler.daemon")
}

/// 单进程信息富化：可执行 / home / 主类 / 版本（home 级缓存）。
async fn enrich(registry: &mut ToolchainRegistry, proc: RawProcess) -> JavaProcessInfo {
    let tokens: Vec<&str> = proc.command_line.split_whitespace().collect();
    // java 可执行 = 含 /bin/java 的第一个 token，或首个裸 java
    let mut java_exec = String::new();
    for (i, token) in tokens.iter().enumerate() {
        let lower = token.to_lowercase();
        if lower.ends_with("/bin/java")
            || lower.ends_with("\\bin\\java.exe")
            || lower.ends_with("/bin/java.exe")
            || (i == 0 && (lower == "java" || lower == "java.exe"))
        {
            java_exec = token.to_string();
            break;
        }
    }
    let java_home = infer_java_home(&java_exec);
    let (java_version, jvm_vendor, kind) = if !java_home.is_empty() {
        let toolchain = toolchain_for_home(registry, &java_home).await;
        (
            toolchain.version_string.clone(),
            toolchain.vendor.clone(),
            if toolchain.has_javac { "jdk" } else { "jre" }.to_string(),
        )
    } else {
        (String::new(), String::new(), "unknown".into())
    };
    let main_class = extract_main_class(&tokens);
    let display = if main_class.is_empty() {
        // 取命令行最后一个有意义的 token 或截断全行
        tokens
            .last()
            .map(|t| t.chars().take(80).collect())
            .unwrap_or_else(|| "java".into())
    } else {
        main_class.clone()
    };
    JavaProcessInfo {
        pid: proc.pid,
        main_class,
        display_name: display,
        command_line: redact_command_line(&proc.command_line),
        java_executable: java_exec,
        java_home,
        java_version,
        jvm_vendor,
        kind,
        started_at: proc.started_at,
        elapsed: proc.elapsed,
        uid: proc.uid,
        attachable: false,
        attach_block_reason: None,
        attached: false,
        source: "os_scan".into(),
    }
}

/// home → 工具链信息（注册表缓存 miss 时探测一次）。
async fn toolchain_for_home(
    registry: &mut ToolchainRegistry,
    home: &str,
) -> crate::jdk::JdkToolchain {
    let cached = registry.all().iter().find(|t| t.home == home).cloned();
    if let Some(found) = cached {
        return found;
    }
    let probed = crate::jdk::JdkToolchain::probe(PathBuf::from(home), "process_home");
    registry.upsert(probed.clone());
    probed
}

/// 从 java 可执行路径推断 JAVA_HOME（macOS Contents/Home / 标准 bin 布局）。
fn infer_java_home(java_exec: &str) -> String {
    if java_exec.is_empty() {
        return String::new();
    }
    let path = PathBuf::from(java_exec);
    if let Some(bin) = path.parent() {
        // <home>/bin/java
        if let Some(home) = bin.parent() {
            let home_str = home.to_string_lossy().to_string();
            if home_str.ends_with("Contents/Home") || home_str != "/" {
                return home_str;
            }
        }
    }
    String::new()
}

/// 主类 / jar 入口提取（`-cp X Main` → Main；`-jar app.jar` → app.jar）。
fn extract_main_class(tokens: &[&str]) -> String {
    let mut i = 0;
    while i < tokens.len() {
        let token = tokens[i];
        if token == "-jar" || token == "--jar" {
            if let Some(jar) = tokens.get(i + 1) {
                // 取文件名（启动命令常带绝对路径）
                return jar.rsplit(['/', '\\']).next().unwrap_or(jar).to_string();
            }
        }
        if token == "-cp" || token == "-classpath" || token == "--class-path" {
            i += 2; // 跳过 cp 值
            continue;
        }
        if !token.starts_with('-') && token.contains('.') && !token.contains('/') && i > 0 {
            // 形如 com.example.Main 的候选：优先带包名的最后一个
            let candidate = token.to_string();
            if candidate.contains('.') && !candidate.contains('=') {
                let mut j = i + 1;
                let mut last = candidate.clone();
                while j < tokens.len() {
                    let t = tokens[j];
                    if !t.starts_with('-')
                        && t.contains('.')
                        && !t.contains('/')
                        && !t.contains('=')
                    {
                        last = t.to_string();
                    } else if t.starts_with('-') {
                        break;
                    }
                    j += 1;
                }
                return last;
            }
        }
        i += 1;
    }
    String::new()
}

/// 不可 attach 的原因（诚实提示）。
fn attach_block_reason(info: &JavaProcessInfo) -> String {
    let current_uid = current_user_id();
    if !info.uid.is_empty() && !current_uid.is_empty() && info.uid != current_uid {
        return format!(
            "进程属于其他用户（uid {}），当前用户（uid {}）无权 attach",
            info.uid, current_uid
        );
    }
    if !info.kind.is_empty() && info.kind == "unknown" {
        return "无法识别目标 JVM 版本（java 可执行不可探测）".into();
    }
    "未通过 Attach listener 探测（可能非 HotSpot JVM、禁止 attach，或进程刚退出）".into()
}

/// 当前用户 id（macOS/Linux `id -u`；Windows 返回空——Windows attach 按
/// 会话隔离，launcher 错误码会给出明确原因）。
pub fn current_user_id() -> String {
    if let Ok(output) = std::process::Command::new("id").arg("-u").output() {
        if output.status.success() {
            return String::from_utf8_lossy(&output.stdout).trim().to_string();
        }
    }
    String::new()
}

// ---------------------------------------------------------------------------
// 平台实现
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
async fn scan_macos() -> Vec<RawProcess> {
    let output = tokio::process::Command::new("ps")
        .args(["-axo", "pid=,uid=,etime=,lstart=,command="])
        .output()
        .await
        .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() {
            continue;
        }
        // pid uid etime lstart=Mon Nov 17 20:00:00 2026 command...
        let mut parts = line.splitn(2, char::is_whitespace);
        let pid: i64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        if pid == 0 {
            continue;
        }
        let rest = parts.next().unwrap_or("").trim_start();
        let mut rest_parts = rest.splitn(2, char::is_whitespace);
        let uid = rest_parts.next().unwrap_or("").to_string();
        let rest = rest_parts.next().unwrap_or("").trim_start();
        // etime（[[dd-]hh:]mm:ss）
        let mut rest_parts = rest.splitn(2, char::is_whitespace);
        let elapsed = rest_parts.next().unwrap_or("").to_string();
        let rest = rest_parts.next().unwrap_or("").trim_start();
        // lstart = 7 个 token（Mon Nov 17 20:00:00 2026）后是命令
        let tokens: Vec<&str> = rest.splitn(8, char::is_whitespace).collect();
        if tokens.len() < 8 {
            continue;
        }
        let started_at = tokens[..7].join(" ");
        let command_line = tokens[7].to_string();
        out.push(RawProcess {
            pid,
            uid,
            elapsed,
            started_at,
            command_line,
        });
    }
    out
}

#[cfg(target_os = "linux")]
async fn scan_linux() -> Vec<RawProcess> {
    let mut out = Vec::new();
    let mut entries = match tokio::fs::read_dir("/proc").await {
        Ok(entries) => entries,
        Err(_) => return out,
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let pid: i64 = match name.parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let Ok(cmdline) = tokio::fs::read(format!("/proc/{pid}/cmdline")).await else {
            continue;
        };
        let command_line = String::from_utf8_lossy(&cmdline)
            .split('\0')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if command_line.is_empty() {
            continue;
        }
        // uid + started（/proc/<pid>/status + stat starttime 太复杂——etime 用 ps 兜底）
        let status = tokio::fs::read_to_string(format!("/proc/{pid}/status"))
            .await
            .unwrap_or_default();
        let uid = status
            .lines()
            .find(|l| l.starts_with("Uid:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("")
            .to_string();
        let exe = tokio::fs::read_link(format!("/proc/{pid}/exe"))
            .await
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        // started_at 粗略：进程目录 ctime；elapsed 用 ps -p 补
        let mut elapsed = String::new();
        let mut started_at = String::new();
        if let Ok(ps_out) = tokio::process::Command::new("ps")
            .args(["-p", &pid.to_string(), "-o", "etime=,lstart="])
            .output()
            .await
        {
            let text = String::from_utf8_lossy(&ps_out.stdout);
            let mut parts = text.split_whitespace();
            elapsed = parts.next().unwrap_or("").to_string();
            let rest: Vec<&str> = parts.collect();
            if rest.len() >= 5 {
                started_at = rest.join(" ");
            }
        }
        let command_line = if exe.contains("/java") {
            // exe 路径不在 cmdline 里时（JVM wrapper）补上
            if command_line.contains("/bin/java") {
                command_line
            } else {
                format!("{exe} {command_line}")
            }
        } else {
            command_line
        };
        out.push(RawProcess {
            pid,
            uid,
            elapsed,
            started_at,
            command_line,
        });
    }
    out
}

#[cfg(target_os = "windows")]
async fn scan_windows() -> Vec<RawProcess> {
    // PowerShell CIM 查询（java.exe / javaw.exe），JSON 输出
    let script = "Get-CimInstance Win32_Process -Filter \"Name='java.exe' OR Name='javaw.exe'\" \
        | Select-Object ProcessId,CommandLine,CreationDate | ConvertTo-Json -Compress";
    let output = tokio::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .output()
        .await
        .ok();
    let Some(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let mut out = Vec::new();
    let parsed: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return out,
    };
    // 单条记录是对象、多条是数组——统一成数组
    let items = match parsed {
        serde_json::Value::Array(items) => items,
        obj @ serde_json::Value::Object(_) => vec![obj],
        _ => Vec::new(),
    };
    for item in items {
        let pid = item.get("ProcessId").and_then(|v| v.as_i64()).unwrap_or(0);
        if pid == 0 {
            continue;
        }
        let command_line = item
            .get("CommandLine")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let started = item
            .get("CreationDate")
            .map(|v| match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Object(map) => map
                    .get("DateTime")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
                _ => String::new(),
            })
            .unwrap_or_default();
        out.push(RawProcess {
            pid,
            uid: String::new(), // Windows 按会话隔离（GetOwner 需要额外查询；错误码会提示）
            elapsed: String::new(),
            started_at: started,
            command_line,
        });
    }
    out
}

/// 手动 pid 探测（跨平台：/proc 或 ps）。
async fn probe_single(pid: i64) -> Option<RawProcess> {
    if cfg!(target_os = "linux") {
        let cmdline = tokio::fs::read(format!("/proc/{pid}/cmdline")).await.ok()?;
        let command_line = String::from_utf8_lossy(&cmdline)
            .split('\0')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let status = tokio::fs::read_to_string(format!("/proc/{pid}/status"))
            .await
            .unwrap_or_default();
        let uid = status
            .lines()
            .find(|l| l.starts_with("Uid:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .unwrap_or("")
            .to_string();
        return Some(RawProcess {
            pid,
            uid,
            elapsed: String::new(),
            started_at: String::new(),
            command_line,
        });
    }
    // macOS / Windows：ps -p（Windows 的 ps 不存在——走 powershell CIM）
    if cfg!(target_os = "windows") {
        let script = format!(
            "Get-CimInstance Win32_Process -Filter \"ProcessId={pid}\" \
             | Select-Object ProcessId,CommandLine,CreationDate | ConvertTo-Json -Compress"
        );
        let output = tokio::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .await
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout).to_string();
        let parsed: serde_json::Value = serde_json::from_str(&text).ok()?;
        let command_line = parsed
            .get("CommandLine")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        if command_line.is_empty() {
            return None;
        }
        return Some(RawProcess {
            pid,
            uid: String::new(),
            elapsed: String::new(),
            started_at: String::new(),
            command_line,
        });
    }
    let output = tokio::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "uid=,etime=,lstart=,command="])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return None;
    }
    let mut parts = text.splitn(2, char::is_whitespace);
    let uid = parts.next().unwrap_or("").to_string();
    let rest = parts.next().unwrap_or("").trim_start();
    let mut rest_parts = rest.splitn(2, char::is_whitespace);
    let elapsed = rest_parts.next().unwrap_or("").to_string();
    let rest = rest_parts.next().unwrap_or("").trim_start();
    let tokens: Vec<&str> = rest.splitn(8, char::is_whitespace).collect();
    if tokens.len() < 8 {
        // lstart 可能解析失败——整行当命令
        return Some(RawProcess {
            pid,
            uid,
            elapsed,
            started_at: String::new(),
            command_line: rest.to_string(),
        });
    }
    Some(RawProcess {
        pid,
        uid,
        elapsed,
        started_at: tokens[..7].join(" "),
        command_line: tokens[7].to_string(),
    })
}

// ---------------------------------------------------------------------------
// 测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_class_extraction() {
        assert_eq!(
            extract_main_class(&["/usr/bin/java", "-cp", "build", "com.example.PlainApp"]),
            "com.example.PlainApp"
        );
        assert_eq!(
            extract_main_class(&["java", "-jar", "/opt/app/service.jar", "--port=8080"]),
            "service.jar"
        );
        assert_eq!(
            extract_main_class(&[
                "java",
                "-Dspring.profiles.active=dev",
                "-cp",
                "a:b:c",
                "com.example.App",
                "--x"
            ]),
            "com.example.App"
        );
        assert_eq!(extract_main_class(&["java", "-version"]), "");
    }

    #[test]
    fn java_home_inference() {
        assert_eq!(
            infer_java_home("/Library/Java/JavaVirtualMachines/zulu-8.jdk/Contents/Home/bin/java"),
            "/Library/Java/JavaVirtualMachines/zulu-8.jdk/Contents/Home"
        );
        assert_eq!(
            infer_java_home("/usr/lib/jvm/java-21/bin/java"),
            "/usr/lib/jvm/java-21"
        );
        assert_eq!(infer_java_home("java"), "");
    }

    #[test]
    fn looks_like_java_detection() {
        assert!(looks_like_java("/usr/bin/java -jar app.jar"));
        assert!(looks_like_java(
            "/Library/Java/.../bin/java -cp build com.x.Main"
        ));
        assert!(!looks_like_java("/bin/bash /opt/start.sh"));
        assert!(!looks_like_java("/usr/bin/python3 server.py"));
    }
}
