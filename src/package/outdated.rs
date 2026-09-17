use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

use crate::network::registry::RegistryClient;
use crate::package::lockfile::PackageLock;
use crate::package::manifest::PackageJson;
use crate::package::semver::{SemverSpec, parse_dependency_req_with_catalogs};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OutdatedPackageInfo {
    pub current: String,
    pub wanted: String,
    pub latest: String,
    pub dependent: String,
    pub location: String,
    pub dep_type: String,
}

pub async fn check_outdated(
    project_dir: &Path,
    registry: &RegistryClient,
    package_filter: &[String],
    prod_only: bool,
    dev_only: bool,
) -> Result<BTreeMap<String, OutdatedPackageInfo>> {
    let pkg_json_path = project_dir.join("package.json");
    if !pkg_json_path.exists() {
        return Err(anyhow!("package.json not found in {:?}", project_dir));
    }
    let manifest = PackageJson::from_path(&pkg_json_path)?;

    let lock_path = project_dir.join("package-lock.json");
    let lockfile = if lock_path.exists() {
        Some(PackageLock::from_path(&lock_path)?)
    } else {
        None
    };

    let mut declared_deps: BTreeMap<String, (String, String)> = BTreeMap::new();

    if !dev_only {
        if let Some(deps) = &manifest.dependencies {
            for (pkg, req) in deps {
                declared_deps.insert(pkg.clone(), (req.clone(), "dependencies".to_string()));
            }
        }
        if let Some(opt_deps) = &manifest.optional_dependencies {
            for (pkg, req) in opt_deps {
                declared_deps.insert(
                    pkg.clone(),
                    (req.clone(), "optionalDependencies".to_string()),
                );
            }
        }
    }

    if !prod_only {
        if let Some(dev_deps) = &manifest.dev_dependencies {
            for (pkg, req) in dev_deps {
                declared_deps
                    .entry(pkg.clone())
                    .or_insert_with(|| (req.clone(), "devDependencies".to_string()));
            }
        }
    }

    let filter_set: Option<std::collections::HashSet<&str>> = if package_filter.is_empty() {
        None
    } else {
        Some(package_filter.iter().map(|s| s.as_str()).collect())
    };

    let mut results: BTreeMap<String, OutdatedPackageInfo> = BTreeMap::new();

    for (pkg_name, (raw_req, dep_type)) in declared_deps {
        if let Some(set) = &filter_set {
            if !set.contains(pkg_name.as_str()) {
                continue;
            }
        }

        let parsed_spec =
            parse_dependency_req_with_catalogs(&pkg_name, &raw_req, manifest.catalogs.as_ref());

        if parsed_spec.is_local_file || parsed_spec.is_git || parsed_spec.is_tarball_url {
            continue;
        }

        let reg_name = parsed_spec.registry_name;
        let version_req = parsed_spec.version_req;

        // Current installed version from lockfile or node_modules
        let current_version = if let Some(lock) = &lockfile {
            let top_key = format!("node_modules/{}", pkg_name);
            if let Some(pkg) = lock.packages.get(&top_key) {
                pkg.version.clone().unwrap_or_else(|| "MISSING".to_string())
            } else {
                "MISSING".to_string()
            }
        } else {
            let pkg_mod_json = project_dir
                .join("node_modules")
                .join(&pkg_name)
                .join("package.json");
            if let Ok(installed_manifest) = PackageJson::from_path(&pkg_mod_json) {
                installed_manifest
                    .version
                    .unwrap_or_else(|| "MISSING".to_string())
            } else {
                "MISSING".to_string()
            }
        };

        // Fetch metadata from registry
        let meta = match registry.fetch_package_metadata(&reg_name).await {
            Ok(m) => m,
            Err(e) => {
                tracing::debug!("Failed to fetch metadata for {}: {:#}", reg_name, e);
                continue;
            }
        };

        let latest_tag_ver = meta.dist_tags.get("latest").cloned();
        let all_versions: Vec<&str> = meta.versions.keys().map(|k| k.as_str()).collect();

        let spec = SemverSpec::new(&version_req);
        let wanted_ver = spec
            .select_best(&all_versions)
            .map(|s| s.to_string())
            .or_else(|| latest_tag_ver.clone())
            .unwrap_or_else(|| current_version.clone());

        let latest_ver = latest_tag_ver.unwrap_or_else(|| wanted_ver.clone());

        // Report if current differs from wanted or latest, or current is MISSING
        if current_version == "MISSING"
            || current_version != wanted_ver
            || current_version != latest_ver
        {
            let dep_name = manifest.name.clone().unwrap_or_else(|| "root".to_string());
            results.insert(
                pkg_name.clone(),
                OutdatedPackageInfo {
                    current: current_version,
                    wanted: wanted_ver,
                    latest: latest_ver,
                    dependent: dep_name,
                    location: format!("node_modules/{}", pkg_name),
                    dep_type,
                },
            );
        }
    }

    Ok(results)
}
