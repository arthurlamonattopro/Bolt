use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PackageJson {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bin: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scripts: Option<BTreeMap<String, String>>,

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

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspaces: Option<serde_json::Value>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overrides: Option<BTreeMap<String, serde_json::Value>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolutions: Option<BTreeMap<String, serde_json::Value>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalogs: Option<BTreeMap<String, BTreeMap<String, String>>>,
    /// Preserve all unknown/unrelated fields
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl PackageJson {
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read package.json at {:?}", path.as_ref()))?;
        Self::from_str(&content)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Result<Self> {
        let pkg: Self = serde_json::from_str(s).context("Failed to parse package.json")?;
        Ok(pkg)
    }

    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize package.json")?;
        fs::write(path.as_ref(), format!("{}\n", content))
            .with_context(|| format!("Failed to write package.json at {:?}", path.as_ref()))?;
        Ok(())
    }

    pub fn add_dependency(&mut self, name: String, version_req: String, dep_type: DependencyType) {
        match dep_type {
            DependencyType::Prod => {
                let deps = self.dependencies.get_or_insert_with(BTreeMap::new);
                deps.insert(name, version_req);
            }
            DependencyType::Dev => {
                let deps = self.dev_dependencies.get_or_insert_with(BTreeMap::new);
                deps.insert(name, version_req);
            }
            DependencyType::Optional => {
                let deps = self.optional_dependencies.get_or_insert_with(BTreeMap::new);
                deps.insert(name, version_req);
            }
            DependencyType::Peer => {
                let deps = self.peer_dependencies.get_or_insert_with(BTreeMap::new);
                deps.insert(name, version_req);
            }
        }
    }

    pub fn remove_dependency(&mut self, name: &str) -> bool {
        let mut removed = false;
        if let Some(deps) = &mut self.dependencies {
            removed |= deps.remove(name).is_some();
        }
        if let Some(deps) = &mut self.dev_dependencies {
            removed |= deps.remove(name).is_some();
        }
        if let Some(deps) = &mut self.optional_dependencies {
            removed |= deps.remove(name).is_some();
        }
        if let Some(deps) = &mut self.peer_dependencies {
            removed |= deps.remove(name).is_some();
        }
        removed
    }

    /// Extract executable binaries defined in "bin"
    pub fn get_bins(&self) -> BTreeMap<String, String> {
        let mut result = BTreeMap::new();
        let pkg_name = self.name.as_deref().unwrap_or("");
        let default_bin_name = if let Some(stripped) = pkg_name.strip_prefix('@') {
            stripped.split('/').nth(1).unwrap_or(pkg_name)
        } else {
            pkg_name
        };

        if let Some(bin_val) = &self.bin {
            match bin_val {
                serde_json::Value::String(s) => {
                    if !default_bin_name.is_empty() {
                        result.insert(default_bin_name.to_string(), s.clone());
                    }
                }
                serde_json::Value::Object(map) => {
                    for (k, v) in map {
                        if let serde_json::Value::String(s) = v {
                            result.insert(k.clone(), s.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        result
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DependencyType {
    Prod,
    Dev,
    Optional,
    Peer,
}
