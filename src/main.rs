use anyhow::{Context, Result, anyhow};
use clap::Parser;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::exit;

use bolt::cli::{Cli, Commands};
use bolt::config::NpmConfig;
use bolt::engine::installer::Installer;
use bolt::engine::scripts::{exec_binary, run_script};
use bolt::network::registry::RegistryClient;
use bolt::package::lockfile::PackageLock;
use bolt::package::manifest::{DependencyType, PackageJson};
use bolt::package::semver::parse_package_spec;
use bolt::resolver::Resolver;
use bolt::workspace::{discover_workspaces, find_workspace_member, link_workspaces};

#[tokio::main]
async fn main() {
    if let Err(err) = run().await {
        eprintln!("bolt error: {:#}", err);
        exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    let root_dir = if let Some(cwd) = &cli.cwd {
        cwd.clone()
    } else {
        env::current_dir().context("Failed to determine current directory")?
    };

    let npm_config = NpmConfig::load_from_env_and_files(&root_dir);
    let registry_client =
        RegistryClient::new_opt(cli.registry.clone(), None, Some(npm_config), cli.offline);

    // Handle workspace target if specified (-w / --workspace)
    let project_dir = if let Some(ws_target) = &cli.workspace {
        let root_pkg_json = root_dir.join("package.json");
        if root_pkg_json.exists() {
            let root_manifest = PackageJson::from_path(&root_pkg_json)?;
            let members = discover_workspaces(&root_dir, &root_manifest)?;
            if let Some(member) = find_workspace_member(&members, ws_target) {
                member.path.clone()
            } else {
                return Err(anyhow!("Workspace member '{}' not found", ws_target));
            }
        } else {
            root_dir.clone()
        }
    } else {
        root_dir.clone()
    };

    match cli.command.unwrap_or(Commands::Install {
        packages: vec![],
        save_dev: false,
        save_optional: false,
        save_peer: false,
        save_prod: false,
        no_save: false,
        ignore_scripts: false,
        dry_run: false,
        hardlink: false,
    }) {
        Commands::Install {
            packages,
            save_dev,
            save_optional,
            save_peer,
            save_prod: _,
            no_save,
            ignore_scripts,
            dry_run,
            hardlink,
        } => {
            handle_install(
                &project_dir,
                &registry_client,
                packages,
                save_dev,
                save_optional,
                save_peer,
                no_save,
                ignore_scripts,
                dry_run,
                hardlink,
            )
            .await?;
        }
        Commands::Ci { ignore_scripts } => {
            handle_ci(&project_dir, &registry_client, ignore_scripts).await?;
        }

        Commands::Uninstall { packages, no_save } => {
            handle_uninstall(&project_dir, &registry_client, packages, no_save).await?;
        }

        Commands::Update { packages } => {
            handle_update(&project_dir, &registry_client, packages).await?;
        }

        Commands::Run { script, args } => {
            if cli.workspaces {
                let root_pkg_json = root_dir.join("package.json");
                let mut members = Vec::new();
                if root_pkg_json.exists() {
                    let root_manifest = PackageJson::from_path(&root_pkg_json)?;
                    members = discover_workspaces(&root_dir, &root_manifest)?;
                }

                if members.is_empty() {
                    println!("No workspaces found to run '{}'", script);
                } else {
                    for member in &members {
                        if let Some(scripts) = &member.manifest.scripts
                            && scripts.contains_key(&script)
                        {
                            println!(
                                "> {}@{}: run {}",
                                member.name,
                                member.manifest.version.as_deref().unwrap_or("1.0.0"),
                                script
                            );
                            let status = run_script(&member.path, &script, &args)?;
                            if !status.success() {
                                exit(status.code().unwrap_or(1));
                            }
                        }
                    }
                }
            } else {
                let status = run_script(&project_dir, &script, &args)?;
                if !status.success() {
                    exit(status.code().unwrap_or(1));
                }
            }
        }

        Commands::Exec { command, args, yes } => {
            handle_exec(&project_dir, &registry_client, &command, &args, yes).await?;
        }

        Commands::Audit {
            format,
            audit_level,
            omit_dev,
        } => {
            handle_audit(
                &project_dir,
                &registry_client,
                &format,
                &audit_level,
                omit_dev,
            )
            .await?;
        }

        Commands::Sbom { format, output } => {
            handle_sbom(&project_dir, &format, output.as_deref())?;
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_install(
    project_dir: &Path,
    registry: &RegistryClient,
    packages: Vec<String>,
    save_dev: bool,
    save_optional: bool,
    save_peer: bool,
    no_save: bool,
    ignore_scripts: bool,
    dry_run: bool,
    hardlink: bool,
) -> Result<()> {
    let pkg_json_path = project_dir.join("package.json");
    let mut manifest = if pkg_json_path.exists() {
        PackageJson::from_path(&pkg_json_path)?
    } else {
        PackageJson {
            name: project_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string()),
            version: Some("1.0.0".to_string()),
            ..Default::default()
        }
    };

    let lock_path = project_dir.join("package-lock.json");
    let existing_lock = if lock_path.exists() {
        PackageLock::from_path(&lock_path).ok()
    } else {
        None
    };

    let dep_type = if save_dev {
        DependencyType::Dev
    } else if save_optional {
        DependencyType::Optional
    } else if save_peer {
        DependencyType::Peer
    } else {
        DependencyType::Prod
    };

    if !packages.is_empty() {
        for pkg_spec in &packages {
            let (pkg_name, req) = parse_package_spec(pkg_spec)?;
            let actual_req = if req == "latest" {
                let meta = registry.fetch_package_metadata(&pkg_name).await?;
                if let Some(latest_ver) = meta.dist_tags.get("latest") {
                    format!("^{}", latest_ver)
                } else {
                    "^1.0.0".to_string()
                }
            } else if req.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                format!("^{}", req)
            } else {
                req
            };

            manifest.add_dependency(pkg_name, actual_req, dep_type);
        }
    }

    println!("Resolving dependencies...");
    let resolver = Resolver::new(project_dir.to_path_buf(), registry.clone(), existing_lock);

    let workspace_members = discover_workspaces(project_dir, &manifest)?;
    let resolved = if !workspace_members.is_empty() {
        let mut all_manifests = vec![manifest.clone()];
        for m in &workspace_members {
            all_manifests.push(m.manifest.clone());
        }
        resolver.resolve_manifests(&all_manifests).await?
    } else {
        resolver
            .resolve_manifests(std::slice::from_ref(&manifest))
            .await?
    };

    println!("Resolved {} packages.", resolved.len());

    let lockfile = Resolver::build_lockfile(&manifest, &resolved);

    if dry_run {
        println!("Dry run complete. No files written.");
        return Ok(());
    }

    println!("Installing packages...");
    let installer = Installer::new(
        project_dir.to_path_buf(),
        registry.clone(),
        ignore_scripts,
        hardlink,
    );
    installer.install_all(&resolved).await?;
    // Handle workspaces linking if monorepo
    let workspace_members = discover_workspaces(project_dir, &manifest)?;
    if !workspace_members.is_empty() {
        println!("Linking {} workspace packages...", workspace_members.len());
        link_workspaces(project_dir, &workspace_members)?;
    }

    if !no_save {
        manifest.save(&pkg_json_path)?;
    }
    lockfile.save(&lock_path)?;

    println!("Success! Installed {} packages.", resolved.len());
    Ok(())
}

async fn handle_ci(
    project_dir: &Path,
    registry: &RegistryClient,
    ignore_scripts: bool,
) -> Result<()> {
    let pkg_json_path = project_dir.join("package.json");
    if !pkg_json_path.exists() {
        return Err(anyhow!("package.json not found for ci"));
    }
    let manifest = PackageJson::from_path(&pkg_json_path)?;

    let lock_path = project_dir.join("package-lock.json");
    if !lock_path.exists() {
        return Err(anyhow!(
            "package-lock.json not found for ci. Use 'bolt install' to create one."
        ));
    }
    let lockfile = PackageLock::from_path(&lock_path)?;

    #[allow(clippy::collapsible_if)]
    if let Some(root_lock) = lockfile.packages.get("") {
        if manifest.dependencies != root_lock.dependencies
            || manifest.dev_dependencies != root_lock.dev_dependencies
        {
            return Err(anyhow!(
                "package.json and package-lock.json do not match. 'bolt ci' strictly requires lockfile synchronization."
            ));
        }
    }

    let node_modules_dir = project_dir.join("node_modules");
    if node_modules_dir.exists() {
        fs::remove_dir_all(&node_modules_dir)?;
    }

    let resolver = Resolver::new(
        project_dir.to_path_buf(),
        registry.clone(),
        Some(lockfile.clone()),
    );
    let resolved = resolver.resolve_manifest(&manifest).await?;

    let installer = Installer::new(
        project_dir.to_path_buf(),
        registry.clone(),
        ignore_scripts,
        false,
    );
    installer.install_all(&resolved).await?;
    let workspace_members = discover_workspaces(project_dir, &manifest)?;
    if !workspace_members.is_empty() {
        link_workspaces(project_dir, &workspace_members)?;
    }

    println!(
        "bolt ci complete: installed {} packages strictly from lockfile.",
        resolved.len()
    );
    Ok(())
}

async fn handle_uninstall(
    project_dir: &Path,
    registry: &RegistryClient,
    packages: Vec<String>,
    no_save: bool,
) -> Result<()> {
    let pkg_json_path = project_dir.join("package.json");
    let mut manifest = PackageJson::from_path(&pkg_json_path)?;

    for pkg_name in &packages {
        manifest.remove_dependency(pkg_name);
    }

    let resolver = Resolver::new(project_dir.to_path_buf(), registry.clone(), None);
    let resolved = resolver.resolve_manifest(&manifest).await?;

    let lock_path = project_dir.join("package-lock.json");
    let lockfile = Resolver::build_lockfile(&manifest, &resolved);

    let _installer = Installer::new(project_dir.to_path_buf(), registry.clone(), false, false);

    if !no_save {
        manifest.save(&pkg_json_path)?;
    }
    lockfile.save(&lock_path)?;

    println!("Uninstalled {:?}.", packages);
    Ok(())
}

async fn handle_update(
    project_dir: &Path,
    registry: &RegistryClient,
    packages: Vec<String>,
) -> Result<()> {
    let pkg_json_path = project_dir.join("package.json");
    let manifest = PackageJson::from_path(&pkg_json_path)?;

    let lock_path = project_dir.join("package-lock.json");
    let mut existing_lock = if lock_path.exists() {
        PackageLock::from_path(&lock_path).ok()
    } else {
        None
    };

    // Selective update: if specific packages provided, remove them from existing_lock to allow resolving latest
    if !packages.is_empty() {
        if let Some(lock) = &mut existing_lock {
            for pkg_name in &packages {
                let target_path = format!("node_modules/{}", pkg_name);
                lock.packages.remove(&target_path);
            }
        }
    } else {
        // Full update: discard entire existing lock constraint
        existing_lock = None;
    }

    let resolver = Resolver::new(project_dir.to_path_buf(), registry.clone(), existing_lock);
    let resolved = resolver.resolve_manifest(&manifest).await?;

    let lockfile = Resolver::build_lockfile(&manifest, &resolved);

    let _installer = Installer::new(project_dir.to_path_buf(), registry.clone(), false, false);

    lockfile.save(&lock_path)?;

    println!("Updated dependencies. Total installed: {}", resolved.len());
    Ok(())
}

async fn handle_exec(
    project_dir: &Path,
    _registry: &RegistryClient,
    command: &str,
    args: &[String],
    yes: bool,
) -> Result<()> {
    let node_modules_bin = project_dir.join("node_modules").join(".bin");
    let exists_locally = node_modules_bin.join(command).exists()
        || node_modules_bin.join(format!("{}.cmd", command)).exists();

    if !exists_locally && !yes {
        print!(
            "Need to install the following package: {}.\nOk to proceed? (y) ",
            command
        );
        let _ = io::stdout().flush();
        let mut input = String::new();
        if io::stdin().read_line(&mut input).is_ok() {
            let trimmed = input.trim().to_lowercase();
            if trimmed != "y" && trimmed != "yes" && !trimmed.is_empty() {
                println!("Operation cancelled.");
                return Ok(());
            }
        }
    }

    let status = exec_binary(project_dir, command, args)?;
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }

    Ok(())
}

async fn handle_audit(
    project_dir: &Path,
    registry: &RegistryClient,
    format: &str,
    audit_level: &str,
    omit_dev: bool,
) -> Result<()> {
    let lock_path = project_dir.join("package-lock.json");
    if !lock_path.exists() {
        return Err(anyhow!(
            "package-lock.json not found for audit. Run 'bolt install' first."
        ));
    }
    let lockfile = PackageLock::from_path(&lock_path)?;

    let report = bolt::network::audit::run_audit(registry, &lockfile, omit_dev).await?;

    if format == "json" {
        let json_str = serde_json::to_string_pretty(&report)?;
        println!("{}", json_str);
    } else {
        println!(
            "=== Bolt Security Audit (scanned {} dependencies) ===",
            report.metadata.total_dependencies
        );
        if report.advisories.is_empty() {
            println!("found 0 vulnerabilities");
        } else {
            for (pkg_name, advisories) in &report.advisories {
                for adv in advisories {
                    let sev = adv.severity.as_deref().unwrap_or("unknown");
                    let title = adv.title.as_deref().unwrap_or("No title");
                    let vulnerable_range = adv.vulnerable_versions.as_deref().unwrap_or("*");
                    let url = adv.url.as_deref().unwrap_or("");
                    println!("\nPackage: {}", pkg_name);
                    println!("Severity: {}", sev);
                    println!("Title: {}", title);
                    println!("Vulnerable versions: {}", vulnerable_range);
                    if !url.is_empty() {
                        println!("More info: {}", url);
                    }
                }
            }
            let v = &report.metadata.vulnerabilities;
            println!(
                "\nTotal: {} vulnerabilities ({} low, {} moderate, {} high, {} critical)",
                v.total, v.low, v.moderate, v.high, v.critical
            );
        }
    }

    let threshold = bolt::network::audit::Severity::from_str_loose(audit_level);
    let mut should_fail = false;
    for list in report.advisories.values() {
        for adv in list {
            let s = adv
                .severity
                .as_deref()
                .map(bolt::network::audit::Severity::from_str_loose)
                .unwrap_or(bolt::network::audit::Severity::Low);
            if s.rank() >= threshold.rank() {
                should_fail = true;
                break;
            }
        }
        if should_fail {
            break;
        }
    }

    if should_fail {
        exit(1);
    }

    Ok(())
}

fn handle_sbom(project_dir: &Path, format: &str, output_file: Option<&Path>) -> Result<()> {
    let pkg_json_path = project_dir.join("package.json");
    let manifest = if pkg_json_path.exists() {
        PackageJson::from_path(&pkg_json_path)?
    } else {
        PackageJson::default()
    };

    let lock_path = project_dir.join("package-lock.json");
    let lockfile = if lock_path.exists() {
        PackageLock::from_path(&lock_path)?
    } else {
        PackageLock::new_v3(
            manifest.name.clone().unwrap_or_else(|| "app".to_string()),
            manifest.version.clone(),
        )
    };

    let output_content = if format.eq_ignore_ascii_case("spdx") {
        let doc = bolt::package::sbom::generate_spdx(&manifest, &lockfile)?;
        serde_json::to_string_pretty(&doc)?
    } else {
        let bom = bolt::package::sbom::generate_cyclonedx(&manifest, &lockfile)?;
        serde_json::to_string_pretty(&bom)?
    };

    if let Some(out_path) = output_file {
        if let Some(parent) = out_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(out_path, format!("{}\n", output_content))?;
        println!("Generated SBOM ({}) at {:?}", format, out_path);
    } else {
        println!("{}", output_content);
    }

    Ok(())
}
