use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(
    name = "bolt",
    version,
    about = "A fast, npm-compatible package manager in Rust"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Path to package.json directory (workspace root or project directory)
    #[arg(short = 'C', long, global = true)]
    pub cwd: Option<PathBuf>,

    /// Workspace package to target (e.g. -w @my-scope/pkg or -w packages/pkg-a)
    #[arg(short = 'w', long = "workspace", global = true)]
    pub workspace: Option<String>,

    /// Target all workspace packages
    #[arg(long = "workspaces", global = true)]
    pub workspaces: bool,

    /// Offline mode: use only cached packages
    #[arg(long, global = true)]
    pub offline: bool,

    /// Custom npm registry URL
    #[arg(long, global = true)]
    pub registry: Option<String>,

    /// Verbose output
    #[arg(short, long, global = true)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Install dependencies according to package.json or add new packages
    Install {
        /// Package specifiers to install (e.g. `express`, `lodash@^4.17.21`, `@types/node@latest`)
        packages: Vec<String>,

        /// Save package to devDependencies (-D / --save-dev)
        #[arg(short = 'D', long = "save-dev")]
        save_dev: bool,

        /// Save package to optionalDependencies (-O / --save-optional)
        #[arg(short = 'O', long = "save-optional")]
        save_optional: bool,

        /// Save package to peerDependencies
        #[arg(long = "save-peer")]
        save_peer: bool,

        /// Save package to regular dependencies (default)
        #[arg(short = 'P', long = "save-prod")]
        save_prod: bool,

        /// Do not save installed packages to package.json
        #[arg(long = "no-save")]
        no_save: bool,

        /// Skip running lifecycle scripts
        #[arg(long = "ignore-scripts")]
        ignore_scripts: bool,

        /// Dry run: resolve and calculate lockfile/actions without writing files
        #[arg(long)]
        dry_run: bool,

        /// Use hard-link content-addressable store (CAS) mode
        #[arg(long)]
        hardlink: bool,
    },

    /// Install dependencies strictly adhering to package-lock.json (clean install)
    Ci {
        /// Skip running lifecycle scripts
        #[arg(long = "ignore-scripts")]
        ignore_scripts: bool,
    },

    /// Remove packages from package.json and node_modules
    Uninstall {
        /// Package names to uninstall
        packages: Vec<String>,

        /// Do not save removal to package.json
        #[arg(long = "no-save")]
        no_save: bool,
    },

    /// Update packages to their latest versions matching semver in package.json
    Update {
        /// Optional specific package names to update
        packages: Vec<String>,
    },

    /// Run an arbitrary package script defined in package.json
    Run {
        /// Script name to execute
        script: String,

        /// Arguments passed to the script
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Execute a package binary (from local node_modules/.bin or npx-style)
    Exec {
        /// Binary name or command to execute
        command: String,

        /// Arguments passed to the command
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,

        /// Automatically proceed without confirmation prompt if package needs download
        #[arg(short = 'y', long = "yes")]
        yes: bool,
    },

    /// Scan dependencies for known security vulnerabilities via npm Security Advisory database
    Audit {
        /// Output format: 'json' or 'table' (default)
        #[arg(long, default_value = "table")]
        format: String,

        /// Exit with a non-zero code only if vulnerabilities match or exceed the specified severity
        #[arg(long, default_value = "low")]
        audit_level: String,

        /// Only audit production dependencies
        #[arg(long)]
        omit_dev: bool,
    },

    /// Generate a Software Bill of Materials (SBOM) in CycloneDX or SPDX JSON format
    Sbom {
        /// SBOM specification format: 'cyclonedx' or 'spdx'
        #[arg(long, default_value = "cyclonedx")]
        format: String,

        /// Output file path (defaults to stdout if omitted)
        #[arg(short = 'o', long = "output")]
        output: Option<std::path::PathBuf>,
    },
}
