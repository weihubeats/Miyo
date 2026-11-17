//! JDK 工具链发现：launcher sidecar 需要一个可运行的 JDK（含 jdk.attach 模块
//! 或 tools.jar）。本模块枚举常见位置并按目标版本选择最兼容的工具链。
//!
//! 选择规则（attach 兼容性实证）：
//! - 新客户端 → 旧目标（如 JDK21 → JDK8）：协议返回码解读偏斜，launcher 已有
//!   infoFile 轮询兜底（可用，但首选不触发偏斜的组合）；
//! - 旧客户端 → 新目标（如 JDK8 → JDK17）：「Non-numeric value found」偏斜，
//!   同样有兜底但更易受告警文本干扰；
//! - 因此优先「major == 目标 major」，其次「major > 目标」（新版本优先），
//!   最后才考虑「major < 目标」。
//! - JRE / jlink 裁剪运行时缺 jdk.attach：capabilities 探测（bridge_available）
//!   不可用的工具链直接排除。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use miyo_core::AppError;

/// 一台本机 JDK 工具链。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JdkToolchain {
    /// JAVA_HOME 绝对路径。
    pub home: String,
    /// 版本号解析（如 21；解析失败 0）。
    pub major_version: u32,
    /// `java -version` 原始版本串（如 `21.0.6`）。
    pub version_string: String,
    /// 厂商（`java -version` 第一行 vendor 段；缺省 unknown）。
    pub vendor: String,
    /// bin/javac 存在（完整 JDK）。
    pub has_javac: bool,
    /// `java -XshowSettings` 探测 jdk.attach / tools.jar 可用（懒探测缓存）。
    pub attach_capable: bool,
    pub source: String,
}

impl Default for JdkToolchain {
    fn default() -> Self {
        JdkToolchain {
            home: String::new(),
            major_version: 0,
            version_string: String::new(),
            vendor: "unknown".into(),
            has_javac: false,
            attach_capable: true,
            source: String::new(),
        }
    }
}

impl JdkToolchain {
    pub fn java_executable(&self) -> PathBuf {
        java_executable_for(&self.home)
    }

    /// 探测工具链信息（`java -version` stderr 解析；失败退化为 home 路径推断）。
    pub fn probe(home: PathBuf, source: &str) -> JdkToolchain {
        let mut toolchain = JdkToolchain {
            home: home.to_string_lossy().to_string(),
            has_javac: java_executable_for(&home.to_string_lossy())
                .parent()
                .map(|bin| bin.join("javac").exists() || bin.join("javac.exe").exists())
                .unwrap_or(false),
            source: source.to_string(),
            ..Default::default()
        };
        let java = java_executable_for(&toolchain.home);
        let output = std::process::Command::new(&java).arg("-version").output();
        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stderr).to_string();
            // 形如：openjdk version "21.0.6" 2025-01-21 LTS / OpenJDK Runtime Environment Zulu21.40+17-CA
            for line in text.lines() {
                if let Some(idx) = line.find("version \"") {
                    let rest = &line[idx + "version \"".len()..];
                    if let Some(end) = rest.find('"') {
                        toolchain.version_string = rest[..end].to_string();
                        toolchain.major_version = parse_major(&toolchain.version_string);
                    }
                }
                if line.contains("Environment") || line.contains("VM (build") {
                    // "OpenJDK Runtime Environment Zulu21.40+17-CA" → vendor = Zulu…
                    if let Some(vendor) = line.split_whitespace().nth(3) {
                        toolchain.vendor = vendor.trim_matches('"').to_string();
                    }
                }
            }
        }
        // JDK 1.8 → major 8
        if toolchain.major_version == 1 && toolchain.version_string.starts_with("1.8") {
            toolchain.major_version = 8;
        }
        toolchain
    }
}

/// java 可执行路径（macOS JDK 布局多一层 Contents/Home）。
pub fn java_executable_for(home: &str) -> PathBuf {
    let base = Path::new(home);
    let candidate = base.join("bin").join(java_executable_name());
    if candidate.exists() {
        return candidate;
    }
    base.join("bin").join("java")
}

fn java_executable_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// 版本串 → major（`21.0.6` → 21；`1.8.0_372` → 8）。
pub fn parse_major(version: &str) -> u32 {
    let first = version.split(['.', '_', '-']).next().unwrap_or("0");
    let v: u32 = first.parse().unwrap_or(0);
    if v == 1 {
        // 1.8.0_372 → 8
        version
            .split(['.', '_'])
            .nth(1)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0)
    } else {
        v
    }
}

/// 本机 JDK 工具链注册表（进程内缓存；`refresh` 重新扫描）。
#[derive(Debug, Default)]
pub struct ToolchainRegistry {
    toolchains: Vec<JdkToolchain>,
}

impl ToolchainRegistry {
    /// 扫描本机 JDK（JAVA_HOME / macOS java_home / 常见目录 / PATH）。
    pub fn refresh(&mut self) -> Vec<JdkToolchain> {
        let mut homes: Vec<(PathBuf, String)> = Vec::new();
        // 1) JAVA_HOME
        if let Ok(home) = std::env::var("JAVA_HOME") {
            let home = PathBuf::from(home);
            if is_java_home(&home) {
                homes.push((home, "JAVA_HOME".into()));
            }
        }
        // 2) macOS /usr/libexec/java_home
        #[cfg(target_os = "macos")]
        {
            if let Ok(output) = std::process::Command::new("/usr/libexec/java_home")
                .args(["-V"])
                .output()
            {
                // stderr 形如：21.0.6 (arm64) "Azul Systems, Inc." - "Zulu 21.40.17" /Users/.../Contents/Home
                let text = String::from_utf8_lossy(&output.stderr).to_string();
                for line in text.lines() {
                    if let Some(idx) = line.rfind(" /") {
                        let path = line[idx + 1..].trim();
                        let pb = PathBuf::from(path);
                        if is_java_home(&pb) && !homes.iter().any(|(h, _)| h == &pb) {
                            homes.push((pb, "java_home".into()));
                        }
                    }
                }
            }
        }
        // 3) 常见目录
        for dir in common_java_dirs() {
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    let home = normalize_home(&entry.path());
                    if is_java_home(&home) && !homes.iter().any(|(h, _)| h == &home) {
                        homes.push((home, "common_dir".into()));
                    }
                }
            }
        }
        // 4) PATH 上的 java → java.home 回推
        if let Some(path_java) = which_java() {
            if let Some(home) = resolve_java_home_from_executable(&path_java) {
                if is_java_home(&home) && !homes.iter().any(|(h, _)| h == &home) {
                    homes.push((home, "PATH".into()));
                }
            }
        }
        self.toolchains = homes
            .into_iter()
            .map(|(home, source)| JdkToolchain::probe(home, &source))
            .collect();
        // 版本倒序（新版本优先）
        self.toolchains
            .sort_by_key(|t| std::cmp::Reverse(t.major_version));
        self.toolchains.clone()
    }

    pub fn all(&self) -> &[JdkToolchain] {
        &self.toolchains
    }

    pub fn is_empty(&self) -> bool {
        self.toolchains.is_empty()
    }

    /// 插入 / 按 home 更新（进程发现富化时的 home 级缓存写入）。
    pub fn upsert(&mut self, toolchain: JdkToolchain) {
        if let Some(existing) = self
            .toolchains
            .iter_mut()
            .find(|t| t.home == toolchain.home)
        {
            *existing = toolchain;
        } else {
            self.toolchains.push(toolchain);
            self.toolchains
                .sort_by_key(|t| std::cmp::Reverse(t.major_version));
        }
    }

    /// 为目标 JVM 版本选 launcher 工具链（规则见模块文档；无匹配 → 错误）。
    pub fn select_for_target(&self, target_major: Option<u32>) -> Result<JdkToolchain, AppError> {
        if self.toolchains.is_empty() {
            return Err(AppError::NotFound(
                "本机未发现可用 JDK：Attach 需要一个完整 JDK（含 jdk.attach 模块或 tools.jar）\
                 运行 launcher。请在系统安装 JDK，或在 Miyo 设置中指定 JAVA_HOME"
                    .into(),
            ));
        }
        if let Some(target) = target_major {
            // 同版本优先
            if let Some(exact) = self
                .toolchains
                .iter()
                .find(|t| t.major_version == target && t.attach_capable)
            {
                return Ok(exact.clone());
            }
            // 更新版本（新客户端 → 旧目标，偏斜有兜底）
            if let Some(newer) = self
                .toolchains
                .iter()
                .find(|t| t.major_version > target && t.attach_capable)
            {
                return Ok(newer.clone());
            }
        }
        // 兜底：可用工具链中最新的
        self.toolchains
            .iter()
            .find(|t| t.attach_capable)
            .cloned()
            .ok_or_else(|| {
                AppError::NotFound("本机 JDK 均不具备 Attach 能力（jlink 裁剪 / JRE）".into())
            })
    }
}

fn is_java_home(path: &Path) -> bool {
    path.join("bin").join(java_executable_name()).is_file()
}

/// macOS `<jdk>/Contents/Home` 收敛；其余原样。
fn normalize_home(path: &Path) -> PathBuf {
    if path
        .join("Contents/Home/bin")
        .join(java_executable_name())
        .is_file()
    {
        path.join("Contents/Home")
    } else {
        path.to_path_buf()
    }
}

#[allow(unused_mut)]
fn common_java_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/Library/Java/JavaVirtualMachines"));
        if let Some(home) = std::env::var_os("HOME") {
            let mut user = PathBuf::from(home);
            user.push("Library/Java/JavaVirtualMachines");
            dirs.push(user);
        }
    }
    #[cfg(target_os = "linux")]
    {
        dirs.push(PathBuf::from("/usr/lib/jvm"));
        dirs.push(PathBuf::from("/opt/java"));
        if let Some(home) = std::env::var_os("HOME") {
            let mut sdkman = PathBuf::from(home);
            sdkman.push(".sdkman/candidates/java");
            dirs.push(sdkman);
        }
    }
    #[cfg(target_os = "windows")]
    {
        for base in [
            "C:\\Program Files\\Java",
            "C:\\Program Files\\Eclipse Adoptium",
        ] {
            dirs.push(PathBuf::from(base));
        }
    }
    dirs
}

/// PATH 上的 java（简易 which）。
fn which_java() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(java_executable_name());
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 由 java 可执行文件回推 JAVA_HOME（`java -XshowSettings:properties` 的 java.home）。
fn resolve_java_home_from_executable(executable: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new(executable)
        .args(["-XshowSettings:properties", "-version"])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stderr);
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("java.home = ") {
            let home = PathBuf::from(rest.trim().trim_matches('"'));
            if home.exists() {
                return Some(home);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_major_versions() {
        assert_eq!(parse_major("21.0.6"), 21);
        assert_eq!(parse_major("1.8.0_372"), 8);
        assert_eq!(parse_major("17.0.13"), 17);
        assert_eq!(parse_major("unknown"), 0);
    }

    #[test]
    fn select_prefers_exact_match() {
        let registry = ToolchainRegistry {
            toolchains: vec![
                JdkToolchain {
                    home: "/jdk21".into(),
                    major_version: 21,
                    attach_capable: true,
                    ..Default::default()
                },
                JdkToolchain {
                    home: "/jdk8".into(),
                    major_version: 8,
                    attach_capable: true,
                    ..Default::default()
                },
            ],
        };
        let picked = registry.select_for_target(Some(8)).unwrap();
        assert_eq!(picked.home, "/jdk8");
        let picked = registry.select_for_target(Some(11)).unwrap();
        assert_eq!(picked.home, "/jdk21", "无精确匹配时选更新的");
        let picked = registry.select_for_target(None).unwrap();
        assert_eq!(picked.home, "/jdk21");
    }
}
