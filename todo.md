# Bolt - Project Roadmap & TODO

This document tracks completed features, capabilities, and upcoming engineering milestones for **Bolt**, a high-performance npm-compatible package manager written in Rust.

---

## Completed Milestones

### 1. CLI and Manifest Management
- [x] Full CLI argument parser using `clap` (`install [packages]`, `install -D / -O / --save-peer`, `ci`, `uninstall`, `update [packages]`, `run <script>`, `exec <command>`).
- [x] Workspace targeting flags (`-w / --workspace <member>`, `--workspaces`).
- [x] Uncached execution confirmation prompt (`bolt exec` with `-y / --yes` bypass).
- [x] Manifest engine (`package.json`) with round-trip preservation of unknown/unrelated fields (`#[serde(flatten)]`).
- [x] Extraction and parsing of package `bin` declarations (single string or key-value object).
- [x] Clean uninstall and prune logic updating manifests and lockfiles.

### 2. npm Semver & Dependency Resolution
- [x] npm-compatible semver parsing (`^`, `~`, exact, wildcards, tags like `latest`).
- [x] Pre-release version exclusion rules matching npm semantics.
- [x] Scoped package specifiers (`@scope/pkg@^1.0.0`).
- [x] npm alias dependencies (`npm:real-pkg@^2.0.0`).
- [x] Local file dependencies (`file:../local-pkg`).
- [x] Git repository dependencies (`github:user/repo`, `git+https://...`, `git+file://...`, commit/branch/tag ref checking).
- [x] Direct tarball URL dependencies (`https://.../pkg.tgz`).
- [x] Selective updating (`bolt update <pkg>` updates specified packages while keeping all other locked dependencies pinned).
- [x] Platform constraint evaluation: OS (`win32`, `darwin`, `linux`, `!win32`) and CPU (`x64`, `arm64`, `!x64`).
- [x] Optional and peer dependency handling (platform-incompatible optional dependencies are skipped gracefully).
- [x] Full peer dependency auto-installation matching npm v7+ tree resolution algorithms with `peerDependenciesMeta` support.
- [x] Dependency cycle detection breaking recursion loops.
- [x] Nested conflict resolution (conflict packages nested under parent `node_modules` while compatible dependencies stay hoisted).

### 3. Lockfile Engine (v2 / v3)
- [x] Parse npm `package-lock.json` v2 and v3.
- [x] Serialize clean npm v3 format with complete package path keys (`node_modules/...`).
- [x] Lockfile synchronization check for `bolt ci` (rejects uncommitted manifest/lockfile drift without mutation).
- [x] Automatic upgrade of legacy v1/v2 lockfiles to v3 without duplicating legacy `dependencies` objects.
- [x] Full interoperability verified against npm (`npm ci` cleanly parses and installs Bolt's generated `package-lock.json`).

### 4. Concurrent Installation & Verification
- [x] Asynchronous HTTP client using `reqwest` and `tokio`.
- [x] HTTP/2 multiplexing pipeline with adaptive window sizing.
- [x] Bounded concurrency with Tokio semaphores and retry backoff.
- [x] Multi-tier caching: in-memory metadata cache and disk cache (`%LOCALAPPDATA%/bolt/cache`, `~/.cache/bolt`, or `BOLT_CACHE_DIR`).
- [x] Hard-link / Content-addressable store (CAS) installation mode (`--hardlink`) linking packages from a shared cache store.
- [x] SHA-512 and SHA-1 base64/hex integrity validation before unpacking.
- [x] Safe tarball extraction rejecting directory traversal (`..`) and absolute paths, stripping root `package/` prefix.
- [x] Windows file lock retry handlers for antivirus or indexing contention (`remove_dir_all_with_retry`, `copy_file_with_retry`).
- [x] Cross-platform binary shims (`.cmd`, `.ps1`, and Unix `sh`) placed into `node_modules/.bin`.

### 5. Workspaces and Lifecycle Execution
- [x] Monorepo workspace discovery from root `package.json` (`workspaces: ["packages/*"]`).
- [x] Monorepo linking via cross-platform directory junctions/symlinks in `node_modules`.
- [x] Workspace targeted execution (`bolt -w <workspace-member> run <script>`).
- [x] Workspace recursive command execution (`bolt --workspaces run <script>`).
- [x] Dependency hoisting optimization across heterogeneous multi-package workspaces (`resolve_manifests`).
- [x] Execution of `preinstall`, `install`, and `postinstall` lifecycle scripts with local `.bin` injected into `PATH`.
- [x] Support for `--ignore-scripts` to skip script execution.
- [x] `.npmrc` configuration reader with scoped registry authentication (`Bearer <token>`).
- [x] Automated benchmark harness with historical regression tracking dashboard (`benches/benchmark.mjs` $\to$ `benches/results.json`).

---

## Completed Milestones (Recent Extensions)

### Core Extensions
- [x] Support for npm overrides / pnpm resolutions in `package.json` to force specific dependency versions across sub-trees.
- [x] Offline-only cache mode (`--offline`) completely bypassing network lookups when local tarballs exist.
- [x] Catalog protocol support for shared workspace version definitions (`catalog:default`).

### Audit & Security
- [x] `bolt audit` command verifying installed packages against the npm Security Advisory database.
- [x] Software Bill of Materials (SBOM) generation (CycloneDX / SPDX JSON export).

### Developer Experience & Inspection
- [x] `bolt outdated` command comparing installed/locked packages against registry wanted and latest semver tags.
- [x] `bolt why` / `bolt explain` command analyzing dependency trees and explaining why packages are installed (direct vs. transitive chain).

### Packaging & Release Automation
- [x] Multi-platform GitHub Actions release workflow (`.github/workflows/release.yml`) targeting Linux (x86_64, aarch64), macOS (Intel, Apple Silicon), and Windows (x86_64).
- [x] Polished NSIS installer packaging `bolt.exe`, `readme.md`, and `LICENSE` with automated checksum generation.
