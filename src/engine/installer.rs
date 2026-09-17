use super::bin_shim::create_bin_shims;
use super::scripts::run_script;
use crate::network::registry::RegistryClient;
use crate::network::tarball::{extract_tarball_safe, verify_integrity};
use crate::package::manifest::PackageJson;
use crate::resolver::ResolvedNode;
use anyhow::{Context, Result, anyhow};
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
pub struct Installer {
    root_dir: PathBuf,
    registry: RegistryClient,
    concurrency_limit: usize,
    ignore_scripts: bool,
    hardlink: bool,
}

impl Installer {
    pub fn new(
        root_dir: PathBuf,
        registry: RegistryClient,
        ignore_scripts: bool,
        hardlink: bool,
    ) -> Self {
        Self {
            root_dir,
            registry,
            concurrency_limit: 16,
            ignore_scripts,
            hardlink,
        }
    }

    /// Install all resolved nodes into node_modules
    pub async fn install_all(&self, resolved: &BTreeMap<String, ResolvedNode>) -> Result<()> {
        let semaphore = Arc::new(Semaphore::new(self.concurrency_limit));
        let root_dir = self.root_dir.clone();
        let registry = self.registry.clone();

        // 1. Download/copy and extract packages concurrently, grouped by depth so parents are extracted before children
        let mut install_items: Vec<ResolvedNode> = resolved.values().cloned().collect();
        install_items.sort_by_key(|n| n.install_path.matches('/').count());

        let total_items = install_items.len() as u64;
        let pb = Arc::new(ProgressBar::new(total_items));
        pb.set_style(
            ProgressStyle::default_bar()
                .template(
                    "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} {msg}",
                )
                .unwrap_or_else(|_| ProgressStyle::default_bar())
                .progress_chars("#>-"),
        );
        pb.enable_steady_tick(Duration::from_millis(100));

        // Group items by depth
        let mut depth_groups: BTreeMap<usize, Vec<ResolvedNode>> = BTreeMap::new();
        for item in install_items {
            let depth = item.install_path.matches('/').count();
            depth_groups.entry(depth).or_default().push(item);
        }

        for (_depth, items) in depth_groups {
            let tasks = stream::iter(items).map(|node| {
                let sem = semaphore.clone();
                let root = root_dir.clone();
                let reg = registry.clone();
                let hardlink_mode = self.hardlink;
                let progress = pb.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    let target_dir = root.join(&node.install_path);
                    let parent_dir = target_dir.parent().unwrap_or(&root);
                    create_dir_all_with_retry(parent_dir)?;
                    if target_dir.exists() {
                        let _ = remove_dir_all_with_retry(&target_dir);
                    }
                    create_dir_all_with_retry(&target_dir)?;
                    if node.is_local_file {
                        if let Some(src_path) = &node.local_source_path {
                            copy_dir_all(src_path, &target_dir).with_context(|| {
                                format!(
                                    "Failed to copy local package from {:?} to {:?}",
                                    src_path, target_dir
                                )
                            })?;
                        }
                    } else {
                        if node.resolved_url.is_empty() {
                            return Err(anyhow!(
                                "Empty resolved_url for package {}@{}",
                                node.name,
                                node.version
                            ));
                        }
                        let tarball_bytes = reg
                            .fetch_tarball(&node.resolved_url)
                            .await
                            .with_context(|| {
                                format!(
                                    "Failed to download tarball for {}@{} from '{}'",
                                    node.name, node.version, node.resolved_url
                                )
                            })?;
                        verify_integrity(&tarball_bytes, &node.integrity).with_context(|| {
                            format!("Integrity check failed for {}@{}", node.name, node.version)
                        })?;
                        if hardlink_mode {
                            // CAS Store mode: extract into store first, then hardlink
                            let store_dir = reg.cache_dir().join("store").join(format!(
                                "{}@{}",
                                node.name.replace('/', "+"),
                                node.version
                            ));
                            if !store_dir.exists() {
                                extract_tarball_safe(&tarball_bytes, &store_dir).with_context(
                                    || {
                                        format!(
                                            "Failed to extract {}@{} to CAS store {:?}",
                                            node.name, node.version, store_dir
                                        )
                                    },
                                )?;
                            }
                            hardlink_dir_all(&store_dir, &target_dir).with_context(|| {
                                format!(
                                    "Failed to hardlink {}@{} from store {:?}",
                                    node.name, node.version, store_dir
                                )
                            })?;
                        } else {
                            extract_tarball_safe(&tarball_bytes, &target_dir).with_context(
                                || {
                                    format!(
                                        "Failed to extract {}@{} to {:?}",
                                        node.name, node.version, target_dir
                                    )
                                },
                            )?;
                        }
                    }
                    progress.inc(1);
                    progress.set_message(format!("{}@{}", node.name, node.version));
                    Result::<(), anyhow::Error>::Ok(())
                })
            });
            let results = tasks
                .buffer_unordered(self.concurrency_limit)
                .collect::<Vec<_>>()
                .await;
            for res in results {
                res.context("Join error in download task")??;
            }
        }
        pb.finish_and_clear();

        // 2. Link binary shims in node_modules/.bin for all installed packages
        let bin_dir = self.root_dir.join("node_modules").join(".bin");

        for node in resolved.values() {
            let pkg_dir = self.root_dir.join(&node.install_path);
            let pkg_json_path = pkg_dir.join("package.json");

            if let Ok(pkg_json) = PackageJson::from_path(&pkg_json_path) {
                let _ = create_bin_shims(&bin_dir, &pkg_dir, &pkg_json);
            }
        }

        // 3. Execute lifecycle scripts unless --ignore-scripts is specified
        if !self.ignore_scripts {
            for node in resolved.values() {
                let pkg_dir = self.root_dir.join(&node.install_path);
                let pkg_json_path = pkg_dir.join("package.json");
                if let Ok(pkg_json) = PackageJson::from_path(&pkg_json_path) {
                    if let Some(scripts) = &pkg_json.scripts {
                        for script_name in &["preinstall", "install", "postinstall"] {
                            if scripts.contains_key(*script_name) {
                                let _ = run_script(&pkg_dir, script_name, &[]);
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
fn copy_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_child = dst.join(entry.file_name());
        if ty.is_dir() {
            let name = entry.file_name();
            if name != "node_modules" && name != ".git" {
                copy_dir_all(&entry.path(), &dest_child)?;
            }
        } else {
            copy_file_with_retry(&entry.path(), &dest_child)?;
        }
    }
    Ok(())
}

fn hardlink_dir_all(src: &PathBuf, dst: &PathBuf) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_child = dst.join(entry.file_name());
        if ty.is_dir() {
            hardlink_dir_all(&entry.path(), &dest_child)?;
        } else {
            // Try hard link first; fallback to copy if cross-filesystem or unsupported
            if fs::hard_link(entry.path(), &dest_child).is_err() {
                copy_file_with_retry(&entry.path(), &dest_child)?;
            }
        }
    }
    Ok(())
}

/// Retry filesystem removal on Windows when files are locked by antivirus or indexer
pub fn remove_dir_all_with_retry(path: &std::path::Path) -> std::io::Result<()> {
    for attempt in 0..5 {
        match fs::remove_dir_all(path) {
            Ok(()) => return Ok(()),
            Err(_e) if attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(15 * (1 << attempt)));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Retry directory creation on Windows when under indexing or antivirus contention
fn create_dir_all_with_retry<P: AsRef<std::path::Path>>(path: P) -> Result<()> {
    let mut attempts = 0;
    loop {
        match fs::create_dir_all(path.as_ref()) {
            Ok(()) => return Ok(()),
            Err(e) => {
                attempts += 1;
                if attempts >= 5 {
                    return Err(e)
                        .context(format!("Failed to create directory {:?}", path.as_ref()));
                }
                std::thread::sleep(std::time::Duration::from_millis(50 * attempts));
            }
        }
    }
}
pub fn copy_file_with_retry(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<u64> {
    for attempt in 0..5 {
        match fs::copy(from, to) {
            Ok(bytes) => return Ok(bytes),
            Err(_e) if attempt < 4 => {
                std::thread::sleep(std::time::Duration::from_millis(15 * (1 << attempt)));
            }
            Err(e) => return Err(e),
        }
    }
    fs::copy(from, to)
}
