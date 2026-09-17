use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Represents npm's `package-lock.json` format.
/// Supports reading v2 and v3, automatically normalizing/upgrading to v3.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageLock {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub lockfile_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<bool>,
    /// The `packages` map maps package installation paths to their metadata:
    /// `""` represents the root project.
    /// `"node_modules/foo"` represents top-level installed package `foo`.
    /// `"node_modules/foo/node_modules/bar"` represents nested `bar`.
    #[serde(default)]
    pub packages: BTreeMap<String, LockPackage>,
    /// Legacy dependencies map (used in lockfileVersion 1 and 2)
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LockPackage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dev: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub optional: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub dev_optional: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub has_install_script: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engines: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dependencies: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dev_dependencies: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_dependencies_meta: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub optional_dependencies: Option<BTreeMap<String, String>>,
    /// Workspaces link or extra fields
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<bool>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl PackageLock {
    pub fn new_v3(name: String, version: Option<String>) -> Self {
        let mut packages = BTreeMap::new();
        // Insert root entry ""
        let root_pkg = LockPackage {
            name: Some(name.clone()),
            version: version.clone(),
            ..Default::default()
        };
        packages.insert("".to_string(), root_pkg);

        Self {
            name,
            version,
            lockfile_version: 3,
            requires: None,
            packages,
            dependencies: BTreeMap::new(),
        }
    }

    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read lockfile at {:?}", path.as_ref()))?;
        Self::from_str(&content)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self> {
        let mut lock: Self =
            serde_json::from_str(s).context("Failed to parse package-lock.json")?;

        // Upgrade v2 lockfile or normalize: v3 lockfiles set lockfileVersion: 3 and drop redundant dependencies object
        if lock.lockfile_version < 3 {
            lock.lockfile_version = 3;
            // In v3, `dependencies` is no longer generated or required because `packages` holds full layout
            lock.dependencies.clear();
            lock.requires = None;
        }

        // Ensure root package exists if missing
        if !lock.packages.contains_key("") {
            lock.packages.insert(
                "".to_string(),
                LockPackage {
                    name: Some(lock.name.clone()),
                    version: lock.version.clone(),
                    ..Default::default()
                },
            );
        }
        Ok(lock)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize package-lock.json")?;
        fs::write(path.as_ref(), format!("{}\n", content))
            .with_context(|| format!("Failed to write package-lock.json at {:?}", path.as_ref()))?;
        Ok(())
    }
}
