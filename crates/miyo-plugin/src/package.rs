//! 插件包（`.miyo` = tar.gz）安装：解包、完整性、签名校验、路径穿越防护。
//!
//! 安全模型（manifest 2.0）：
//! - **完整性**：`integrity` = manifest.json 字节的 sha256（hex）。安装时计算
//!   并落库；激活前可复核（`verify_manifest_integrity`），包被篡改即拒绝。
//! - **全文件清单**：manifest 可声明 `files`（path + sha256 + size）。声明即
//!   强制——检视 / 解包 / 启用三个环节逐文件验 sha256，且包内文件集合必须与
//!   清单完全一致（多余文件拒绝）；`entry` / `sidecar.executable` 必须在清单内。
//! - **签名**：ed25519，官方公钥编译期内置（keyring 支持轮换，任一命中即
//!   official）。签名 payload 见 [`signing_payload`]（规范化 manifest +
//!   files 行清单）——manifest 内容与全部文件摘要都在覆盖范围内。
//!   未签名包不拒绝安装，但必须由用户显式信任（UI 确认 + 落库 `signed=false`）。
//! - **解包防护**：仅接受常规文件；拒绝绝对路径、`..` 段、反斜杠、符号链接；
//!   条目数 ≤ 200、单文件 ≤ 10 MiB、总解包体积 ≤ 64 MiB；落盘路径全部相对 dest。
//! - **目录形态**：开发期允许直接从目录安装（读取目录内 manifest.json），
//!   完整性同样按 manifest 字节计算。

use std::io::Read;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use sha2::{Digest, Sha256};
use tar::Archive;

use miyo_core::AppError;

use crate::manifest::{PluginFileEntry, PluginManifest};

/// 解包限制。
pub const MAX_ENTRIES: usize = 200;
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// ed25519 官方发布公钥 keyring（hex，32 字节；支持密钥轮换——任一命中即
/// official 信任级）。新密钥追加到表尾，旧密钥退役后移除。
/// 对应私钥只保存在 CI Secret（建议名 `MIYO_PLUGIN_SIGNING_SEED`），
/// 签名工具见 scripts/sign_plugin.mjs。
pub const OFFICIAL_SIGNING_PUBKEYS: &[&str] =
    &["92739639878299e7f459b10fe3ba2c6994559594ff2feb1cbf61eef3d71cd206"];

/// 签名信任级（安装落库 / UI 展示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrustLevel {
    /// 官方 keyring 验签通过。
    Official,
    /// 声明了签名但未命中官方 keyring（如第三方自签）。
    Foreign,
    /// 未声明签名。
    Unsigned,
}

/// 包检视结果（安装确认页展示：清单 + 完整性 + 签名状态）。
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub manifest: PluginManifest,
    /// manifest.json 字节的 sha256（hex）。
    pub integrity: String,
    /// 签名是否通过官方公钥校验。
    pub signed: bool,
    /// 签名信任级。
    pub trust: TrustLevel,
    /// 全文件清单校验结果：None = manifest 未声明 files（v1 包）；
    /// Some(true) = 清单存在且包内全部文件逐字节吻合；Some(false) = 不吻合
    /// （检视阶段即发现篡改，安装流程拒绝）。
    pub files_verified: Option<bool>,
}

/// 构造签名 payload（规范化、跨语言确定性）。
///
/// 定义（签名工具与验证端必须一致，见 scripts/sign_plugin.mjs）：
/// 1. manifest.json 解析为 JSON 对象；`signature` 键**恒参与** payload——
///    值替换为 128 个 `'0'`（原文缺失该键时也插入置零键；解决签名覆盖
///    自身的鸡生蛋问题，并保证「签名前计算」与「验签时重算」形状一致）；
/// 2. 按键名字典序、无空白序列化（serde_json 默认 BTreeMap 行为，
///    Node 侧按同规则排序）→ 字节 A；
/// 3. files 行清单：`{path} {sha256hex} {size}\n`，按 path 字典序排序 → 字节 B；
/// 4. payload = A（无 files 时）或 `A || '\n' || B`。
pub fn signing_payload(manifest_bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut value: serde_json::Value = serde_json::from_slice(manifest_bytes)
        .map_err(|e| AppError::Validation(format!("manifest.json 解析失败：{e}")))?;
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "signature".to_string(),
            serde_json::Value::String("0".repeat(128)),
        );
    }
    let manifest: PluginManifest = serde_json::from_value(value.clone())
        .map_err(|e| AppError::Validation(format!("manifest.json 校验失败：{e}")))?;
    let mut payload =
        serde_json::to_vec(&value).map_err(|e| AppError::Validation(format!("规范化失败：{e}")))?;
    if let Some(files) = &manifest.files {
        payload.push(b'\n');
        let mut sorted: Vec<&PluginFileEntry> = files.iter().collect();
        sorted.sort_by(|a, b| a.path.cmp(&b.path));
        for f in sorted {
            payload.extend_from_slice(format!("{} {} {}\n", f.path, f.sha256, f.size).as_bytes());
        }
    }
    Ok(payload)
}

/// 从 `.miyo` 包字节检视（不落盘；files 清单在解包前对流式二次遍历预验）。
pub fn inspect_package_bytes(bytes: &[u8]) -> Result<PackageInfo, AppError> {
    inspect_package_bytes_with_keys(bytes, OFFICIAL_SIGNING_PUBKEYS)
}

pub(crate) fn inspect_package_bytes_with_keys(
    bytes: &[u8],
    keys: &[&str],
) -> Result<PackageInfo, AppError> {
    let manifest_bytes = extract_manifest_bytes(bytes)?;
    let manifest = PluginManifest::parse(&manifest_bytes)?;
    let integrity = sha256_hex(&manifest_bytes);
    let trust = verify_signature_with_keys(&manifest, &manifest_bytes, keys, TrustLevel::Official);
    let files_verified = match &manifest.files {
        Some(files) => Some({
            verify_archive_files(bytes, files)?;
            true
        }),
        None => None,
    };
    Ok(PackageInfo {
        manifest,
        integrity,
        signed: trust == TrustLevel::Official,
        trust,
        files_verified,
    })
}

/// 从插件目录检视（开发期目录安装）。
pub fn inspect_package_dir(dir: &Path) -> Result<PackageInfo, AppError> {
    inspect_package_dir_with_keys(dir, OFFICIAL_SIGNING_PUBKEYS)
}

pub(crate) fn inspect_package_dir_with_keys(
    dir: &Path,
    keys: &[&str],
) -> Result<PackageInfo, AppError> {
    let manifest_path = dir.join("manifest.json");
    let manifest_bytes = read_file(&manifest_path, "manifest.json")?;
    let manifest = PluginManifest::parse(&manifest_bytes)?;
    let integrity = sha256_hex(&manifest_bytes);
    let trust = verify_signature_with_keys(&manifest, &manifest_bytes, keys, TrustLevel::Official);
    let files_verified = match &manifest.files {
        Some(files) => Some({
            verify_dir_files(dir, files)?;
            true
        }),
        None => None,
    };
    Ok(PackageInfo {
        manifest,
        integrity,
        signed: trust == TrustLevel::Official,
        trust,
        files_verified,
    })
}

/// 把 `.miyo` 包解包到目标目录（调用方保证 dest 为全新空目录）。
pub fn extract_package_to_dir(bytes: &[u8], dest: &Path) -> Result<PackageInfo, AppError> {
    extract_package_to_dir_with_keys(bytes, dest, OFFICIAL_SIGNING_PUBKEYS)
}

pub(crate) fn extract_package_to_dir_with_keys(
    bytes: &[u8],
    dest: &Path,
    keys: &[&str],
) -> Result<PackageInfo, AppError> {
    std::fs::create_dir_all(dest).map_err(|e| AppError::Io(io_err("创建插件目录失败", e)))?;
    let dest_canonical = dest
        .canonicalize()
        .map_err(|e| AppError::Io(io_err("解析插件目录失败", e)))?;

    let gz = GzDecoder::new(bytes);
    let mut archive = Archive::new(gz);
    archive.set_preserve_permissions(false);

    let mut total: u64 = 0;
    let mut count: usize = 0;
    let mut manifest_bytes: Option<Vec<u8>> = None;

    for entry in archive
        .entries()
        .map_err(|e| AppError::Validation(format!("插件包读取失败：{e}")))?
    {
        let mut entry =
            entry.map_err(|e| AppError::Validation(format!("插件包条目读取失败：{e}")))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(AppError::Validation(format!(
                "插件包条目数超过上限（{MAX_ENTRIES}）"
            )));
        }
        let header = entry.header();
        let entry_type = header.entry_type();
        // 接受常规文件与目录条目（tar 打包常含目录项）；符号链接 / 硬链接 /
        // 设备等其他类型一律拒绝（防链接逃逸）。
        if entry_type != tar::EntryType::Regular && entry_type != tar::EntryType::Directory {
            return Err(AppError::Validation(format!(
                "插件包含非常规条目（{entry_type:?}），已拒绝"
            )));
        }
        let size = header
            .size()
            .map_err(|e| AppError::Validation(format!("插件包条目大小非法：{e}")))?;
        if size > MAX_FILE_BYTES {
            return Err(AppError::Validation("插件包单文件超过 10 MiB 上限".into()));
        }
        total = total.saturating_add(size);
        if total > MAX_TOTAL_BYTES {
            return Err(AppError::Validation(
                "插件包总解包体积超过 64 MiB 上限".into(),
            ));
        }
        let raw_path = header
            .path()
            .map_err(|e| AppError::Validation(format!("插件包条目路径非法：{e}")))?
            .to_path_buf();
        let safe = sanitize_entry_path(&raw_path.to_string_lossy())?;
        let out_path = dest.join(&safe);
        if entry_type == tar::EntryType::Directory {
            std::fs::create_dir_all(&out_path)
                .map_err(|e| AppError::Io(io_err("创建插件目录失败", e)))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| AppError::Io(io_err("创建插件子目录失败", e)))?;
        }
        let created = std::fs::File::create(&out_path)
            .map_err(|e| AppError::Io(io_err("写入插件文件失败", e)))?;
        let written = std::io::copy(&mut entry, &mut std::io::BufWriter::new(created))? as u64;
        if written != size {
            return Err(AppError::Validation("插件包条目大小与声明不符".into()));
        }
        if safe.as_os_str() == "manifest.json" {
            manifest_bytes = Some(
                std::fs::read(dest.join(&safe))
                    .map_err(|e| AppError::Io(io_err("回读 manifest.json 失败", e)))?,
            );
        }
    }

    let manifest_bytes =
        manifest_bytes.ok_or_else(|| AppError::Validation("插件包缺少 manifest.json".into()))?;
    // 规范化前缀复核：任何形式的落盘逃逸（符号穿越 / 绝对路径）在这里兜底。
    let canonical_check = std::fs::canonicalize(dest.join("manifest.json"))
        .map_err(|e| AppError::Io(io_err("解析 manifest.json 失败", e)))?;
    if !canonical_check.starts_with(&dest_canonical) {
        return Err(AppError::Validation("插件包路径逃逸，已拒绝".into()));
    }

    let manifest = PluginManifest::parse(&manifest_bytes)?;
    let integrity = sha256_hex(&manifest_bytes);
    let trust = verify_signature_with_keys(&manifest, &manifest_bytes, keys, TrustLevel::Official);
    // 全文件清单复核（解包后落盘状态；防解包环节与流式预验之间的差异）
    let files_verified = match &manifest.files {
        Some(files) => Some({
            verify_dir_files(dest, files)?;
            true
        }),
        None => None,
    };
    Ok(PackageInfo {
        manifest,
        integrity,
        signed: trust == TrustLevel::Official,
        trust,
        files_verified,
    })
}
/// 完整性复核：重算 manifest 字节摘要与落库值比对（激活前可调用）。
pub fn verify_manifest_integrity(manifest_bytes: &[u8], expected: &str) -> Result<(), AppError> {
    let actual = sha256_hex(manifest_bytes);
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(AppError::Validation(
            "插件完整性校验失败：manifest 已被修改，请重新安装该插件".into(),
        ));
    }
    Ok(())
}

/// 全文件清单校验（目录形态）：清单内逐文件 sha256 + 目录文件集合与清单一致。
/// 返回 Err（清单失配即错误，不是「可选信息」）——install / enable 前强制。
pub fn verify_dir_files(dir: &Path, files: &[PluginFileEntry]) -> Result<(), AppError> {
    let mut listed: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for f in files {
        let path = dir.join(&f.path);
        // 符号链接防护：清单内条目一律不允许是链接（防安装后替换指向）
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|_| AppError::Validation(format!("files 清单缺文件：{}", f.path)))?;
        if meta.is_symlink() {
            return Err(AppError::Validation(format!(
                "files 清单条目是符号链接，已拒绝：{}",
                f.path
            )));
        }
        if !meta.is_file() {
            return Err(AppError::Validation(format!(
                "files 清单条目不是常规文件：{}",
                f.path
            )));
        }
        if meta.len() != f.size {
            return Err(AppError::Validation(format!(
                "文件大小与清单不符：{}（实际 {} ≠ 清单 {}）",
                f.path,
                meta.len(),
                f.size
            )));
        }
        let bytes = std::fs::read(&path)
            .map_err(|e| AppError::Io(io_err(&format!("读取 {}", f.path), e)))?;
        let actual = sha256_hex(&bytes);
        if !actual.eq_ignore_ascii_case(&f.sha256) {
            return Err(AppError::Validation(format!(
                "文件内容与清单不符（sha256 失配，可能被篡改）：{}",
                f.path
            )));
        }
        listed.insert(f.path.clone());
    }
    // 目录内不得出现清单之外的文件（防夹带：清单外文件不经完整性保护）；
    // manifest.json 由 integrity 单独保护，不计入集合比对
    let actual_files: Vec<String> = collect_files(dir)?
        .into_iter()
        .filter(|p| p != "manifest.json")
        .collect();
    if actual_files.len() != listed.len() || actual_files.iter().any(|f| !listed.contains(f)) {
        let extra: Vec<&String> = actual_files
            .iter()
            .filter(|f| !listed.contains(*f))
            .take(5)
            .collect();
        return Err(AppError::Validation(format!(
            "插件目录存在 files 清单之外的文件：{}",
            extra
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

/// 流式全文件清单校验（检视阶段，不解包）：对 tar.gz 原文二次遍历，
/// 逐条目 sha256 与清单比对；条目集合（常规文件）必须与清单一致。
fn verify_archive_files(bytes: &[u8], files: &[PluginFileEntry]) -> Result<(), AppError> {
    let expected: std::collections::HashMap<String, &PluginFileEntry> =
        files.iter().map(|f| (f.path.clone(), f)).collect();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let gz = GzDecoder::new(bytes);
    let mut archive = Archive::new(gz);
    for entry in archive
        .entries()
        .map_err(|e| AppError::Validation(format!("插件包读取失败：{e}")))?
    {
        let mut entry =
            entry.map_err(|e| AppError::Validation(format!("插件包条目读取失败：{e}")))?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let raw_path = entry
            .header()
            .path()
            .map_err(|e| AppError::Validation(format!("插件包条目路径非法：{e}")))?
            .to_path_buf();
        // 清单键恒为正斜杠形式——Windows 上 PathBuf 渲染会转反斜杠，
        // 必须归一回 '/' 再比对（否则清单键全部失配，v0.0.61 CI 实测）
        let normalized = raw_path.to_string_lossy().replace('\\', "/");
        let safe = sanitize_entry_path(&normalized)?;
        let path_str = safe.to_string_lossy().replace('\\', "/");
        // manifest.json 不在 files 清单内——由 integrity（manifest 字节 sha256）
        // 单独保护，且是签名 payload 的载体
        if path_str.is_empty() || path_str == "manifest.json" {
            continue;
        }
        let Some(f) = expected.get(path_str.as_str()) else {
            return Err(AppError::Validation(format!(
                "插件包含 files 清单之外的文件：{path_str}"
            )));
        };
        let mut hasher = Sha256::new();
        let mut len: u64 = 0;
        let mut buf = [0u8; 8192];
        loop {
            let n = entry
                .read(&mut buf)
                .map_err(|e| AppError::Validation(format!("插件包条目读取失败：{e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            len += n as u64;
        }
        if len != f.size {
            return Err(AppError::Validation(format!(
                "文件大小与清单不符：{}（实际 {len} ≠ 清单 {}）",
                f.path, f.size
            )));
        }
        let actual = hex::encode(hasher.finalize());
        if !actual.eq_ignore_ascii_case(&f.sha256) {
            return Err(AppError::Validation(format!(
                "文件内容与清单不符（sha256 失配，可能被篡改）：{}",
                f.path
            )));
        }
        seen.insert(path_str);
    }
    if seen.len() != expected.len() {
        let missing: Vec<&String> = expected
            .keys()
            .filter(|p| !seen.contains(*p))
            .take(5)
            .collect();
        return Err(AppError::Validation(format!(
            "插件包缺少 files 清单声明的文件：{}",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Ok(())
}

/// 递归收集目录内全部文件的相对路径（正斜杠形式，排除目录项）。
fn collect_files(dir: &Path) -> Result<Vec<String>, AppError> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), AppError> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| AppError::Io(io_err(&format!("遍历 {}", dir.display()), e)))?;
        for entry in entries {
            let entry = entry.map_err(|e| AppError::Io(io_err("遍历插件目录失败", e)))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let meta = std::fs::symlink_metadata(&path)
                .map_err(|e| AppError::Io(io_err("读取插件文件元数据失败", e)))?;
            if meta.is_symlink() {
                return Err(AppError::Validation(format!(
                    "插件目录含符号链接，已拒绝：{rel}"
                )));
            }
            if meta.is_dir() {
                walk(&path, &rel, out)?;
            } else if meta.is_file() {
                out.push(rel);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(dir, "", &mut out)?;
    out.sort();
    Ok(out)
}

/// ed25519 签名校验（官方 keyring；未声明签名 → Unsigned）。
///
/// payload = [`signed_payload`]（manifest 原文 + files 行清单）。
/// 任一官方公钥命中 → Official；声明了签名但全部失配 → Foreign；
/// hex / key 解析失败同样归 Foreign（绝不把坏签名当通过）。
/// 可注入公钥表的验签核心（生产传官方 keyring；测试传临时密钥做
/// roundtrip / 篡改 / 轮换验证——官方私钥永不进仓库或测试代码）。
pub(crate) fn verify_signature_with_keys(
    manifest: &PluginManifest,
    manifest_bytes: &[u8],
    keys: &[&str],
    matched_trust: TrustLevel,
) -> TrustLevel {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Some(sig_hex) = manifest.signature.as_deref() else {
        return TrustLevel::Unsigned;
    };
    let Ok(payload) = signing_payload(manifest_bytes) else {
        return TrustLevel::Foreign;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return TrustLevel::Foreign;
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return TrustLevel::Foreign;
    };
    for key_hex in keys {
        let Ok(pubkey_bytes) = hex::decode(key_hex) else {
            continue;
        };
        let Ok(bytes) = <[u8; 32]>::try_from(pubkey_bytes.as_slice()) else {
            continue;
        };
        let Ok(vk) = VerifyingKey::from_bytes(&bytes) else {
            continue;
        };
        if vk.verify(&payload, &sig).is_ok() {
            return matched_trust;
        }
    }
    TrustLevel::Foreign
}

/// 条目路径白名单：相对路径、仅 [A-Za-z0-9._- ]、无 `..`、无反斜杠、非空。
fn sanitize_entry_path(raw: &str) -> Result<PathBuf, AppError> {
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') || raw.contains('\\') {
        return Err(AppError::Validation(format!("插件包条目路径非法：{raw}")));
    }
    let mut safe = PathBuf::new();
    for seg in raw.split('/') {
        if seg == ".." {
            return Err(AppError::Validation(format!(
                "插件包条目路径含穿越段：{raw}"
            )));
        }
        // 空段与 "."（tar 目录项 "./"）归一跳过——不改变落盘根。
        if seg.is_empty() || seg == "." {
            continue;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' '))
        {
            return Err(AppError::Validation(format!(
                "插件包条目名含非法字符：{seg}"
            )));
        }
        safe.push(seg);
    }
    Ok(safe)
}

/// 从 tar.gz 字节中提取 manifest.json 原文（不落盘的检视路径）。
fn extract_manifest_bytes(bytes: &[u8]) -> Result<Vec<u8>, AppError> {
    let gz = GzDecoder::new(bytes);
    let mut archive = Archive::new(gz);
    let mut count = 0usize;
    for entry in archive
        .entries()
        .map_err(|e| AppError::Validation(format!("插件包读取失败：{e}")))?
    {
        let mut entry = entry.map_err(|e| AppError::Validation(format!("条目读取失败：{e}")))?;
        count += 1;
        if count > MAX_ENTRIES {
            return Err(AppError::Validation(format!(
                "插件包条目数超过上限（{MAX_ENTRIES}）"
            )));
        }
        let et = entry.header().entry_type();
        if et != tar::EntryType::Regular && et != tar::EntryType::Directory {
            continue;
        }
        let path = entry
            .header()
            .path()
            .map_err(|e| AppError::Validation(format!("条目路径非法：{e}")))?
            .to_path_buf();
        if path == Path::new("manifest.json") {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| AppError::Validation(format!("manifest.json 读取失败：{e}")))?;
            return Ok(buf);
        }
    }
    Err(AppError::Validation("插件包缺少 manifest.json".into()))
}

fn read_file(path: &Path, what: &str) -> Result<Vec<u8>, AppError> {
    std::fs::read(path).map_err(|e| AppError::Io(io_err(&format!("读取 {what} 失败"), e)))
}

/// 带上下文包装 io::Error（AppError::Io 持有 io::Error 而非 String）。
fn io_err(context: &str, e: std::io::Error) -> std::io::Error {
    std::io::Error::other(format!("{context}：{e}"))
}

/// sha256 → 小写 hex。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO_MANIFEST: &str = r#"{
      "id": "com.example.demo",
      "name": "Demo",
      "description": "示例",
      "version": "0.1.0",
      "authors": ["t"],
      "kind": "ui",
      "min_app_version": "0.0.60",
      "sdk_version": 1,
      "permissions": ["std.storage"],
      "entry": "index.html"
    }"#;

    /// 构造内存 tar.gz（entry_name → 内容）。
    fn build_tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, name, std::io::Cursor::new(*content))
                .unwrap();
        }
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn inspect_and_extract_roundtrip() {
        let pkg = build_tar_gz(&[
            ("manifest.json", DEMO_MANIFEST.as_bytes()),
            ("index.html", b"<html></html>"),
        ]);
        let info = inspect_package_bytes(&pkg).unwrap();
        assert_eq!(info.manifest.id, "com.example.demo");
        assert!(!info.signed); // 无签名声明
        assert_eq!(info.integrity, sha256_hex(DEMO_MANIFEST.as_bytes()));

        let tmp = tempfile::tempdir().unwrap();
        let info2 = extract_package_to_dir(&pkg, tmp.path()).unwrap();
        assert_eq!(info2.manifest.id, "com.example.demo");
        assert!(tmp.path().join("index.html").exists());
        assert!(tmp.path().join("manifest.json").exists());

        // 完整性复核：篡改即失败。
        verify_manifest_integrity(DEMO_MANIFEST.as_bytes(), &info.integrity).unwrap();
        let tampered = format!("{DEMO_MANIFEST}<!--x-->");
        assert!(verify_manifest_integrity(tampered.as_bytes(), &info.integrity).is_err());
    }

    /// 手工构造含 `..` 条目的 tar.gz（tar::Builder 会拒绝穿越名，必须裸写 ustar 头）。
    fn build_tar_gz_with_traversal_entry() -> Vec<u8> {
        let content = b"evil";
        let mut header = [0u8; 512];
        let name = b"../escape.txt";
        header[..name.len()].copy_from_slice(name);
        header[100..108].copy_from_slice(b"0000644\0"); // mode
        header[108..116].copy_from_slice(b"0000000\0"); // uid
        header[116..124].copy_from_slice(b"0000000\0"); // gid
        let size_str = format!("{:011o}\0", content.len());
        header[124..136].copy_from_slice(size_str.as_bytes()); // size
        header[156] = b'0'; // typeflag: regular
                            // checksum：校验字段自身按空格计
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|b| *b as u32).sum();
        let chk = format!("{:06o}\0 ", sum);
        header[148..156].copy_from_slice(chk.as_bytes());

        let mut tar_bytes = Vec::new();
        tar_bytes.extend_from_slice(&header);
        tar_bytes.extend_from_slice(content);
        let pad = (512 - content.len() % 512) % 512;
        tar_bytes.extend(std::iter::repeat_n(0u8, pad));
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn rejects_traversal_entry() {
        let pkg = build_tar_gz_with_traversal_entry();
        let tmp = tempfile::tempdir().unwrap();
        let err = extract_package_to_dir(&pkg, tmp.path()).unwrap_err();
        assert!(
            err.user_message().contains("非法") || err.user_message().contains("穿越"),
            "{err}"
        );
        // 穿越目标不存在于插件目录之外。
        assert!(!tmp.path().parent().unwrap().join("escape.txt").exists());
    }

    #[test]
    fn rejects_symlink_entry() {
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_cksum();
        builder
            .append_link(&mut header, "evil-link", "/etc/passwd")
            .unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        let symlink_pkg = gz.finish().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let err = extract_package_to_dir(&symlink_pkg, tmp.path()).unwrap_err();
        assert!(err.user_message().contains("非常规条目"), "{err}");
    }

    #[test]
    fn rejects_missing_manifest() {
        let pkg = build_tar_gz(&[("index.html", b"<html></html>")]);
        let err = inspect_package_bytes(&pkg).unwrap_err();
        assert!(err.user_message().contains("manifest.json"), "{err}");
    }

    #[test]
    fn sanitize_entry_path_rules() {
        assert!(sanitize_entry_path("assets/app.js").is_ok());
        assert!(sanitize_entry_path("ok_name-1.html").is_ok());
        assert!(sanitize_entry_path("/abs/path").is_err());
        assert!(sanitize_entry_path("a/../b").is_err());
        assert!(sanitize_entry_path("a\\b").is_err());
        assert!(sanitize_entry_path("").is_err());
        // tar 目录项（"./"）与双斜杠归一通过；穿越段仍拒绝。
        assert_eq!(sanitize_entry_path("./").unwrap(), PathBuf::new());
        assert_eq!(sanitize_entry_path("a//b").unwrap(), PathBuf::from("a/b"));
        assert_eq!(
            sanitize_entry_path("./sub/x").unwrap(),
            PathBuf::from("sub/x")
        );
    }

    #[test]
    fn accepts_directory_entries() {
        // scripts/pack_plugin.sh 的产物形态：./ 目录条目 + 常规文件。
        let mut builder = tar::Builder::new(Vec::new());
        let mut manifest_header = tar::Header::new_gnu();
        manifest_header.set_size(DEMO_MANIFEST.len() as u64);
        manifest_header.set_mode(0o644);
        manifest_header.set_cksum();
        builder
            .append_data(
                &mut manifest_header,
                "manifest.json",
                std::io::Cursor::new(DEMO_MANIFEST.as_bytes()),
            )
            .unwrap();
        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::Directory);
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        dir_header.set_cksum();
        builder
            .append_data(&mut dir_header, "./", std::io::empty())
            .unwrap();
        let mut file_header = tar::Header::new_gnu();
        let content = b"<html></html>";
        file_header.set_size(content.len() as u64);
        file_header.set_mode(0o644);
        file_header.set_cksum();
        builder
            .append_data(
                &mut file_header,
                "index.html",
                std::io::Cursor::new(content),
            )
            .unwrap();
        let tar_bytes = builder.into_inner().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &tar_bytes).unwrap();
        let pkg_with_dir = gz.finish().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let info = extract_package_to_dir(&pkg_with_dir, tmp.path()).unwrap();
        assert_eq!(info.manifest.id, "com.example.demo");
        assert!(tmp.path().join("index.html").exists());
    }

    /// 跨语言 golden vector（scripts/sign_plugin.mjs selftest 同源互验）：
    /// Node 端规范化实现与本端 [`signing_payload`] 必须产出逐字节一致的
    /// payload——任一侧漂移即 CI 红灯（防签名工具与宿主验证脱节）。
    #[test]
    fn signing_payload_matches_node_golden_vector() {
        const GOLDEN_MANIFEST: &str = r#"{"id":"com.example.golden","name":"Golden","description":"golden","version":"1.0.0","kind":"ui","min_app_version":"0.0.60","sdk_version":1,"permissions":["std.storage"],"files":[{"path":"a.js","sha256":"abababababababababababababababababababababababababababababababab","size":3},{"path":"sub/b.css","sha256":"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd","size":5}]}"#;
        const GOLDEN_PAYLOAD_HEX: &str =
            include_str!("../../../scripts/testdata/golden_payload.hex");
        let payload = signing_payload(GOLDEN_MANIFEST.as_bytes()).unwrap();
        assert_eq!(hex::encode(&payload), GOLDEN_PAYLOAD_HEX.trim());
    }

    #[test]
    fn unsigned_packages_never_report_signed() {
        // 非法签名长度在 manifest 校验拦截。
        let signed_manifest = DEMO_MANIFEST.replace(
            r#""entry": "index.html""#,
            r#""entry": "index.html", "signature": "00aa""#,
        );
        assert!(PluginManifest::parse(signed_manifest.as_bytes()).is_err());
    }

    // ---------------- manifest 2.0：签名 roundtrip / 篡改 / 轮换 ----------------

    use ed25519_dalek::Signer;

    /// 生成临时密钥对（hex 公钥 + SigningKey）。
    fn test_keypair(seed_byte: u8) -> (String, ed25519_dalek::SigningKey) {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[seed_byte; 32]);
        (hex::encode(sk.verifying_key().to_bytes()), sk)
    }

    /// 构造带 files 清单的 manifest JSON（无 signature）+ 对应包内容。
    fn build_files_package() -> (String, Vec<(String, Vec<u8>)>) {
        let html = b"<html><body>demo</body></html>";
        let js = b"console.log('hi')";
        let html_sha = sha256_hex(html);
        let js_sha = sha256_hex(js);
        let manifest = format!(
            r#"{{
              "id": "com.example.demo",
              "name": "Demo",
              "description": "示例插件",
              "version": "0.1.0",
              "authors": ["t"],
              "kind": "ui",
              "min_app_version": "0.0.60",
              "sdk_version": 1,
              "permissions": ["std.storage"],
              "entry": "index.html",
              "files": [
                {{"path": "index.html", "sha256": "{html_sha}", "size": {}}},
                {{"path": "assets/app.js", "sha256": "{js_sha}", "size": {}}}
              ]
            }}"#,
            html.len(),
            js.len()
        );
        (
            manifest,
            vec![
                ("index.html".into(), html.to_vec()),
                ("assets/app.js".into(), js.to_vec()),
            ],
        )
    }

    /// 对 manifest JSON 签名（模拟签名工具流程：无签名原文 → 置零 payload
    /// 计算 → 签名 → 排序键序列化落回 signature 字段）。
    fn sign_manifest(manifest_json: &str, sk: &ed25519_dalek::SigningKey) -> String {
        let payload = signing_payload(manifest_json.as_bytes()).unwrap();
        let sig = sk.sign(&payload);
        let mut value: serde_json::Value = serde_json::from_str(manifest_json).unwrap();
        value.as_object_mut().unwrap().insert(
            "signature".to_string(),
            serde_json::Value::String(hex::encode(sig.to_bytes())),
        );
        serde_json::to_string(&value).unwrap()
    }

    #[test]
    fn signing_payload_is_deterministic_and_hides_signature() {
        let (manifest, _) = build_files_package();
        let p1 = signing_payload(manifest.as_bytes()).unwrap();
        // signature 置零后与原 payload 一致（自引用字段不影响 payload）
        let with_sig = sign_manifest(&manifest, &test_keypair(1).1);
        let p2 = signing_payload(with_sig.as_bytes()).unwrap();
        assert_eq!(p1, p2);
        // 确定性：两次计算结果一致
        assert_eq!(p1, signing_payload(manifest.as_bytes()).unwrap());
    }

    #[test]
    fn signed_package_roundtrip_official_trust() {
        let (manifest, files) = build_files_package();
        let (pub_hex, sk) = test_keypair(42);
        let signed = sign_manifest(&manifest, &sk);

        let pkg = build_tar_gz(&[
            ("manifest.json", signed.as_bytes()),
            ("index.html", files[0].1.as_slice()),
            ("assets/app.js", files[1].1.as_slice()),
        ]);

        // 检视：注入测试公钥 → Official + files 预验通过
        let info = inspect_package_bytes_with_keys(&pkg, &[pub_hex.as_str()]);
        let info = info.unwrap();
        assert_eq!(info.trust, TrustLevel::Official);
        assert!(info.signed);
        assert_eq!(info.files_verified, Some(true));

        // 解包后复核同样通过
        let tmp = tempfile::tempdir().unwrap();
        let extracted =
            extract_package_to_dir_with_keys(&pkg, tmp.path(), &[pub_hex.as_str()]).unwrap();
        assert_eq!(extracted.trust, TrustLevel::Official);
        assert_eq!(extracted.files_verified, Some(true));

        // 目录检视（开发期）一致
        let dir_info = inspect_package_dir_with_keys(tmp.path(), &[pub_hex.as_str()]).unwrap();
        assert_eq!(dir_info.trust, TrustLevel::Official);
        assert_eq!(dir_info.files_verified, Some(true));
    }

    #[test]
    fn tampered_files_detected_at_inspect_and_extract() {
        let (manifest, files) = build_files_package();
        let (pub_hex, sk) = test_keypair(43);
        let signed = sign_manifest(&manifest, &sk);
        let tampered_js = b"console.log('Xx')".to_vec(); // 等长篡改（17B）——命中最深一环：内容摘要

        let pkg = build_tar_gz(&[
            ("manifest.json", signed.as_bytes()),
            ("index.html", files[0].1.as_slice()),
            ("assets/app.js", &tampered_js),
        ]);
        let err = inspect_package_bytes_with_keys(&pkg, &[pub_hex.as_str()]).unwrap_err();
        assert!(err.user_message().contains("sha256 失配"), "{err}");

        let tmp = tempfile::tempdir().unwrap();
        let err =
            extract_package_to_dir_with_keys(&pkg, tmp.path(), &[pub_hex.as_str()]).unwrap_err();
        assert!(err.user_message().contains("sha256 失配"), "{err}");
    }

    #[test]
    fn extra_file_not_in_list_rejected() {
        let (manifest, files) = build_files_package();
        let (pub_hex, sk) = test_keypair(44);
        let signed = sign_manifest(&manifest, &sk);
        let pkg = build_tar_gz(&[
            ("manifest.json", signed.as_bytes()),
            ("index.html", files[0].1.as_slice()),
            ("assets/app.js", files[1].1.as_slice()),
            (" smuggle.js", b"evil".as_slice()),
        ]);
        let err = inspect_package_bytes_with_keys(&pkg, &[pub_hex.as_str()]).unwrap_err();
        assert!(err.user_message().contains("清单之外"), "{err}");
    }

    #[test]
    fn key_rotation_second_key_accepted() {
        let (manifest, files) = build_files_package();
        let (old_pub, old_sk) = test_keypair(45);
        let (new_pub, new_sk) = test_keypair(46);
        let _ = old_sk;
        let signed = sign_manifest(&manifest, &new_sk);
        let pkg = build_tar_gz(&[
            ("manifest.json", signed.as_bytes()),
            ("index.html", files[0].1.as_slice()),
            ("assets/app.js", files[1].1.as_slice()),
        ]);
        // keyring = [旧钥, 新钥]：新钥命中 → Official（轮换期并存）
        let info =
            inspect_package_bytes_with_keys(&pkg, &[old_pub.as_str(), new_pub.as_str()]).unwrap();
        assert_eq!(info.trust, TrustLevel::Official);
        // keyring 只剩旧钥（新钥已退役）：Foreign（不再官方信任）
        let info = inspect_package_bytes_with_keys(&pkg, &[old_pub.as_str()]).unwrap();
        assert_eq!(info.trust, TrustLevel::Foreign);
        assert!(!info.signed);
    }

    #[test]
    fn unsigned_with_files_still_verifies_files() {
        let (manifest, files) = build_files_package();
        let pkg = build_tar_gz(&[
            ("manifest.json", manifest.as_bytes()),
            ("index.html", files[0].1.as_slice()),
            ("assets/app.js", files[1].1.as_slice()),
        ]);
        let info = inspect_package_bytes_with_keys(&pkg, &[]).unwrap();
        assert_eq!(info.trust, TrustLevel::Unsigned);
        assert_eq!(info.files_verified, Some(true));
    }
}
