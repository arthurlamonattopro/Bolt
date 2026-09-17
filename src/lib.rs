#![allow(clippy::collapsible_if)]

pub mod cli;
pub mod config;
pub mod engine;
pub mod network;
pub mod package;
pub mod resolver;
pub mod workspace;

// Re-exports for backwards compatibility and ergonomic public API
pub use cli as cli_mod;
pub use config as config_mod;
pub use engine::bin_shim;
pub use engine::installer;
pub use engine::scripts;
pub use network::audit;
pub use network::registry;
pub use network::tarball;
pub use package::lockfile;
pub use package::manifest;
pub use package::platform;
pub use package::sbom;
pub use package::semver;
pub use resolver as resolver_mod;
pub use workspace as workspace_mod;
