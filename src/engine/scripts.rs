use anyhow::{Context, Result, anyhow};
use std::env;
use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::package::manifest::PackageJson;

/// Run package.json script (e.g. `bolt run build`)
pub fn run_script(
    project_dir: &Path,
    script_name: &str,
    extra_args: &[String],
) -> Result<ExitStatus> {
    let pkg_json_path = project_dir.join("package.json");
    let pkg_json = PackageJson::from_path(&pkg_json_path)?;

    let scripts = pkg_json
        .scripts
        .ok_or_else(|| anyhow!("No scripts found in package.json"))?;
    let script_cmd = scripts
        .get(script_name)
        .ok_or_else(|| anyhow!("Missing script '{}' in package.json", script_name))?;

    // Combine local node_modules/.bin onto PATH
    let node_modules_bin = project_dir.join("node_modules").join(".bin");
    let path_env = get_extended_path(&node_modules_bin);

    // Full command string including passed extra arguments
    let full_command = if extra_args.is_empty() {
        script_cmd.clone()
    } else {
        format!("{} {}", script_cmd, extra_args.join(" "))
    };

    println!(
        "> {}@{} {}",
        pkg_json.name.as_deref().unwrap_or(""),
        pkg_json.version.as_deref().unwrap_or(""),
        script_name
    );
    println!("> {}", full_command);

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd.exe");
        c.arg("/d").arg("/c").arg(&full_command);
        c
    };

    #[cfg(not(windows))]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(&full_command);
        c
    };

    cmd.current_dir(project_dir);
    cmd.env("PATH", &path_env);

    let status = cmd.status().context("Failed to spawn script process")?;
    Ok(status)
}

/// Execute a binary located in `node_modules/.bin` or run directly (bolt exec <cmd>)
pub fn exec_binary(project_dir: &Path, bin_name: &str, args: &[String]) -> Result<ExitStatus> {
    let node_modules_bin = project_dir.join("node_modules").join(".bin");
    let path_env = get_extended_path(&node_modules_bin);

    #[cfg(windows)]
    let bin_to_run = {
        let cmd_file = node_modules_bin.join(format!("{}.cmd", bin_name));
        let ps1_file = node_modules_bin.join(format!("{}.ps1", bin_name));
        let raw_file = node_modules_bin.join(bin_name);

        if cmd_file.exists() {
            cmd_file.to_string_lossy().to_string()
        } else if raw_file.exists() {
            raw_file.to_string_lossy().to_string()
        } else if ps1_file.exists() {
            ps1_file.to_string_lossy().to_string()
        } else {
            bin_name.to_string()
        }
    };

    #[cfg(not(windows))]
    let bin_to_run = {
        let raw_file = node_modules_bin.join(bin_name);
        if raw_file.exists() {
            raw_file.to_string_lossy().to_string()
        } else {
            bin_name.to_string()
        }
    };

    let mut cmd = Command::new(&bin_to_run);
    cmd.args(args);
    cmd.current_dir(project_dir);
    cmd.env("PATH", &path_env);

    let status = cmd
        .status()
        .with_context(|| format!("Failed to execute command {:?}", bin_to_run))?;
    Ok(status)
}

fn get_extended_path(bin_dir: &Path) -> String {
    let current_path = env::var("PATH").unwrap_or_default();
    let bin_str = bin_dir.to_string_lossy();
    #[cfg(windows)]
    {
        format!("{};{}", bin_str, current_path)
    }
    #[cfg(not(windows))]
    {
        format!("{}:{}", bin_str, current_path)
    }
}
