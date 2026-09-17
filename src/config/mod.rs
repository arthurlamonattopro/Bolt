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
        let request_url = url::Url::parse(url).ok()?;
        if request_url.scheme() != "https"
            || !request_url.username().is_empty()
            || request_url.password().is_some()
        {
            return None;
        }

        self.auth_tokens
            .iter()
            .filter_map(|(key, token)| {
                let registry_part = key.strip_suffix(":_authToken")?;
                if !registry_part.starts_with("//") {
                    return None;
                }
                let scope = url::Url::parse(&format!("https:{registry_part}")).ok()?;
                if scope.host_str() != request_url.host_str()
                    || scope.port_or_known_default() != request_url.port_or_known_default()
                    || !scope.username().is_empty()
                    || scope.password().is_some()
                    || scope.query().is_some()
                    || scope.fragment().is_some()
                {
                    return None;
                }

                let scope_path = scope.path().trim_end_matches('/');
                let remainder = request_url.path().strip_prefix(scope_path)?;
                if !remainder.is_empty() && !remainder.starts_with('/') {
                    return None;
                }

                Some((scope_path.len(), token))
            })
            .max_by_key(|(length, _)| *length)
            .map(|(_, token)| format!("Bearer {token}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_tokens_require_matching_secure_scope() {
        let mut config = NpmConfig::default();
        config.auth_tokens.insert(
            "//registry.example.test/packages/:_authToken".to_string(),
            "test-token".to_string(),
        );

        for url in [
            "https://registry.example.test/packages/pkg",
            "https://REGISTRY.example.test:443/packages/pkg",
        ] {
            assert_eq!(
                config.get_auth_token_for_url(url).as_deref(),
                Some("Bearer test-token"),
                "{url}"
            );
        }

        for url in [
            "http://registry.example.test/packages/pkg",
            "https://other.example.test/packages/pkg",
            "https://registry.example.test:8443/packages/pkg",
            "https://registry.example.test/packages-other/pkg",
            "https://registry.example.test/other/pkg",
            "not a URL",
        ] {
            assert!(config.get_auth_token_for_url(url).is_none(), "{url}");
        }
    }

    #[test]
    fn auth_tokens_reject_invalid_scopes() {
        for scope in [
            "//:_authToken",
            "registry.example.test/:_authToken",
            "//registry.example.test/?query=value:_authToken",
            "//registry.example.test/#fragment:_authToken",
        ] {
            let mut config = NpmConfig::default();
            config
                .auth_tokens
                .insert(scope.to_string(), "test-token".to_string());
            assert!(
                config
                    .get_auth_token_for_url("https://registry.example.test/packages/pkg")
                    .is_none(),
                "{scope}"
            );
        }
    }

    #[test]
    fn auth_tokens_match_explicit_port_and_path_boundary() {
        let mut config = NpmConfig::default();
        config.auth_tokens.insert(
            "//registry.example.test:8443/packages:_authToken".to_string(),
            "test-token".to_string(),
        );

        for url in [
            "https://registry.example.test:8443/packages",
            "https://registry.example.test:8443/packages/",
            "https://registry.example.test:8443/packages/pkg",
        ] {
            assert_eq!(
                config.get_auth_token_for_url(url).as_deref(),
                Some("Bearer test-token")
            );
        }
        assert!(
            config
                .get_auth_token_for_url("https://registry.example.test/packages/pkg")
                .is_none()
        );
    }

    #[test]
    fn auth_tokens_prefer_most_specific_path() {
        let mut config = NpmConfig::default();
        config.auth_tokens.insert(
            "//registry.example.test/:_authToken".to_string(),
            "root-token".to_string(),
        );
        config.auth_tokens.insert(
            "//registry.example.test/packages/:_authToken".to_string(),
            "scoped-token".to_string(),
        );

        assert_eq!(
            config
                .get_auth_token_for_url("https://registry.example.test/packages/pkg")
                .as_deref(),
            Some("Bearer scoped-token")
        );
        assert_eq!(
            config
                .get_auth_token_for_url("https://registry.example.test/other/pkg")
                .as_deref(),
            Some("Bearer root-token")
        );
    }
}
