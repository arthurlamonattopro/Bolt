use anyhow::{Context, Result, anyhow};
use base64::Engine;
use flate2::read::GzDecoder;
use sha2::Digest;
use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use tar::Archive;

/// Verify integrity string (e.g. `sha512-...` or `sha1-...`)
pub fn verify_integrity(bytes: &[u8], integrity_str: &str) -> Result<()> {
    let integrity_str = integrity_str.trim();
    if integrity_str.is_empty() {
        return Ok(());
    }

    if let Some(hash_val) = integrity_str.strip_prefix("sha512-") {
        let mut hasher = sha2::Sha512::new();
        hasher.update(bytes);
        let actual_hash = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
        if actual_hash != hash_val {
            return Err(anyhow!(
                "Integrity check failed! Expected sha512-{}, got sha512-{}",
                hash_val,
                actual_hash
            ));
        }
        Ok(())
    } else if let Some(hash_val) = integrity_str.strip_prefix("sha1-") {
        // Sha1 could be base64 or hex
        let mut hasher = sha1::Sha1::new();
        hasher.update(bytes);
        let digest = hasher.finalize();
        let actual_b64 = base64::engine::general_purpose::STANDARD.encode(digest);
        let actual_hex = hex::encode(digest);

        if actual_b64 == hash_val || actual_hex == hash_val {
            Ok(())
        } else {
            Err(anyhow!(
                "Integrity check failed! Expected sha1-{}, got sha1-{}",
                hash_val,
                actual_b64
            ))
        }
    } else {
        // Unknown integrity format, pass with warning
        Ok(())
    }
}

/// Extract npm tarball (usually has root folder `package/...`) safely into target_dir.
/// Guarantees:
/// - Rejects directory traversal paths (`..`)
/// - Rejects absolute paths
/// - Strips standard leading `package/` folder so files land directly in target_dir
pub fn extract_tarball_safe(tarball_bytes: &[u8], target_dir: &Path) -> Result<()> {
    let cursor = Cursor::new(tarball_bytes);
    let gz = GzDecoder::new(cursor);
    let mut archive = Archive::new(gz);

    // Create target dir if not exists
    fs::create_dir_all(target_dir)
        .with_context(|| format!("Failed to create directory {:?}", target_dir))?;

    for entry_result in archive
        .entries()
        .context("Failed to read tar archive entries")?
    {
        let mut entry = entry_result.context("Failed to read tar entry")?;
        let path = entry.path().context("Invalid path in tar entry")?;

        // Validate path security: no `..` or leading `/`
        for comp in path.components() {
            match comp {
                Component::ParentDir => {
                    return Err(anyhow!("Unsafe archive entry with '..': {:?}", path));
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(anyhow!(
                        "Unsafe archive entry with absolute path: {:?}",
                        path
                    ));
                }
                _ => {}
            }
        }

        // Standard npm tarballs put everything inside `package/` folder. Strip it!
        let stripped_path: PathBuf = if path.starts_with("package") {
            path.iter().skip(1).collect()
        } else {
            path.to_path_buf()
        };

        if stripped_path.as_os_str().is_empty() {
            continue;
        }

        let dest = target_dir.join(stripped_path);

        if entry.header().entry_type().is_dir() {
            fs::create_dir_all(&dest)?;
        } else {
            if let Some(parent) = dest.parent() {
                fs::create_dir_all(parent)?;
            }
            entry
                .unpack(&dest)
                .with_context(|| format!("Failed to unpack to {:?}", dest))?;

            // On Unix, preserve executable permissions if possible
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(mode) = entry.header().mode() {
                    let _ = fs::set_permissions(&dest, fs::Permissions::from_mode(mode));
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verify_integrity_sha512() {
        let data = b"hello world";
        let mut hasher = sha2::Sha512::new();
        hasher.update(data);
        let b64 = base64::engine::general_purpose::STANDARD.encode(hasher.finalize());
        let integrity = format!("sha512-{}", b64);

        assert!(verify_integrity(data, &integrity).is_ok());
        assert!(verify_integrity(b"wrong data", &integrity).is_err());
    }
}
