use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

use crate::package::lockfile::PackageLock;
use crate::package::manifest::PackageJson;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycloneDxBom {
    pub bom_format: String,
    pub spec_version: String,
    pub serial_number: String,
    pub version: u32,
    pub metadata: CycloneDxMetadata,
    pub components: Vec<CycloneDxComponent>,
    pub dependencies: Vec<CycloneDxDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycloneDxMetadata {
    pub timestamp: String,
    pub tools: serde_json::Value,
    pub component: CycloneDxComponent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycloneDxComponent {
    #[serde(rename = "type")]
    pub component_type: String,
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bom_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CycloneDxDependency {
    pub r#ref: String,
    pub depends_on: Vec<String>,
}

pub fn generate_cyclonedx(manifest: &PackageJson, lockfile: &PackageLock) -> Result<CycloneDxBom> {
    let root_name = manifest.name.clone().unwrap_or_else(|| "app".to_string());
    let root_ver = manifest
        .version
        .clone()
        .unwrap_or_else(|| "1.0.0".to_string());
    let root_purl = format!("pkg:npm/{}@{}", root_name.replace('/', "%2F"), root_ver);

    let root_component = CycloneDxComponent {
        component_type: "application".to_string(),
        name: root_name.clone(),
        version: root_ver.clone(),
        purl: Some(root_purl.clone()),
        bom_ref: Some(root_purl.clone()),
        description: manifest.description.clone(),
    };

    let mut components = Vec::new();
    let mut component_purls: BTreeMap<String, String> = BTreeMap::new();

    for (path, pkg) in &lockfile.packages {
        if path.is_empty() {
            continue;
        }

        let name = if let Some(n) = &pkg.name {
            n.clone()
        } else if let Some(stripped) = path.strip_prefix("node_modules/") {
            stripped
                .split("/node_modules/")
                .last()
                .unwrap_or(stripped)
                .to_string()
        } else {
            path.clone()
        };

        let ver = pkg.version.clone().unwrap_or_else(|| "0.0.0".to_string());
        let encoded_name = if let Some(stripped) = name.strip_prefix('@') {
            if let Some((scope, pkg_n)) = stripped.split_once('/') {
                format!("%40{}%2F{}", scope, pkg_n)
            } else {
                name.clone()
            }
        } else {
            name.clone()
        };
        let purl = format!("pkg:npm/{}@{}", encoded_name, ver);

        component_purls.insert(path.clone(), purl.clone());

        components.push(CycloneDxComponent {
            component_type: "library".to_string(),
            name,
            version: ver,
            purl: Some(purl.clone()),
            bom_ref: Some(purl),
            description: None,
        });
    }

    let mut dependencies = Vec::new();
    let mut root_depends = Vec::new();
    if let Some(root_pkg) = lockfile.packages.get("") {
        if let Some(deps) = &root_pkg.dependencies {
            for dep_name in deps.keys() {
                let target_path = format!("node_modules/{}", dep_name);
                if let Some(purl) = component_purls.get(&target_path) {
                    root_depends.push(purl.clone());
                }
            }
        }
    }
    dependencies.push(CycloneDxDependency {
        r#ref: root_purl,
        depends_on: root_depends,
    });

    for (path, pkg) in &lockfile.packages {
        if path.is_empty() {
            continue;
        }
        if let Some(purl) = component_purls.get(path) {
            let mut dep_refs = Vec::new();
            if let Some(deps) = &pkg.dependencies {
                for dep_name in deps.keys() {
                    let direct_child = format!("{}/node_modules/{}", path, dep_name);
                    let top_child = format!("node_modules/{}", dep_name);
                    if let Some(child_purl) = component_purls
                        .get(&direct_child)
                        .or_else(|| component_purls.get(&top_child))
                    {
                        dep_refs.push(child_purl.clone());
                    }
                }
            }
            dependencies.push(CycloneDxDependency {
                r#ref: purl.clone(),
                depends_on: dep_refs,
            });
        }
    }

    Ok(CycloneDxBom {
        bom_format: "CycloneDX".to_string(),
        spec_version: "1.5".to_string(),
        serial_number: format!(
            "urn:uuid:{}",
            hex::encode(&sha2::Sha256::digest(root_name.as_bytes())[..16])
        ),
        version: 1,
        metadata: CycloneDxMetadata {
            timestamp: "2026-09-17T00:00:00Z".to_string(),
            tools: json!([
                {
                    "vendor": "Bolt",
                    "name": "bolt",
                    "version": env!("CARGO_PKG_VERSION")
                }
            ]),
            component: root_component,
        },
        components,
        dependencies,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxDocument {
    pub spdx_version: String,
    pub data_license: String,
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    pub name: String,
    pub document_namespace: String,
    pub creation_info: SpdxCreationInfo,
    pub packages: Vec<SpdxPackage>,
    pub relationships: Vec<SpdxRelationship>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxCreationInfo {
    pub created: String,
    pub creators: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxPackage {
    pub name: String,
    #[serde(rename = "SPDXID")]
    pub spdx_id: String,
    pub version_info: String,
    pub download_location: String,
    pub files_analyzed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_refs: Option<Vec<SpdxExternalRef>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxExternalRef {
    pub reference_category: String,
    pub reference_type: String,
    pub reference_locator: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpdxRelationship {
    pub spdx_element_id: String,
    pub relationship_type: String,
    pub related_spdx_element: String,
}

use sha2::Digest;

pub fn generate_spdx(manifest: &PackageJson, lockfile: &PackageLock) -> Result<SpdxDocument> {
    let root_name = manifest.name.clone().unwrap_or_else(|| "app".to_string());
    let root_ver = manifest
        .version
        .clone()
        .unwrap_or_else(|| "1.0.0".to_string());
    let root_spdx_id = "SPDXRef-RootPackage".to_string();

    let root_pkg = SpdxPackage {
        name: root_name.clone(),
        spdx_id: root_spdx_id.clone(),
        version_info: root_ver.clone(),
        download_location: "NOASSERTION".to_string(),
        files_analyzed: false,
        external_refs: Some(vec![SpdxExternalRef {
            reference_category: "PACKAGE-MANAGER".to_string(),
            reference_type: "purl".to_string(),
            reference_locator: format!("pkg:npm/{}@{}", root_name.replace('/', "%2F"), root_ver),
        }]),
    };

    let mut packages = vec![root_pkg];
    let mut relationships = Vec::new();
    let mut id_counter = 0;

    relationships.push(SpdxRelationship {
        spdx_element_id: "SPDXRef-DOCUMENT".to_string(),
        relationship_type: "DESCRIBES".to_string(),
        related_spdx_element: root_spdx_id.clone(),
    });

    for (path, pkg) in &lockfile.packages {
        if path.is_empty() {
            continue;
        }

        let name = if let Some(n) = &pkg.name {
            n.clone()
        } else if let Some(stripped) = path.strip_prefix("node_modules/") {
            stripped
                .split("/node_modules/")
                .last()
                .unwrap_or(stripped)
                .to_string()
        } else {
            path.clone()
        };

        let ver = pkg.version.clone().unwrap_or_else(|| "0.0.0".to_string());
        id_counter += 1;
        let spdx_id = format!("SPDXRef-Package-{}", id_counter);

        let encoded_name = if let Some(stripped) = name.strip_prefix('@') {
            if let Some((scope, pkg_n)) = stripped.split_once('/') {
                format!("%40{}%2F{}", scope, pkg_n)
            } else {
                name.clone()
            }
        } else {
            name.clone()
        };
        let purl = format!("pkg:npm/{}@{}", encoded_name, ver);

        let download_loc = pkg
            .resolved
            .clone()
            .unwrap_or_else(|| "NOASSERTION".to_string());

        packages.push(SpdxPackage {
            name: name.clone(),
            spdx_id: spdx_id.clone(),
            version_info: ver,
            download_location: download_loc,
            files_analyzed: false,
            external_refs: Some(vec![SpdxExternalRef {
                reference_category: "PACKAGE-MANAGER".to_string(),
                reference_type: "purl".to_string(),
                reference_locator: purl,
            }]),
        });

        relationships.push(SpdxRelationship {
            spdx_element_id: root_spdx_id.clone(),
            relationship_type: "DEPENDS_ON".to_string(),
            related_spdx_element: spdx_id,
        });
    }

    Ok(SpdxDocument {
        spdx_version: "SPDX-2.3".to_string(),
        data_license: "CC0-1.0".to_string(),
        spdx_id: "SPDXRef-DOCUMENT".to_string(),
        name: root_name.clone(),
        document_namespace: format!(
            "https://spdx.org/spdxdocs/{}-{}",
            root_name,
            hex::encode(&sha2::Sha256::digest(root_name.as_bytes())[..8])
        ),
        creation_info: SpdxCreationInfo {
            created: "2026-09-17T00:00:00Z".to_string(),
            creators: vec![format!("Tool: bolt-{}", env!("CARGO_PKG_VERSION"))],
        },
        packages,
        relationships,
    })
}
