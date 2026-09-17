use anyhow::{Result, anyhow};
use node_semver::{Range, Version};

/// Wrapper around npm semver comparison and range matching
#[derive(Debug, Clone)]
pub struct SemverSpec {
    pub raw: String,
    parsed_sets: Option<Vec<Vec<Range>>>,
}

impl SemverSpec {
    pub fn new(raw: &str) -> Self {
        let trimmed = raw.trim();
        let parsed_sets = Self::parse_complex_range(trimmed);
        Self {
            raw: trimmed.to_string(),
            parsed_sets,
        }
    }

    fn parse_complex_range(input: &str) -> Option<Vec<Vec<Range>>> {
        if input == "*" || input == "latest" || input.is_empty() {
            return None;
        }

        // If the whole expression parses cleanly as a single Range:
        if let Ok(single) = Range::parse(input) {
            return Some(vec![vec![single]]);
        }

        let mut or_sets = Vec::new();
        for or_part in input.split("||") {
            let or_part = or_part.trim();
            if or_part.is_empty() {
                continue;
            }
            if let Ok(single) = Range::parse(or_part) {
                or_sets.push(vec![single]);
                continue;
            }

            // Tokenize space-separated comparators
            let mut and_ranges = Vec::new();
            let words: Vec<&str> = or_part.split_whitespace().collect();
            let mut i = 0;
            let mut ok = true;
            while i < words.len() {
                if words[i] == "-" {
                    ok = false;
                    break;
                }
                if i + 2 < words.len() && words[i + 1] == "-" {
                    let hyphen_slice = format!("{} - {}", words[i], words[i + 2]);
                    if let Ok(r) = Range::parse(&hyphen_slice) {
                        and_ranges.push(r);
                        i += 3;
                        continue;
                    }
                }
                if (words[i] == ">"
                    || words[i] == ">="
                    || words[i] == "<"
                    || words[i] == "<="
                    || words[i] == "=")
                    && i + 1 < words.len()
                {
                    let combined = format!("{}{}", words[i], words[i + 1]);
                    if let Ok(r) = Range::parse(&combined) {
                        and_ranges.push(r);
                        i += 2;
                        continue;
                    }
                }
                if let Ok(r) = Range::parse(words[i]) {
                    and_ranges.push(r);
                    i += 1;
                } else {
                    ok = false;
                    break;
                }
            }
            if ok && !and_ranges.is_empty() {
                or_sets.push(and_ranges);
            } else {
                return None;
            }
        }

        if or_sets.is_empty() {
            None
        } else {
            Some(or_sets)
        }
    }

    /// Check if a version matches this range / tag / specifier
    pub fn matches(&self, version_str: &str) -> bool {
        let trimmed = self.raw.trim();
        if trimmed == "*" || trimmed == "latest" || trimmed.is_empty() {
            return true;
        }

        // Direct exact match
        if trimmed == version_str {
            return true;
        }

        let Ok(ver) = Version::parse(version_str) else {
            return false;
        };

        if let Ok(raw_ver) = Version::parse(trimmed) {
            if ver == raw_ver {
                return true;
            }
        }

        if let Some(sets) = &self.parsed_sets {
            return sets
                .iter()
                .any(|and_group| and_group.iter().all(|r| ver.satisfies(r)));
        }

        // Exact match fallback
        trimmed == version_str
    }
    /// Pick the highest matching version from a list of version strings
    #[allow(clippy::collapsible_if)]
    pub fn select_best<'a>(&self, versions: &[&'a str]) -> Option<&'a str> {
        let trimmed = self.raw.trim();
        let mut candidates: Vec<(Version, &'a str)> = Vec::new();

        for &v in versions {
            if let Ok(parsed) = Version::parse(v) {
                if self.matches(v) {
                    candidates.push((parsed, v));
                }
            }
        }

        // Sort descending by semver
        candidates.sort_by(|a, b| b.0.cmp(&a.0));

        // If range allows pre-release, or no non-prerelease found, return highest
        let allows_prerelease = trimmed.contains('-');
        if !allows_prerelease {
            if let Some((_, v)) = candidates
                .iter()
                .find(|(parsed, _)| parsed.build.is_empty() && parsed.pre_release.is_empty())
            {
                return Some(*v);
            }
        }

        candidates.first().map(|(_, v)| *v)
    }
}

/// Parsed representation of a package dependency requirement,
/// supporting standard dependencies, npm aliases, local files, git repos, and tarball URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencySpec {
    /// The actual package name or path
    pub registry_name: String,
    /// The version constraint, file path, git URL, or tarball URL
    pub version_req: String,
    /// If an alias was used, this holds the custom alias name (key in dependencies)
    pub alias_name: Option<String>,
    /// Whether this is a local path dependency (`file:...`)
    pub is_local_file: bool,
    /// Whether this is a git dependency (`git+...`, `git://...`, `github:...`, etc.)
    pub is_git: bool,
    /// Whether this is a direct remote tarball URL (`http://...`, `https://...` ending with .tgz or tarball)
    pub is_tarball_url: bool,
}

/// Parse a dependency requirement in `package.json` for a given dependency key `dep_name`.
/// Supports:
/// - Standard: `dep_name: "^1.0.0"` -> registry_name = `dep_name`, version_req = `"^1.0.0"`
/// - Local file: `dep_name: "file:../local-pkg"` -> is_local_file = true
/// - npm alias: `my-alias: "npm:real-pkg@^2.0.0"` -> registry_name = `"real-pkg"`, version_req = `"^2.0.0"`
/// - Git: `dep_name: "github:user/repo"` or `"git+https://..."` or `"git://..."` -> is_git = true
/// - Tarball URL: `dep_name: "https://example.com/pkg.tgz"` -> is_tarball_url = true
pub fn parse_dependency_req(dep_name: &str, req: &str) -> DependencySpec {
    parse_dependency_req_with_catalogs(dep_name, req, None)
}

pub fn parse_dependency_req_with_catalogs(
    dep_name: &str,
    req: &str,
    catalogs: Option<
        &std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
    >,
) -> DependencySpec {
    let mut req = req.trim();
    let resolved_catalog_req: String;
    if req == "catalog:" || req == "catalog:default" {
        if let Some(cats) = catalogs {
            if let Some(default_cat) = cats.get("default") {
                if let Some(target_ver) = default_cat.get(dep_name) {
                    resolved_catalog_req = target_ver.clone();
                    req = &resolved_catalog_req;
                }
            }
        }
    } else if let Some(named_catalog) = req.strip_prefix("catalog:") {
        let cat_name = named_catalog.trim();
        if let Some(cats) = catalogs {
            if let Some(cat) = cats.get(cat_name) {
                if let Some(target_ver) = cat.get(dep_name) {
                    resolved_catalog_req = target_ver.clone();
                    req = &resolved_catalog_req;
                }
            }
        }
    }
    if let Some(stripped) = req.strip_prefix("file:") {
        DependencySpec {
            registry_name: dep_name.to_string(),
            version_req: stripped.to_string(),
            alias_name: None,
            is_local_file: true,
            is_git: false,
            is_tarball_url: false,
        }
    } else if let Some(stripped) = req.strip_prefix("npm:") {
        let (real_pkg, real_req) = if let Ok((p, r)) = parse_package_spec(stripped) {
            (p, r)
        } else {
            (stripped.to_string(), "latest".to_string())
        };

        DependencySpec {
            registry_name: real_pkg,
            version_req: real_req,
            alias_name: Some(dep_name.to_string()),
            is_local_file: false,
            is_git: false,
            is_tarball_url: false,
        }
    } else if is_git_specifier(req) {
        DependencySpec {
            registry_name: dep_name.to_string(),
            version_req: req.to_string(),
            alias_name: None,
            is_local_file: false,
            is_git: true,
            is_tarball_url: false,
        }
    } else if is_tarball_url_specifier(req) {
        DependencySpec {
            registry_name: dep_name.to_string(),
            version_req: req.to_string(),
            alias_name: None,
            is_local_file: false,
            is_git: false,
            is_tarball_url: true,
        }
    } else {
        DependencySpec {
            registry_name: dep_name.to_string(),
            version_req: req.to_string(),
            alias_name: None,
            is_local_file: false,
            is_git: false,
            is_tarball_url: false,
        }
    }
}

/// Detects if a specifier is a Git URL or shorthand (e.g. github:user/repo, git+https://..., git://...)
pub fn is_git_specifier(req: &str) -> bool {
    let s = req.trim();
    s.starts_with("git+")
        || s.starts_with("git://")
        || s.starts_with("git@")
        || s.starts_with("github:")
        || s.starts_with("gitlab:")
        || s.starts_with("bitbucket:")
        || (s.starts_with("https://") && (s.ends_with(".git") || s.contains(".git#")))
        || (s.starts_with("http://") && (s.ends_with(".git") || s.contains(".git#")))
}

/// Detects if a specifier is a direct tarball URL (e.g. https://.../pkg.tgz or http://.../pkg.tar.gz)
pub fn is_tarball_url_specifier(req: &str) -> bool {
    let s = req.trim();
    if is_git_specifier(s) {
        return false;
    }
    (s.starts_with("https://") || s.starts_with("http://"))
        && (s.ends_with(".tgz")
            || s.ends_with(".tar.gz")
            || s.ends_with(".tar")
            || s.contains(".tgz?")
            || s.contains(".tar.gz?"))
}

/// Normalizes a Git specifier into a cloneable URL and optional git committish/branch/tag ref
pub fn normalize_git_url(spec: &str) -> (String, Option<String>) {
    let spec = spec.trim();
    let (url_part, committish) = if let Some(idx) = spec.find('#') {
        (&spec[..idx], Some(spec[idx + 1..].to_string()))
    } else {
        (spec, None)
    };

    let normalized_url = if let Some(stripped) = url_part.strip_prefix("github:") {
        format!("https://github.com/{}.git", stripped)
    } else if let Some(stripped) = url_part.strip_prefix("gitlab:") {
        format!("https://gitlab.com/{}.git", stripped)
    } else if let Some(stripped) = url_part.strip_prefix("bitbucket:") {
        format!("https://bitbucket.org/{}.git", stripped)
    } else if let Some(stripped) = url_part.strip_prefix("git+ssh://") {
        format!("ssh://{}", stripped)
    } else if let Some(stripped) = url_part.strip_prefix("git+https://") {
        format!("https://{}", stripped)
    } else if let Some(stripped) = url_part.strip_prefix("git+http://") {
        format!("http://{}", stripped)
    } else if let Some(stripped) = url_part.strip_prefix("git+file://") {
        format!("file://{}", stripped)
    } else {
        url_part.to_string()
    };

    (normalized_url, committish)
}

/// Parse a CLI package specifier like `express`, `lodash@^4.17.21`, `@types/node@latest`, `@scope/pkg@1.0.0`, `file:../pkg`
pub fn parse_package_spec(spec: &str) -> Result<(String, String)> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err(anyhow!("Empty package specifier"));
    }

    if let Some(stripped) = spec.strip_prefix("file:") {
        let path = std::path::Path::new(stripped);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "local-pkg".to_string());
        return Ok((name, format!("file:{}", stripped)));
    }

    if is_git_specifier(spec) {
        let (norm_url, _) = normalize_git_url(spec);
        let repo_name = norm_url
            .trim_end_matches('/')
            .strip_suffix(".git")
            .unwrap_or(&norm_url)
            .rsplit('/')
            .next()
            .unwrap_or("git-pkg")
            .to_string();
        return Ok((repo_name, spec.to_string()));
    }

    if is_tarball_url_specifier(spec) {
        let url_path = spec.split('?').next().unwrap_or(spec);
        let file_name = url_path.rsplit('/').next().unwrap_or("tarball-pkg");
        let pkg_name = file_name
            .strip_suffix(".tgz")
            .or_else(|| file_name.strip_suffix(".tar.gz"))
            .or_else(|| file_name.strip_suffix(".tar"))
            .unwrap_or(file_name)
            .to_string();
        return Ok((pkg_name, spec.to_string()));
    }

    if let Some(stripped) = spec.strip_prefix('@') {
        if let Some(idx) = stripped.find('@') {
            let pkg_name = format!("@{}", &stripped[..idx]);
            let version = &stripped[idx + 1..];
            Ok((pkg_name, version.to_string()))
        } else {
            Ok((format!("@{}", stripped), "latest".to_string()))
        }
    } else if let Some(idx) = spec.find('@') {
        let pkg_name = &spec[..idx];
        let version = &spec[idx + 1..];
        Ok((pkg_name.to_string(), version.to_string()))
    } else {
        Ok((spec.to_string(), "latest".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gen_mapping_best() {
        let spec = SemverSpec::new("0.4.0-beta.0");
        let versions = vec![
            "0.1.0",
            "0.1.1",
            "0.2.0",
            "0.3.0",
            "0.3.1",
            "0.3.2",
            "0.3.3",
            "0.3.4",
            "0.3.5",
            "0.3.6-beta.0",
            "0.3.6-beta.1",
            "0.3.6",
            "0.3.12",
            "0.3.13",
            "0.4.0-beta.0",
        ];
        assert!(spec.matches("0.4.0-beta.0"));
        assert_eq!(spec.select_best(&versions), Some("0.4.0-beta.0"));
    }
}
