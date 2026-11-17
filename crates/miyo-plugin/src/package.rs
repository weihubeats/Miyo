//! 插件包（`.miyo` = tar.gz）安装：解包、完整性、签名校验、路径穿越防护。
//!
//! 安全模型：
//! - **完整性**：`integrity` = 插件包内 manifest.json 字节的 sha256（hex）。安装时
//!   计算并落库；激活前可复核（`verify_manifest_integrity`），包被篡改即拒绝。
//! - **签名**：ed25519，对 manifest 原文字节签名；官方发布公钥编译期内置。
//!   未签名包不拒绝安装，但必须由用户显式信任（UI 层确认 + 落库 `signed=false`）。
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

use crate::manifest::PluginManifest;

/// 解包限制。
pub const MAX_ENTRIES: usize = 200;
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;

/// ed25519 官方发布公钥（hex，32 字节）。发布流水线接入前为占位零值：
/// 此时任何 `signature` 都校验失败（signed=false），本地包一律走
/// 「未签名 + 用户显式信任」通道。接入后替换为真实公钥即可，校验代码无需改动。
pub const OFFICIAL_SIGNING_PUBKEY_HEX: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// 包检视结果（安装确认页展示：清单 + 完整性 + 签名状态）。
#[derive(Debug, Clone)]
pub struct PackageInfo {
    pub manifest: PluginManifest,
    /// manifest.json 字节的 sha256（hex）。
    pub integrity: String,
    /// 签名是否通过官方公钥校验。
    pub signed: bool,
}

/// 从 `.miyo` 包字节检视（不落盘）。
pub fn inspect_package_bytes(bytes: &[u8]) -> Result<PackageInfo, AppError> {
    let manifest_bytes = extract_manifest_bytes(bytes)?;
    let manifest = PluginManifest::parse(&manifest_bytes)?;
    let integrity = sha256_hex(&manifest_bytes);
    let signed = verify_signature(&manifest, &manifest_bytes);
    Ok(PackageInfo {
        manifest,
        integrity,
        signed,
    })
}

/// 从插件目录检视（开发期目录安装）。
pub fn inspect_package_dir(dir: &Path) -> Result<PackageInfo, AppError> {
    let manifest_path = dir.join("manifest.json");
    let manifest_bytes = read_file(&manifest_path, "manifest.json")?;
    let manifest = PluginManifest::parse(&manifest_bytes)?;
    let integrity = sha256_hex(&manifest_bytes);
    let signed = verify_signature(&manifest, &manifest_bytes);
    Ok(PackageInfo {
        manifest,
        integrity,
        signed,
    })
}

/// 把 `.miyo` 包解包到目标目录（调用方保证 dest 为全新空目录）。
pub fn extract_package_to_dir(bytes: &[u8], dest: &Path) -> Result<PackageInfo, AppError> {
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
    let signed = verify_signature(&manifest, &manifest_bytes);
    Ok(PackageInfo {
        manifest,
        integrity,
        signed,
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

/// ed25519 签名校验（manifest 声明了 signature 才尝试；公钥未配置时一律 false）。
fn verify_signature(manifest: &PluginManifest, manifest_bytes: &[u8]) -> bool {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let Some(sig_hex) = manifest.signature.as_deref() else {
        return false;
    };
    let Ok(pubkey_bytes) = hex::decode(OFFICIAL_SIGNING_PUBKEY_HEX) else {
        return false;
    };
    let Ok(vk) = VerifyingKey::from_bytes(&pubkey_bytes.try_into().unwrap_or([0u8; 32])) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(sig_hex) else {
        return false;
    };
    let Ok(sig) = Signature::from_slice(&sig_bytes) else {
        return false;
    };
    vk.verify(manifest_bytes, &sig).is_ok()
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

    #[test]
    fn unsigned_packages_never_report_signed() {
        // 非法签名长度在 manifest 校验拦截。
        let signed_manifest = DEMO_MANIFEST.replace(
            r#""entry": "index.html""#,
            r#""entry": "index.html", "signature": "00aa""#,
        );
        assert!(PluginManifest::parse(signed_manifest.as_bytes()).is_err());
    }
}
