use anyhow::{Context, Result, anyhow};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;

use crate::network::registry::RegistryClient;
use crate::package::lockfile::PackageLock;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Low,
    Moderate,
    High,
    Critical,
}

impl Severity {
    pub fn from_str_loose(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "info" => Severity::Info,
            "low" => Severity::Low,
            "moderate" | "medium" => Severity::Moderate,
            "high" => Severity::High,
            "critical" => Severity::Critical,
            _ => Severity::Low,
        }
    }

    pub fn rank(&self) -> u8 {
        match self {
            Severity::Info => 0,
            Severity::Low => 1,
            Severity::Moderate => 2,
            Severity::High => 3,
            Severity::Critical => 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisoryItem {
    pub id: Option<serde_json::Value>,
    pub url: Option<String>,
    pub title: Option<String>,
    pub severity: Option<String>,
    pub vulnerable_versions: Option<String>,
    #[serde(default)]
    pub cwe: Vec<String>,
    pub cvss: Option<CvssInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CvssInfo {
    pub score: Option<f64>,
    #[serde(rename = "vectorString")]
    pub vector_string: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditReport {
    pub advisories: BTreeMap<String, Vec<AdvisoryItem>>,
    pub metadata: AuditMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuditMetadata {
    pub total_dependencies: usize,
    pub vulnerabilities: VulnerabilityCount,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct VulnerabilityCount {
    pub info: usize,
    pub low: usize,
    pub moderate: usize,
    pub high: usize,
    pub critical: usize,
    pub total: usize,
}

/// Run security audit against the npm Bulk Advisory endpoint:
/// POST `/-/npm/v1/security/advisories/bulk` with gzipped JSON `{ "<pkg_name>": ["<version>", ...] }`
pub async fn run_audit(
    registry: &RegistryClient,
    lockfile: &PackageLock,
    omit_dev: bool,
) -> Result<AuditReport> {
    let mut payload: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut total_deps = 0;

    for (path, pkg) in &lockfile.packages {
        if path.is_empty() {
            continue; // root project itself
        }
        if omit_dev && pkg.dev {
            continue;
        }

        let name = if let Some(n) = &pkg.name {
            n.clone()
        } else if let Some(stripped) = path.strip_prefix("node_modules/") {
            // In case of nested, take last segment
            stripped
                .split("/node_modules/")
                .last()
                .unwrap_or(stripped)
                .to_string()
        } else {
            path.clone()
        };

        if let Some(v) = &pkg.version {
            total_deps += 1;
            let versions = payload.entry(name).or_default();
            if !versions.contains(v) {
                versions.push(v.clone());
            }
        }
    }

    if payload.is_empty() {
        return Ok(AuditReport {
            advisories: BTreeMap::new(),
            metadata: AuditMetadata {
                total_dependencies: 0,
                vulnerabilities: VulnerabilityCount::default(),
            },
        });
    }

    let json_bytes = serde_json::to_vec(&payload)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&json_bytes)?;
    let gzipped = encoder.finish()?;

    let url = format!(
        "{}/-/npm/v1/security/advisories/bulk",
        registry.registry_url()
    );
    let client = reqwest::Client::new();
    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Content-Encoding", "gzip")
        .header("Accept", "application/json")
        .body(gzipped);

    if let Some(auth_header) = registry.npm_config().get_auth_token_for_url(&url) {
        req = req.header("Authorization", auth_header);
    }

    let resp = req.send().await.context("Failed to send audit request")?;
    if !resp.status().is_success() {
        return Err(anyhow!(
            "Audit endpoint returned HTTP status {}: {}",
            resp.status(),
            url
        ));
    }

    let body_bytes = resp
        .bytes()
        .await
        .context("Failed to read audit response")?;
    // The response might be gzipped or plain json
    let decompressed = if body_bytes.starts_with(&[0x1f, 0x8b]) {
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut decoder = GzDecoder::new(&body_bytes[..]);
        let mut s = Vec::new();
        decoder.read_to_end(&mut s)?;
        s
    } else {
        body_bytes.to_vec()
    };

    let advisories: BTreeMap<String, Vec<AdvisoryItem>> = serde_json::from_slice(&decompressed)
        .with_context(|| {
            format!(
                "Failed to parse audit response: {}",
                String::from_utf8_lossy(&decompressed)
            )
        })?;

    let mut count = VulnerabilityCount::default();
    for list in advisories.values() {
        for adv in list {
            count.total += 1;
            let sev = adv
                .severity
                .as_deref()
                .map(Severity::from_str_loose)
                .unwrap_or(Severity::Low);
            match sev {
                Severity::Info => count.info += 1,
                Severity::Low => count.low += 1,
                Severity::Moderate => count.moderate += 1,
                Severity::High => count.high += 1,
                Severity::Critical => count.critical += 1,
            }
        }
    }

    Ok(AuditReport {
        advisories,
        metadata: AuditMetadata {
            total_dependencies: total_deps,
            vulnerabilities: count,
        },
    })
}
