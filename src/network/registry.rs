use anyhow::{Context, Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

use crate::config::NpmConfig;

const DEFAULT_REGISTRY: &str = "https://registry.npmjs.org";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryPackageMetadata {
    pub name: String,
    #[serde(rename = "dist-tags", default)]
    pub dist_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub versions: BTreeMap<String, RegistryVersionMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryVersionMetadata {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub main: Option<String>,
    #[serde(default)]
    pub bin: Option<serde_json::Value>,
    #[serde(default)]
    pub dist: DistMetadata,
    #[serde(default)]
    pub dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "devDependencies", default)]
    pub dev_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "peerDependencies", default)]
    pub peer_dependencies: Option<BTreeMap<String, String>>,
    #[serde(rename = "peerDependenciesMeta", default)]
    pub peer_dependencies_meta: Option<serde_json::Value>,
    #[serde(rename = "optionalDependencies", default)]
    pub optional_dependencies: Option<BTreeMap<String, String>>,
    #[serde(default, deserialize_with = "deserialize_engines")]
    pub engines: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub os: Option<Vec<String>>,
    #[serde(default)]
    pub cpu: Option<Vec<String>>,
    #[serde(default)]
    pub scripts: Option<BTreeMap<String, String>>,
}

fn deserialize_engines<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<BTreeMap<String, String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let val = serde_json::Value::deserialize(deserializer)?;
    match val {
        serde_json::Value::Object(map) => {
            let mut result = BTreeMap::new();
            for (k, v) in map {
                if let Some(s) = v.as_str() {
                    result.insert(k, s.to_string());
                }
            }
            Ok(Some(result))
        }
        serde_json::Value::Array(arr) => {
            let mut result = BTreeMap::new();
            for item in arr {
                if let Some(s) = item.as_str() {
                    result.insert(s.to_string(), "*".to_string());
                }
            }
            Ok(Some(result))
        }
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DistMetadata {
    #[serde(default)]
    pub tarball: String,
    #[serde(default)]
    pub shasum: Option<String>,
    #[serde(default)]
    pub integrity: Option<String>,
}

#[derive(Clone)]
pub struct RegistryClient {
    client: Client,
    registry_url: String,
    cache_dir: PathBuf,
    npm_config: NpmConfig,
    metadata_cache: Arc<RwLock<BTreeMap<String, RegistryPackageMetadata>>>,
    offline: bool,
}

impl RegistryClient {
    pub fn new(
        registry_url: Option<String>,
        cache_dir: Option<PathBuf>,
        npm_config: Option<NpmConfig>,
    ) -> Self {
        Self::new_opt(registry_url, cache_dir, npm_config, false)
    }

    pub fn new_opt(
        registry_url: Option<String>,
        cache_dir: Option<PathBuf>,
        npm_config: Option<NpmConfig>,
        offline: bool,
    ) -> Self {
        let config = npm_config.unwrap_or_default();
        let registry = registry_url
            .or_else(|| config.registry.clone())
            .unwrap_or_else(|| DEFAULT_REGISTRY.to_string())
            .trim_end_matches('/')
            .to_string();

        let cache = cache_dir.unwrap_or_else(|| {
            if let Ok(custom) = std::env::var("BOLT_CACHE_DIR") {
                PathBuf::from(custom)
            } else if let Some(proj_dirs) = directories::ProjectDirs::from("", "", "bolt") {
                proj_dirs.cache_dir().to_path_buf()
            } else {
                std::env::temp_dir().join("bolt-cache")
            }
        });
        let client = Client::builder()
            .http2_adaptive_window(true)
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(32)
            .build()
            .unwrap_or_else(|_| {
                Client::builder()
                    .timeout(Duration::from_secs(30))
                    .pool_max_idle_per_host(32)
                    .build()
                    .expect("Failed to build HTTP client")
            });

        Self {
            client,
            registry_url: registry,
            cache_dir: cache,
            npm_config: config,
            metadata_cache: Arc::new(RwLock::new(BTreeMap::new())),
            offline,
        }
    }

    pub fn is_offline(&self) -> bool {
        self.offline
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn registry_url(&self) -> &str {
        &self.registry_url
    }

    pub fn npm_config(&self) -> &NpmConfig {
        &self.npm_config
    }

    /// Fetch package metadata document (e.g. `https://registry.npmjs.org/<name>`)
    #[allow(clippy::collapsible_if)]
    pub async fn fetch_package_metadata(&self, name: &str) -> Result<RegistryPackageMetadata> {
        self.fetch_package_metadata_opt(name, false).await
    }

    #[allow(clippy::collapsible_if)]
    pub async fn fetch_package_metadata_fresh(
        &self,
        name: &str,
    ) -> Result<RegistryPackageMetadata> {
        self.fetch_package_metadata_opt(name, true).await
    }

    #[allow(clippy::collapsible_if)]
    async fn fetch_package_metadata_opt(
        &self,
        name: &str,
        bypass_cache: bool,
    ) -> Result<RegistryPackageMetadata> {
        if !bypass_cache {
            let cache = self.metadata_cache.read().await;
            if let Some(meta) = cache.get(name) {
                return Ok(meta.clone());
            }
        }

        let sanitized_name = name.replace('/', "%2f");
        let disk_cache_path = self
            .cache_dir
            .join("metadata")
            .join(format!("{}.json", sanitized_name));
        if !bypass_cache && disk_cache_path.exists() {
            if let Ok(data) = fs::read_to_string(&disk_cache_path) {
                if let Ok(meta) = serde_json::from_str::<RegistryPackageMetadata>(&data) {
                    let mut cache = self.metadata_cache.write().await;
                    cache.insert(name.to_string(), meta.clone());
                    return Ok(meta);
                }
            }
        }

        if self.offline {
            return Err(anyhow!(
                "Offline mode enabled: metadata for '{}' not found in local cache ({:?})",
                name,
                disk_cache_path
            ));
        }
        // Fetch from HTTP registry with retries
        let encoded_name = if let Some(stripped) = name.strip_prefix('@') {
            if let Some((scope, pkg)) = stripped.split_once('/') {
                format!("@{scope}%2f{pkg}")
            } else {
                name.to_string()
            }
        } else {
            name.to_string()
        };

        let url = format!("{}/{}", self.registry_url, encoded_name);

        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))).await;
            }

            let mut req = self.client.get(&url).header(
                "Accept",
                "application/vnd.npm.install-v1+json; q=1.0, application/json; q=0.8",
            );

            if let Some(auth_header) = self.npm_config.get_auth_token_for_url(&url) {
                req = req.header("Authorization", auth_header);
            }

            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let text = resp.text().await.context("Failed to read response body")?;
                        let meta: RegistryPackageMetadata = serde_json::from_str(&text)
                            .with_context(|| format!("Failed to parse metadata for {}", name))?;

                        // Save to disk cache
                        if let Some(parent) = disk_cache_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&disk_cache_path, &text);

                        // Save to in-memory cache
                        let mut cache = self.metadata_cache.write().await;
                        cache.insert(name.to_string(), meta.clone());
                        return Ok(meta);
                    } else if resp.status() == reqwest::StatusCode::NOT_FOUND {
                        return Err(anyhow!(
                            "Package '{}' not found in registry {}",
                            name,
                            self.registry_url
                        ));
                    } else {
                        last_err = Some(anyhow!("Registry HTTP {}: {}", resp.status(), url));
                    }
                }
                Err(e) => {
                    last_err = Some(anyhow!("Request error for {}: {}", url, e));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("Failed to fetch metadata for {}", name)))
    }

    /// Download tarball bytes from URL, caching on disk
    #[allow(clippy::collapsible_if)]
    pub async fn fetch_tarball(&self, tarball_url: &str) -> Result<Vec<u8>> {
        // Compute filename hash for local caching
        let mut hasher = sha2::Sha256::new();
        use sha2::Digest;
        hasher.update(tarball_url.as_bytes());
        let hash = hex::encode(hasher.finalize());

        let tarball_cache_path = self
            .cache_dir
            .join("tarballs")
            .join(format!("{}.tgz", hash));
        if tarball_cache_path.exists() {
            if let Ok(bytes) = fs::read(&tarball_cache_path) {
                return Ok(bytes);
            }
        }

        if self.offline {
            return Err(anyhow!(
                "Offline mode enabled: tarball for '{}' not found in local cache ({:?})",
                tarball_url,
                tarball_cache_path
            ));
        }
        let mut last_err = None;
        for attempt in 0..3 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(200 * (1 << attempt))).await;
            }

            let mut req = self.client.get(tarball_url);
            if let Some(auth_header) = self.npm_config.get_auth_token_for_url(tarball_url) {
                req = req.header("Authorization", auth_header);
            }

            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let bytes = resp
                            .bytes()
                            .await
                            .context("Failed to read tarball stream")?
                            .to_vec();
                        if let Some(parent) = tarball_cache_path.parent() {
                            let _ = fs::create_dir_all(parent);
                        }
                        let _ = fs::write(&tarball_cache_path, &bytes);
                        return Ok(bytes);
                    } else {
                        last_err = Some(anyhow!(
                            "Failed to download tarball from {}: HTTP {}",
                            tarball_url,
                            resp.status()
                        ));
                    }
                }
                Err(e) => {
                    last_err = Some(anyhow!("Network error downloading {}: {}", tarball_url, e));
                }
            }
        }

        Err(last_err.unwrap_or_else(|| anyhow!("Failed to download tarball from {}", tarball_url)))
    }
}
