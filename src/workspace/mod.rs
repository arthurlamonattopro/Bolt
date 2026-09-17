use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

use crate::package::manifest::PackageJson;

#[derive(Debug, Clone)]
pub struct WorkspaceMember {
    pub name: String,
    pub path: PathBuf,
    pub manifest: PackageJson,
}

/// Find all workspace packages defined in root `package.json` under `"workspaces": [...]`
pub fn discover_workspaces(
    root_dir: &Path,
    root_manifest: &PackageJson,
) -> Result<Vec<WorkspaceMember>> {
    let mut members = Vec::new();

    let Some(workspaces_val) = &root_manifest.workspaces else {
        return Ok(members);
    };

    let patterns: Vec<String> = match workspaces_val {
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::Array(arr)) = map.get("packages") {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    };

    for pattern in patterns {
        let clean_pattern = pattern.trim_end_matches('/');
        if let Some(prefix) = clean_pattern.strip_suffix("/*") {
            let parent = root_dir.join(prefix);
            if parent.is_dir() {
                if let Ok(entries) = fs::read_dir(&parent) {
                    for entry in entries.flatten() {
                        let sub_path = entry.path();
                        if sub_path.is_dir() {
                            let pkg_json = sub_path.join("package.json");
                            if pkg_json.exists() {
                                if let Ok(manifest) = PackageJson::from_path(&pkg_json) {
                                    let name = manifest.name.clone().unwrap_or_else(|| {
                                        sub_path
                                            .file_name()
                                            .unwrap_or_default()
                                            .to_string_lossy()
                                            .to_string()
                                    });
                                    members.push(WorkspaceMember {
                                        name,
                                        path: sub_path,
                                        manifest,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        } else {
            let member_path = root_dir.join(clean_pattern);
            let pkg_json = member_path.join("package.json");
            if pkg_json.exists() {
                if let Ok(manifest) = PackageJson::from_path(&pkg_json) {
                    let name = manifest.name.clone().unwrap_or_else(|| {
                        member_path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string()
                    });
                    members.push(WorkspaceMember {
                        name,
                        path: member_path,
                        manifest,
                    });
                }
            }
        }
    }

    Ok(members)
}

/// Link workspace packages into root `node_modules` (or between workspace packages)
pub fn link_workspaces(root_dir: &Path, members: &[WorkspaceMember]) -> Result<()> {
    let node_modules = root_dir.join("node_modules");
    fs::create_dir_all(&node_modules)?;

    for member in members {
        let dest = if let Some(stripped) = member.name.strip_prefix('@') {
            if let Some((scope, name)) = stripped.split_once('/') {
                let scope_dir = node_modules.join(format!("@{}", scope));
                fs::create_dir_all(&scope_dir)?;
                scope_dir.join(name)
            } else {
                node_modules.join(&member.name)
            }
        } else {
            node_modules.join(&member.name)
        };

        // If existing link or directory, remove it first
        if dest.exists() || dest.is_symlink() {
            let _ = fs::remove_dir_all(&dest);
            let _ = fs::remove_file(&dest);
        }

        // Create symlink or junction on Windows
        create_dir_link(&member.path, &dest)
            .with_context(|| format!("Failed to link workspace {} to {:?}", member.name, dest))?;
    }

    Ok(())
}

/// Find a specific workspace member by name or relative directory path
pub fn find_workspace_member<'a>(
    members: &'a [WorkspaceMember],
    target: &str,
) -> Option<&'a WorkspaceMember> {
    members.iter().find(|m| {
        m.name == target
            || m.path
                .to_string_lossy()
                .replace('\\', "/")
                .ends_with(target.trim_end_matches('/'))
    })
}

fn create_dir_link(src: &Path, dest: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(src, dest)
            .or_else(|_| {
                let mut cmd = std::process::Command::new("cmd");
                cmd.arg("/c").arg("mklink").arg("/j").arg(dest).arg(src);
                cmd.output().map(|_| ())
            })
            .context("Failed to create Windows junction/symlink")?;
    }

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(src, dest).context("Failed to create Unix symlink")?;
    }

    Ok(())
}
