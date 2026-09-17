use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NpmConfig {
    pub registry: Option<String>,
    pub auth_tokens: std::collections::BTreeMap<String, String>,
}

impl NpmConfig {
    /// Load .npmrc configurations from user home directory and project directory
    pub fn load_from_env_and_files(project_dir: &Path) -> Self {
        let mut config = Self::default();

        // 1. Check user home .npmrc
        if let Some(user_dirs) = directories::UserDirs::new() {
            let user_npmrc = user_dirs.home_dir().join(".npmrc");
            if user_npmrc.exists() {
                let _ = config.parse_file(&user_npmrc);
            }
        }

        // 2. Check project directory .npmrc
        let proj_npmrc = project_dir.join(".npmrc");
        if proj_npmrc.exists() {
            let _ = config.parse_file(&proj_npmrc);
        }

        // 3. Environment variables (e.g. NPM_CONFIG_REGISTRY)
        if let Ok(reg) = std::env::var("NPM_CONFIG_REGISTRY") {
            config.registry = Some(reg);
        }

        config
    }

    pub fn parse_file(&mut self, path: &Path) -> Result<()> {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read .npmrc at {:?}", path))?;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim();
                let val = val.trim();

                if key == "registry" {
                    self.registry = Some(val.to_string());
                } else if key.ends_with(":_authToken") {
                    // e.g. //registry.npmjs.org/:_authToken=npm_xxxx
                    self.auth_tokens.insert(key.to_string(), val.to_string());
                }
            }
        }

        Ok(())
    }

    /// Retrieve authentication header value if a token is configured for this registry URL
    pub fn get_auth_token_for_url(&self, url: &str) -> Option<String> {
        for (k, v) in &self.auth_tokens {
            if let Some(registry_part) = k.strip_suffix(":_authToken") {
                let trimmed_part = registry_part.trim_start_matches('/').trim_end_matches('/');
                if url.contains(trimmed_part) {
                    return Some(format!("Bearer {}", v));
                }
            }
        }
        None
    }
}
