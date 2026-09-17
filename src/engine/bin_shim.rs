use crate::package::manifest::PackageJson;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

/// Generate executable bin shims in `node_modules/.bin`
/// On Windows: generates `.cmd` and PowerShell `.ps1` alongside shell shim.
/// On Unix: generates executable shell script (`#!/bin/sh`).
pub fn create_bin_shims(
    node_modules_bin_dir: &Path,
    package_dir: &Path,
    pkg_manifest: &PackageJson,
) -> Result<()> {
    let bins = pkg_manifest.get_bins();
    if bins.is_empty() {
        return Ok(());
    }

    fs::create_dir_all(node_modules_bin_dir)
        .with_context(|| format!("Failed to create bin dir {:?}", node_modules_bin_dir))?;

    for (bin_name, rel_bin_target) in bins {
        let bin_target_path = package_dir.join(&rel_bin_target);

        // Compute relative path from bin dir to target script
        let target_relative_str = match pathdiff::diff_paths(&bin_target_path, node_modules_bin_dir)
        {
            Some(diff) => diff.to_string_lossy().replace('\\', "/"),
            None => bin_target_path.to_string_lossy().replace('\\', "/"),
        };

        // 1. Unix shell script shim
        let sh_script = format!(
            r#"#!/bin/sh
basedir=$(dirname "$(echo "$0" | sed -e 's,\\,/,g')")

case `uname` in
    *CYGWIN*|*MINGW*|*MSYS*)
        if command -v cygpath > /dev/null 2>&1; then
            basedir=`cygpath -w "$basedir"`
        fi
    ;;
esac

if [ -x "$basedir/node" ]; then
  exec "$basedir/node" "$basedir/{}" "$@"
else
  exec node "$basedir/{}" "$@"
fi
"#,
            target_relative_str, target_relative_str
        );

        let sh_path = node_modules_bin_dir.join(&bin_name);
        fs::write(&sh_path, sh_script)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&sh_path, fs::Permissions::from_mode(0o755));
        }

        // 2. Windows .cmd shim
        let target_win_rel = target_relative_str.replace('/', "\\");
        let cmd_script = format!(
            r#"@ECHO off
GOTO start
:find_dp0
SET dp0=%~dp0
EXIT /b
:start
SETLOCAL
CALL :find_dp0

IF EXIST "%dp0%\node.exe" (
  SET "_prog=%dp0%\node.exe"
) ELSE (
  SET "_prog=node"
  SET PATHEXT=%PATHEXT:;.JS;=;%
)

endLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & "%_prog%"  "%dp0%\{}" %*
"#,
            target_win_rel
        );

        let cmd_path = node_modules_bin_dir.join(format!("{}.cmd", bin_name));
        fs::write(&cmd_path, cmd_script)?;

        // 3. PowerShell .ps1 shim
        let ps1_script = format!(
            r#"#!/usr/bin/env pwsh
$basedir = Split-Path $MyInvocation.MyCommand.Definition -Parent

$exe = ""
if ($PSVersionTable.PSVersion -lt "6.0" -or $IsWindows) {{
  # Windows
  if (Test-Path "$basedir/node.exe") {{
    $exe = "$basedir/node.exe"
  }} else {{
    $exe = "node"
  }}
}} else {{
  # Non-Windows
  if (Test-Path "$basedir/node") {{
    $exe = "$basedir/node"
  }} else {{
    $exe = "node"
  }}
}}

$ret = 0
if (Test-Path "$basedir/{}") {{
  & "$exe" "$basedir/{}" $args
  $ret = $LASTEXITCODE
}} else {{
  & "$exe" "{}" $args
  $ret = $LASTEXITCODE
}}
exit $ret
"#,
            target_relative_str, target_relative_str, target_relative_str
        );

        let ps1_path = node_modules_bin_dir.join(format!("{}.ps1", bin_name));
        fs::write(&ps1_path, ps1_script)?;
    }

    Ok(())
}
