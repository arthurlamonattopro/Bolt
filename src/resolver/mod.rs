use anyhow::{Result, anyhow};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::PathBuf;
use tracing::warn;

use crate::network::git::fetch_git_repo;
use crate::network::registry::{RegistryClient, RegistryVersionMetadata};
use crate::package::lockfile::{LockPackage, PackageLock};
use crate::package::manifest::PackageJson;
use crate::package::platform::{is_cpu_supported, is_os_supported};
use crate::package::semver::{SemverSpec, normalize_git_url, parse_dependency_req_with_catalogs};
/// Represents a resolved package node in the dependency graph
#[derive(Debug, Clone)]
pub struct ResolvedNode {
    pub name: String,
    pub version: String,
    pub resolved_url: String,
    pub integrity: String,
    pub dev: bool,
    pub optional: bool,
    pub peer: bool,
    pub is_local_file: bool,
    pub local_source_path: Option<PathBuf>,
    pub dependencies: BTreeMap<String, String>,
    pub bin: Option<serde_json::Value>,
    pub has_install_script: bool,
    pub engines: Option<BTreeMap<String, String>>,
    pub os: Option<Vec<String>>,
    pub cpu: Option<Vec<String>>,
    pub install_path: String,
}

pub struct Resolver {
    registry: RegistryClient,
    existing_lock: Option<PackageLock>,
    project_dir: PathBuf,
    overrides: BTreeMap<String, serde_json::Value>,
    catalogs: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(Debug, Clone)]
struct QueueItem {
    pub dep_key: String,
    pub req: String,
    pub is_dev: bool,
    pub is_optional: bool,
    pub is_peer: bool,
    pub parent_path: String,
    pub cycle_path: Vec<String>,
}

impl Resolver {
    pub fn new(
        project_dir: PathBuf,
        registry: RegistryClient,
        existing_lock: Option<PackageLock>,
    ) -> Self {
        Self {
            project_dir,
            registry,
            existing_lock,
            overrides: BTreeMap::new(),
            catalogs: BTreeMap::new(),
        }
    }

    pub fn with_overrides(mut self, overrides: BTreeMap<String, serde_json::Value>) -> Self {
        self.overrides = overrides;
        self
    }

    pub fn with_catalogs(mut self, catalogs: BTreeMap<String, BTreeMap<String, String>>) -> Self {
        self.catalogs = catalogs;
        self
    }

    /// Resolve all dependencies declared in `manifest`
    pub async fn resolve_manifest(
        &self,
        manifest: &PackageJson,
    ) -> Result<BTreeMap<String, ResolvedNode>> {
        self.resolve_manifests(std::slice::from_ref(manifest)).await
    }

    /// Resolve dependencies across the root manifest and all workspace member manifests,
    /// hoisting common compatible dependencies to root `node_modules` and nesting conflicts.
    pub async fn resolve_manifests(
        &self,
        manifests: &[PackageJson],
    ) -> Result<BTreeMap<String, ResolvedNode>> {
        // Merge any root catalogs and overrides if not already set
        let mut effective_catalogs = self.catalogs.clone();
        let mut effective_overrides = self.overrides.clone();

        for manifest in manifests {
            if let Some(cats) = &manifest.catalogs {
                for (cat_name, cat_entries) in cats {
                    let entry = effective_catalogs.entry(cat_name.clone()).or_default();
                    for (k, v) in cat_entries {
                        entry.insert(k.clone(), v.clone());
                    }
                }
            }
            if let Some(ov) = &manifest.overrides {
                for (k, v) in ov {
                    effective_overrides.insert(k.clone(), v.clone());
                }
            }
            if let Some(res) = &manifest.resolutions {
                for (k, v) in res {
                    effective_overrides.insert(k.clone(), v.clone());
                }
            }
        }

        let mut queue: VecDeque<QueueItem> = VecDeque::new();
        for manifest in manifests {
            if let Some(deps) = &manifest.dependencies {
                for (name, req) in deps {
                    queue.push_back(QueueItem {
                        dep_key: name.clone(),
                        req: req.clone(),
                        is_dev: false,
                        is_optional: false,
                        is_peer: false,
                        parent_path: "".to_string(),
                        cycle_path: vec!["root".to_string()],
                    });
                }
            }
            if let Some(dev_deps) = &manifest.dev_dependencies {
                for (name, req) in dev_deps {
                    queue.push_back(QueueItem {
                        dep_key: name.clone(),
                        req: req.clone(),
                        is_dev: true,
                        is_optional: false,
                        is_peer: false,
                        parent_path: "".to_string(),
                        cycle_path: vec!["root".to_string()],
                    });
                }
            }
            if let Some(opt_deps) = &manifest.optional_dependencies {
                for (name, req) in opt_deps {
                    queue.push_back(QueueItem {
                        dep_key: name.clone(),
                        req: req.clone(),
                        is_dev: false,
                        is_optional: true,
                        is_peer: false,
                        parent_path: "".to_string(),
                        cycle_path: vec!["root".to_string()],
                    });
                }
            }
            if let Some(peer_deps) = &manifest.peer_dependencies {
                for (name, req) in peer_deps {
                    queue.push_back(QueueItem {
                        dep_key: name.clone(),
                        req: req.clone(),
                        is_dev: false,
                        is_optional: false,
                        is_peer: true,
                        parent_path: "".to_string(),
                        cycle_path: vec!["root".to_string()],
                    });
                }
            }
        }

        self.resolve_queue_with_config(queue, &effective_overrides, &effective_catalogs)
            .await
    }

    async fn resolve_queue_with_config(
        &self,
        mut queue: VecDeque<QueueItem>,
        overrides: &BTreeMap<String, serde_json::Value>,
        catalogs: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<BTreeMap<String, ResolvedNode>> {
        let mut installed: BTreeMap<String, ResolvedNode> = BTreeMap::new();
        let mut visited: HashSet<(String, String)> = HashSet::new();

        while let Some(mut item) = queue.pop_front() {
            // Check overrides/resolutions for item.dep_key
            if let Some(override_val) = overrides.get(&item.dep_key) {
                if let Some(override_req) = override_val.as_str() {
                    item.req = override_req.to_string();
                } else if let Some(override_obj) = override_val.as_object() {
                    // scoped override e.g. "bar": { ".": "^2.0.0" } or parent matching
                    if let Some(dot_val) = override_obj.get(".") {
                        if let Some(dot_req) = dot_val.as_str() {
                            item.req = dot_req.to_string();
                        }
                    }
                }
            }
            // Also check parent scoped overrides if parent_path is not empty: e.g. "foo": { "bar": "^2.0.0" }
            if !item.parent_path.is_empty() {
                let parent_folder = item.parent_path.split('/').next_back().unwrap_or("");
                if let Some(parent_override) = overrides.get(parent_folder) {
                    if let Some(parent_obj) = parent_override.as_object() {
                        if let Some(dep_override) = parent_obj.get(&item.dep_key) {
                            if let Some(dep_req) = dep_override.as_str() {
                                item.req = dep_req.to_string();
                            }
                        }
                    }
                }
            }

            let dep_spec =
                parse_dependency_req_with_catalogs(&item.dep_key, &item.req, Some(catalogs));
            let folder_name = item.dep_key.clone();
            let top_level_path = format!("node_modules/{}", folder_name);
            // Handle local file dependencies (e.g. `file:../local-pkg`)
            if dep_spec.is_local_file {
                let local_path = self.project_dir.join(&dep_spec.version_req);
                let local_pkg_json = local_path.join("package.json");
                if local_pkg_json.exists() {
                    let local_manifest = PackageJson::from_path(&local_pkg_json)?;
                    let node = ResolvedNode {
                        name: local_manifest.name.unwrap_or(item.dep_key.clone()),
                        version: local_manifest
                            .version
                            .unwrap_or_else(|| "1.0.0".to_string()),
                        resolved_url: format!("file:{}", dep_spec.version_req),
                        integrity: "".to_string(),
                        dev: item.is_dev,
                        optional: item.is_optional,
                        peer: item.is_peer,
                        is_local_file: true,
                        local_source_path: Some(local_path),
                        dependencies: local_manifest.dependencies.unwrap_or_default(),
                        bin: local_manifest.bin,
                        has_install_script: false,
                        engines: None,
                        os: None,
                        cpu: None,
                        install_path: top_level_path.clone(),
                    };
                    installed.insert(top_level_path, node);
                    continue;
                } else {
                    return Err(anyhow!(
                        "Local package path does not exist: {:?}",
                        local_path
                    ));
                }
            }

            // Handle git dependencies (e.g. `github:user/repo`, `git+https://...`)
            if dep_spec.is_git {
                let (norm_url, committish) = normalize_git_url(&dep_spec.version_req);
                let (repo_path, commit_hash) =
                    fetch_git_repo(self.registry.cache_dir(), &norm_url, committish.as_deref())?;

                let git_pkg_json = repo_path.join("package.json");
                let git_manifest = if git_pkg_json.exists() {
                    PackageJson::from_path(&git_pkg_json)?
                } else {
                    PackageJson {
                        name: Some(item.dep_key.clone()),
                        version: Some("1.0.0".to_string()),
                        ..Default::default()
                    }
                };

                let node = ResolvedNode {
                    name: git_manifest.name.unwrap_or(item.dep_key.clone()),
                    version: git_manifest.version.unwrap_or_else(|| "1.0.0".to_string()),
                    resolved_url: format!("git+{}#{}", norm_url, commit_hash),
                    integrity: "".to_string(),
                    dev: item.is_dev,
                    optional: item.is_optional,
                    peer: item.is_peer,
                    is_local_file: true,
                    local_source_path: Some(repo_path),
                    dependencies: git_manifest.dependencies.unwrap_or_default(),
                    bin: git_manifest.bin,
                    has_install_script: false,
                    engines: None,
                    os: None,
                    cpu: None,
                    install_path: top_level_path.clone(),
                };
                installed.insert(top_level_path, node);
                continue;
            }

            // Handle tarball URL dependencies (e.g. `https://example.com/pkg.tgz`)
            if dep_spec.is_tarball_url {
                let tarball_bytes = self.registry.fetch_tarball(&dep_spec.version_req).await?;
                // Inspect package.json inside tarball
                let mut tarball_manifest = PackageJson::default();
                let cursor = std::io::Cursor::new(&tarball_bytes);
                let gz = flate2::read::GzDecoder::new(cursor);
                let mut archive = tar::Archive::new(gz);
                if let Ok(entries) = archive.entries() {
                    for entry in entries.flatten() {
                        if let Ok(path) = entry.path() {
                            let path_str = path.to_string_lossy();
                            if path_str == "package/package.json" || path_str == "package.json" {
                                use std::io::Read;
                                let mut reader = entry;
                                let mut s = String::new();
                                if reader.read_to_string(&mut s).is_ok() {
                                    if let Ok(m) = PackageJson::from_str(&s) {
                                        tarball_manifest = m;
                                    }
                                }
                                break;
                            }
                        }
                    }
                }

                let mut hasher = sha2::Sha512::new();
                use sha2::Digest;
                hasher.update(&tarball_bytes);
                let sha_b64 = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    hasher.finalize(),
                );
                let integrity = format!("sha512-{}", sha_b64);

                let node = ResolvedNode {
                    name: tarball_manifest.name.unwrap_or(item.dep_key.clone()),
                    version: tarball_manifest
                        .version
                        .unwrap_or_else(|| "1.0.0".to_string()),
                    resolved_url: dep_spec.version_req.clone(),
                    integrity,
                    dev: item.is_dev,
                    optional: item.is_optional,
                    peer: item.is_peer,
                    is_local_file: false,
                    local_source_path: None,
                    dependencies: tarball_manifest.dependencies.unwrap_or_default(),
                    bin: tarball_manifest.bin,
                    has_install_script: false,
                    engines: None,
                    os: None,
                    cpu: None,
                    install_path: top_level_path.clone(),
                };
                installed.insert(top_level_path, node);
                continue;
            }
            let pkg_name = dep_spec.registry_name;
            let req = dep_spec.version_req;

            let mut resolved_version_meta: Option<RegistryVersionMetadata> = None;
            let mut matched_version: Option<String> = None;

            // Check existing lockfile at prospective nested path or top_level
            if let Some(lock) = &self.existing_lock {
                let check_path = if item.parent_path.is_empty() {
                    top_level_path.clone()
                } else {
                    format!("{}/node_modules/{}", item.parent_path, folder_name)
                };
                if let Some(lock_pkg) = lock
                    .packages
                    .get(&check_path)
                    .or_else(|| lock.packages.get(&top_level_path))
                {
                    if let Some(v) = &lock_pkg.version {
                        #[allow(clippy::collapsible_if)]
                        if SemverSpec::new(&req).matches(v) {
                            matched_version = Some(v.clone());
                        }
                    }
                }
            }
            // Query registry if needed
            if matched_version.is_none() {
                let pkg_meta_res = self.registry.fetch_package_metadata(&pkg_name).await;
                let pkg_meta = match pkg_meta_res {
                    Ok(m) => m,
                    Err(e) => {
                        if item.is_optional {
                            warn!("Optional dep {} fetch failed: {}. Skipping.", pkg_name, e);
                            continue;
                        } else {
                            return Err(e);
                        }
                    }
                };

                let spec = SemverSpec::new(&req);
                let target_version = if let Some(tag_ver) = pkg_meta.dist_tags.get(&req) {
                    Some(tag_ver.as_str())
                } else if req == "latest" || req == "*" || req.is_empty() {
                    pkg_meta.dist_tags.get("latest").map(|s| s.as_str())
                } else {
                    let versions: Vec<&str> =
                        pkg_meta.versions.keys().map(|s| s.as_str()).collect();
                    spec.select_best(&versions)
                };

                let best_ver = match target_version {
                    Some(v) => v,
                    None => {
                        if item.is_optional {
                            continue;
                        } else {
                            return Err(anyhow!(
                                "No matching version for {} with range {}",
                                pkg_name,
                                req
                            ));
                        }
                    }
                };

                matched_version = Some(best_ver.to_string());
                if let Some(ver_meta) = pkg_meta.versions.get(best_ver) {
                    resolved_version_meta = Some(ver_meta.clone());
                }
            }

            let version = matched_version.unwrap();

            if resolved_version_meta.is_none() {
                let pkg_meta_res = self.registry.fetch_package_metadata(&pkg_name).await;
                let pkg_meta = match pkg_meta_res {
                    Ok(m) => m,
                    Err(e) => {
                        if item.is_optional {
                            continue;
                        } else {
                            return Err(e);
                        }
                    }
                };

                if let Some(vm) = pkg_meta.versions.get(&version) {
                    resolved_version_meta = Some(vm.clone());
                } else if item.is_optional {
                    continue;
                } else {
                    return Err(anyhow!(
                        "Version {} of {} not found in metadata",
                        version,
                        pkg_name
                    ));
                }
            }

            let ver_meta = resolved_version_meta.unwrap();

            if !is_os_supported(&ver_meta.os) || !is_cpu_supported(&ver_meta.cpu) {
                if item.is_optional {
                    continue;
                } else {
                    warn!(
                        "Package {}@{} does not match host platform",
                        pkg_name, version
                    );
                }
            }

            let install_path = if let Some(existing) = installed.get(&top_level_path) {
                if existing.version == version {
                    continue;
                } else if item.parent_path.is_empty() {
                    top_level_path.clone()
                } else {
                    format!("{}/node_modules/{}", item.parent_path, folder_name)
                }
            } else {
                top_level_path.clone()
            };

            let cycle_key = format!("{}@{}", pkg_name, version);
            if item.cycle_path.contains(&cycle_key) {
                continue;
            }

            let mut next_cycle_path = item.cycle_path.clone();
            next_cycle_path.push(cycle_key);

            let node_key = (install_path.clone(), version.clone());
            let is_new = visited.insert(node_key);

            let has_install_script = if let Some(scripts) = &ver_meta.scripts {
                scripts.contains_key("preinstall")
                    || scripts.contains_key("install")
                    || scripts.contains_key("postinstall")
            } else {
                false
            };

            let node = ResolvedNode {
                name: pkg_name.clone(),
                version: version.clone(),
                resolved_url: ver_meta.dist.tarball.clone(),
                integrity: ver_meta.dist.integrity.clone().unwrap_or_else(|| {
                    ver_meta
                        .dist
                        .shasum
                        .as_ref()
                        .map(|s| format!("sha1-{}", s))
                        .unwrap_or_default()
                }),
                dev: item.is_dev,
                optional: item.is_optional,
                peer: item.is_peer,
                is_local_file: false,
                local_source_path: None,
                dependencies: ver_meta.dependencies.clone().unwrap_or_default(),
                bin: ver_meta.bin.clone(),
                has_install_script,
                engines: ver_meta.engines.clone(),
                os: ver_meta.os.clone(),
                cpu: ver_meta.cpu.clone(),
                install_path: install_path.clone(),
            };

            installed.insert(install_path.clone(), node);

            if is_new {
                if let Some(deps) = &ver_meta.dependencies {
                    for (dep_name, dep_req) in deps {
                        queue.push_back(QueueItem {
                            dep_key: dep_name.clone(),
                            req: dep_req.clone(),
                            is_dev: item.is_dev,
                            is_optional: item.is_optional,
                            is_peer: false,
                            parent_path: install_path.clone(),
                            cycle_path: next_cycle_path.clone(),
                        });
                    }
                }
                if let Some(opt_deps) = &ver_meta.optional_dependencies {
                    for (dep_name, dep_req) in opt_deps {
                        queue.push_back(QueueItem {
                            dep_key: dep_name.clone(),
                            req: dep_req.clone(),
                            is_dev: item.is_dev,
                            is_optional: true,
                            is_peer: false,
                            parent_path: install_path.clone(),
                            cycle_path: next_cycle_path.clone(),
                        });
                    }
                }
                if let Some(peer_deps) = &ver_meta.peer_dependencies {
                    let peer_meta = ver_meta.peer_dependencies_meta.as_ref();
                    for (dep_name, dep_req) in peer_deps {
                        let is_peer_optional = peer_meta
                            .and_then(|pm| pm.get(dep_name))
                            .and_then(|obj| obj.get("optional"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);

                        // If an existing installed package at top_level or in ancestry satisfies this peer dep, we don't need to reinstall/duplicate
                        let top_path = format!("node_modules/{}", dep_name);
                        let already_satisfied =
                            if let Some(existing_node) = installed.get(&top_path) {
                                SemverSpec::new(dep_req).matches(&existing_node.version)
                            } else {
                                false
                            };

                        if !already_satisfied {
                            queue.push_back(QueueItem {
                                dep_key: dep_name.clone(),
                                req: dep_req.clone(),
                                is_dev: item.is_dev,
                                is_optional: is_peer_optional,
                                is_peer: true,
                                parent_path: install_path.clone(),
                                cycle_path: next_cycle_path.clone(),
                            });
                        }
                    }
                }
            }
        }

        Ok(installed)
    }

    /// Build a v3 PackageLock from resolved nodes and root manifest
    pub fn build_lockfile(
        manifest: &PackageJson,
        resolved: &BTreeMap<String, ResolvedNode>,
    ) -> PackageLock {
        let root_name = manifest.name.clone().unwrap_or_else(|| "app".to_string());
        let mut lock = PackageLock::new_v3(root_name, manifest.version.clone());

        if let Some(root_pkg) = lock.packages.get_mut("") {
            root_pkg.dependencies = manifest.dependencies.clone();
            root_pkg.dev_dependencies = manifest.dev_dependencies.clone();
            root_pkg.peer_dependencies = manifest.peer_dependencies.clone();
            root_pkg.optional_dependencies = manifest.optional_dependencies.clone();
        }

        for (install_path, node) in resolved {
            let lock_pkg = LockPackage {
                name: Some(node.name.clone()),
                version: Some(node.version.clone()),
                resolved: Some(node.resolved_url.clone()),
                integrity: if node.integrity.is_empty() {
                    None
                } else {
                    Some(node.integrity.clone())
                },
                dev: node.dev,
                optional: node.optional,
                has_install_script: node.has_install_script,
                bin: node.bin.clone(),
                engines: node.engines.clone(),
                os: node.os.clone(),
                cpu: node.cpu.clone(),
                dependencies: if node.dependencies.is_empty() {
                    None
                } else {
                    Some(node.dependencies.clone())
                },
                ..Default::default()
            };
            lock.packages.insert(install_path.clone(), lock_pkg);
        }

        lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::registry::RegistryClient;
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use serde_json::json;
    use tar::Builder;
    use tempfile::tempdir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn create_test_tarball() -> Vec<u8> {
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        {
            let mut tar = Builder::new(&mut enc);
            let data = br#"{"name":"pkg","version":"1.0.0"}"#;
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            tar.append_data(&mut header, "package/package.json", &data[..])
                .unwrap();
            tar.finish().unwrap();
        }
        enc.finish().unwrap()
    }

    #[tokio::test]
    async fn test_resolver_deep_cycle_prevention() {
        let mock_server = MockServer::start().await;
        let tarball = create_test_tarball();

        // Package A depends on B, B depends on C, C depends on A (deep cycle)
        let meta_a = json!({
            "name": "pkg-a",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "pkg-a",
                    "version": "1.0.0",
                    "dist": { "tarball": format!("{}/tarballs/a.tgz", mock_server.uri()), "shasum": "abc" },
                    "dependencies": { "pkg-b": "^1.0.0" }
                }
            }
        });
        let meta_b = json!({
            "name": "pkg-b",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "pkg-b",
                    "version": "1.0.0",
                    "dist": { "tarball": format!("{}/tarballs/b.tgz", mock_server.uri()), "shasum": "abc" },
                    "dependencies": { "pkg-c": "^1.0.0" }
                }
            }
        });
        let meta_c = json!({
            "name": "pkg-c",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "pkg-c",
                    "version": "1.0.0",
                    "dist": { "tarball": format!("{}/tarballs/c.tgz", mock_server.uri()), "shasum": "abc" },
                    "dependencies": { "pkg-a": "^1.0.0" }
                }
            }
        });

        Mock::given(method("GET"))
            .and(path("/pkg-a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_a))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/pkg-b"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_b))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/pkg-c"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_c))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/a.tgz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(tarball.clone(), "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/b.tgz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(tarball.clone(), "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/c.tgz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(tarball, "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;

        let tmp = tempdir().unwrap();
        let reg = RegistryClient::new(
            Some(mock_server.uri()),
            Some(tmp.path().join("cache")),
            None,
        );
        let resolver = Resolver::new(tmp.path().to_path_buf(), reg, None);

        let mut manifest = PackageJson::default();
        let mut deps = BTreeMap::new();
        deps.insert("pkg-a".to_string(), "^1.0.0".to_string());
        manifest.dependencies = Some(deps);

        let resolved = resolver
            .resolve_manifest(&manifest)
            .await
            .expect("Resolution should terminate without infinite loop");
        assert!(resolved.contains_key("node_modules/pkg-a"));
        assert!(resolved.contains_key("node_modules/pkg-b"));
        assert!(resolved.contains_key("node_modules/pkg-c"));
    }

    #[tokio::test]
    async fn test_resolver_incompatible_optional_deps_skipped() {
        let mock_server = MockServer::start().await;
        let tarball = create_test_tarball();

        // Package with an optional dependency targeting an impossible OS
        let meta_main = json!({
            "name": "main-pkg",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "main-pkg",
                    "version": "1.0.0",
                    "dist": { "tarball": format!("{}/tarballs/main.tgz", mock_server.uri()), "shasum": "abc" },
                    "optionalDependencies": { "incompatible-native-pkg": "^1.0.0" }
                }
            }
        });
        let meta_incompatible = json!({
            "name": "incompatible-native-pkg",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "incompatible-native-pkg",
                    "version": "1.0.0",
                    "os": ["nonexistent_os_xyz_404"],
                    "cpu": ["nonexistent_cpu_123"],
                    "dist": { "tarball": format!("{}/tarballs/incomp.tgz", mock_server.uri()), "shasum": "abc" }
                }
            }
        });

        Mock::given(method("GET"))
            .and(path("/main-pkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_main))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/incompatible-native-pkg"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_incompatible))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/main.tgz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(tarball, "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;

        let tmp = tempdir().unwrap();
        let reg = RegistryClient::new(
            Some(mock_server.uri()),
            Some(tmp.path().join("cache")),
            None,
        );
        let resolver = Resolver::new(tmp.path().to_path_buf(), reg, None);

        let mut manifest = PackageJson::default();
        let mut deps = BTreeMap::new();
        deps.insert("main-pkg".to_string(), "^1.0.0".to_string());
        manifest.dependencies = Some(deps);

        let resolved = resolver
            .resolve_manifest(&manifest)
            .await
            .expect("Resolution should succeed");
        assert!(resolved.contains_key("node_modules/main-pkg"));
        // Optional incompatible dep MUST be skipped
        assert!(!resolved.contains_key("node_modules/incompatible-native-pkg"));
    }

    #[tokio::test]
    async fn test_resolver_overrides_colliding_with_catalogs() {
        let mock_server = MockServer::start().await;
        let tarball = create_test_tarball();

        let meta_foo = json!({
            "name": "foo",
            "dist-tags": { "latest": "2.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "foo",
                    "version": "1.0.0",
                    "dist": { "tarball": format!("{}/tarballs/foo-1.tgz", mock_server.uri()), "shasum": "abc" }
                },
                "2.0.0": {
                    "name": "foo",
                    "version": "2.0.0",
                    "dist": { "tarball": format!("{}/tarballs/foo-2.tgz", mock_server.uri()), "shasum": "abc" }
                }
            }
        });

        Mock::given(method("GET"))
            .and(path("/foo"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_foo))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/foo-1.tgz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(tarball.clone(), "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/foo-2.tgz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(tarball, "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;

        let tmp = tempdir().unwrap();
        let reg = RegistryClient::new(
            Some(mock_server.uri()),
            Some(tmp.path().join("cache")),
            None,
        );

        // Manifest uses catalog:default which specifies 1.0.0, but overrides specifies 2.0.0
        let mut catalogs = BTreeMap::new();
        let mut default_cat = BTreeMap::new();
        default_cat.insert("foo".to_string(), "1.0.0".to_string());
        catalogs.insert("default".to_string(), default_cat);

        let mut overrides = BTreeMap::new();
        overrides.insert("foo".to_string(), json!("2.0.0"));

        let resolver = Resolver::new(tmp.path().to_path_buf(), reg, None)
            .with_catalogs(catalogs)
            .with_overrides(overrides);

        let mut manifest = PackageJson::default();
        let mut deps = BTreeMap::new();
        deps.insert("foo".to_string(), "catalog:default".to_string());
        manifest.dependencies = Some(deps);

        let resolved = resolver
            .resolve_manifest(&manifest)
            .await
            .expect("Resolution should succeed");
        let foo_node = resolved
            .get("node_modules/foo")
            .expect("foo should be installed");
        // Overrides MUST take precedence over the catalog specification
        assert_eq!(foo_node.version, "2.0.0");
    }

    #[tokio::test]
    async fn test_resolver_peer_dependency_conflict_and_nesting() {
        let mock_server = MockServer::start().await;
        let tarball = create_test_tarball();

        // Root has plugin-a (requires react ^17.0.0) and plugin-b (requires react ^18.0.0)
        // and root directly depends on react 18.0.0
        let meta_react = json!({
            "name": "react",
            "dist-tags": { "latest": "18.0.0" },
            "versions": {
                "17.0.0": {
                    "name": "react",
                    "version": "17.0.0",
                    "dist": { "tarball": format!("{}/tarballs/react-17.tgz", mock_server.uri()), "shasum": "abc" }
                },
                "18.0.0": {
                    "name": "react",
                    "version": "18.0.0",
                    "dist": { "tarball": format!("{}/tarballs/react-18.tgz", mock_server.uri()), "shasum": "abc" }
                }
            }
        });

        let meta_plugin_a = json!({
            "name": "plugin-a",
            "dist-tags": { "latest": "1.0.0" },
            "versions": {
                "1.0.0": {
                    "name": "plugin-a",
                    "version": "1.0.0",
                    "dist": { "tarball": format!("{}/tarballs/plugin-a.tgz", mock_server.uri()), "shasum": "abc" },
                    "peerDependencies": { "react": "^17.0.0" }
                }
            }
        });

        Mock::given(method("GET"))
            .and(path("/react"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_react))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/plugin-a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(meta_plugin_a))
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/react-17.tgz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(tarball.clone(), "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/react-18.tgz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(tarball.clone(), "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;
        Mock::given(method("GET"))
            .and(path("/tarballs/plugin-a.tgz"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(tarball, "application/octet-stream"),
            )
            .mount(&mock_server)
            .await;

        let tmp = tempdir().unwrap();
        let reg = RegistryClient::new(
            Some(mock_server.uri()),
            Some(tmp.path().join("cache")),
            None,
        );
        let resolver = Resolver::new(tmp.path().to_path_buf(), reg, None);

        let mut manifest = PackageJson::default();
        let mut deps = BTreeMap::new();
        deps.insert("react".to_string(), "18.0.0".to_string());
        deps.insert("plugin-a".to_string(), "1.0.0".to_string());
        manifest.dependencies = Some(deps);

        let resolved = resolver
            .resolve_manifest(&manifest)
            .await
            .expect("Resolution should succeed");
        // Root has react 18
        let root_react = resolved
            .get("node_modules/react")
            .expect("react installed at root");
        assert_eq!(root_react.version, "18.0.0");
        assert!(resolved.contains_key("node_modules/plugin-a"));
        // Conflicting peer dependency react ^17.0.0 requested by plugin-a is nested under plugin-a
        let nested_react = resolved
            .get("node_modules/plugin-a/node_modules/react")
            .expect("nested react installed");
        assert_eq!(nested_react.version, "17.0.0");
    }
}
