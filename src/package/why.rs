use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use crate::package::lockfile::PackageLock;
use crate::package::manifest::PackageJson;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WhyReason {
    pub location: String,
    pub version: String,
    pub required_by: String,
    pub requirement: String,
    pub dep_type: String,
    pub chain: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WhyResult {
    pub package: String,
    pub reasons: Vec<WhyReason>,
}

pub fn explain_package(project_dir: &Path, target_pkg: &str) -> Result<WhyResult> {
    let pkg_json_path = project_dir.join("package.json");
    if !pkg_json_path.exists() {
        return Err(anyhow!("package.json not found in {:?}", project_dir));
    }
    let manifest = PackageJson::from_path(&pkg_json_path)?;
    let root_name = manifest.name.clone().unwrap_or_else(|| "root".to_string());

    let lock_path = project_dir.join("package-lock.json");
    if !lock_path.exists() {
        return Err(anyhow!(
            "package-lock.json not found in {:?}. Run bolt install first.",
            project_dir
        ));
    }
    let lockfile = PackageLock::from_path(&lock_path)?;

    let mut reasons = Vec::new();

    // 1. Direct dependencies from root manifest
    let mut check_root_dep = |dep_name: &str, req: &str, dep_type: &str| {
        if dep_name == target_pkg {
            let top_key = format!("node_modules/{}", target_pkg);
            let version = lockfile
                .packages
                .get(&top_key)
                .and_then(|p| p.version.clone())
                .unwrap_or_else(|| req.to_string());

            reasons.push(WhyReason {
                location: top_key,
                version,
                required_by: format!(
                    "{}@{}",
                    root_name,
                    manifest.version.as_deref().unwrap_or("")
                ),
                requirement: req.to_string(),
                dep_type: dep_type.to_string(),
                chain: vec![
                    format!("{} ({})", root_name, dep_type),
                    target_pkg.to_string(),
                ],
            });
        }
    };

    if let Some(deps) = &manifest.dependencies {
        for (pkg, req) in deps {
            check_root_dep(pkg, req, "dependencies");
        }
    }
    if let Some(dev_deps) = &manifest.dev_dependencies {
        for (pkg, req) in dev_deps {
            check_root_dep(pkg, req, "devDependencies");
        }
    }
    if let Some(opt_deps) = &manifest.optional_dependencies {
        for (pkg, req) in opt_deps {
            check_root_dep(pkg, req, "optionalDependencies");
        }
    }
    if let Some(peer_deps) = &manifest.peer_dependencies {
        for (pkg, req) in peer_deps {
            check_root_dep(pkg, req, "peerDependencies");
        }
    }

    // 2. Transitive dependencies found across all lockfile entries
    for (pkg_path, lock_pkg) in &lockfile.packages {
        if pkg_path.is_empty() {
            continue;
        }

        let parent_name = if let Some(n) = &lock_pkg.name {
            n.clone()
        } else if let Some(stripped) = pkg_path.strip_prefix("node_modules/") {
            stripped
                .split("/node_modules/")
                .last()
                .unwrap_or(stripped)
                .to_string()
        } else {
            pkg_path.clone()
        };

        let parent_version = lock_pkg.version.as_deref().unwrap_or("");

        let mut check_transitive = |deps_map: &Option<BTreeMap<String, String>>, dep_type: &str| {
            if let Some(deps) = deps_map {
                if let Some(req) = deps.get(target_pkg) {
                    let mut nested_key = format!("{}/node_modules/{}", pkg_path, target_pkg);
                    let mut installed_ver = None;
                    if let Some(child) = lockfile.packages.get(&nested_key) {
                        installed_ver = child.version.clone();
                    } else {
                        let top_key = format!("node_modules/{}", target_pkg);
                        if let Some(top) = lockfile.packages.get(&top_key) {
                            nested_key = top_key;
                            installed_ver = top.version.clone();
                        }
                    }

                    let version = installed_ver.unwrap_or_else(|| req.clone());

                    // Build path chain
                    let segments: Vec<&str> = pkg_path
                        .strip_prefix("node_modules/")
                        .unwrap_or(pkg_path)
                        .split("/node_modules/")
                        .collect();

                    let mut chain = vec![root_name.clone()];
                    for seg in segments {
                        chain.push(seg.to_string());
                    }
                    chain.push(target_pkg.to_string());

                    reasons.push(WhyReason {
                        location: nested_key,
                        version,
                        required_by: format!("{}@{}", parent_name, parent_version),
                        requirement: req.clone(),
                        dep_type: dep_type.to_string(),
                        chain,
                    });
                }
            }
        };

        check_transitive(&lock_pkg.dependencies, "dependencies");
        check_transitive(&lock_pkg.optional_dependencies, "optionalDependencies");
        check_transitive(&lock_pkg.peer_dependencies, "peerDependencies");
    }

    // Deduplicate identical reasons
    let mut seen = HashSet::new();
    reasons.retain(|r| {
        let key = (
            r.location.clone(),
            r.required_by.clone(),
            r.requirement.clone(),
        );
        seen.insert(key)
    });

    Ok(WhyResult {
        package: target_pkg.to_string(),
        reasons,
    })
}
