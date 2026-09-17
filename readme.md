# Bolt

[![CI](https://github.com/LamoDev/Bolt/actions/workflows/ci.yml/badge.svg)](https://github.com/LamoDev/Bolt/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

**Bolt** is a fast, lightweight, npm-compatible package manager written in Rust. Designed for predictable, secure, and concurrent package installations across Windows, macOS, and Linux.

---

## ⚡ Why Bolt?

* 🚀 **High Throughput** — Parallel asynchronous downloads and extractions using Tokio workers.
* 🔒 **Hardened Security** — SHA integrity validation (`sha512`/`sha1`) and path traversal safeguards against unsafe archives.
* 📦 **Standard npm Interoperability** — First-class support for `package.json`, standard v3 `package-lock.json`, scoped registries, and workspace linking.
* 🪶 **Low Overhead** — Built natively in Rust without requiring Node.js to resolve or extract dependencies.

---

## 📥 Installation

### Building from Source

Ensure you have Rust and Cargo installed (1.75+ recommended):

```bash
git clone https://github.com/LamoDev/Bolt.git
cd Bolt
cargo build --release
```

The resulting binary will be available at `target/release/bolt` (or `target/release/bolt.exe` on Windows). You can copy or symlink it into your system's `PATH`.

```bash
# On Unix-like systems:
sudo cp target/release/bolt /usr/local/bin/

# On Windows (PowerShell running as Admin, or add target/release to PATH):
Copy-Item target/release/bolt.exe C:\Windows\System32\
```

---

## 🚀 Quick Start & Common Usage

### 1. Install Project Dependencies
```bash
# Resolve and install dependencies from package.json
bolt install

# Install with hard-link content-addressable store (CAS)
bolt install --hardlink

# Perform a dry-run check
bolt install --dry-run
```

### 2. Add New Packages
```bash
# Add to dependencies
bolt install express lodash@^4.17.21

# Add to devDependencies
bolt install -D typescript @types/node

# Add to optionalDependencies
bolt install -O fsevents
```

### 3. Clean & Deterministic CI Installs
```bash
# Install strictly adhering to package-lock.json (fails if manifest and lockfile diverge)
bolt ci

# Skip lifecycle scripts in untrusted CI environments
bolt ci --ignore-scripts
```

### 4. Remove Packages
```bash
bolt uninstall lodash
```

### 5. Run Scripts & Execute Binaries
```bash
# Run scripts declared in package.json
bolt run build
bolt run test -- --watch

# Execute local binaries from node_modules/.bin (or download on the fly)
bolt exec tsc --init
bolt exec eslint .
```

### 6. Security Audit & SBOM Generation
```bash
# Scan dependencies against npm Security Advisory database
bolt audit
bolt audit --format json --audit-level moderate

# Generate Software Bill of Materials (SBOM)
bolt sbom --format cyclonedx -o bom.json
bolt sbom --format spdx -o spdx.json
```

---

## 📋 Command Reference

| Command | Arguments / Flags | Description |
|---|---|---|
| `bolt install` | `[packages...]`, `-D`, `-O`, `--hardlink`, `--ignore-scripts`, `--dry-run` | Installs project dependencies or adds specified packages. |
| `bolt ci` | `--ignore-scripts` | Strictly installs from `package-lock.json` without modifying lockfile or manifest. |
| `bolt uninstall` | `[packages...]`, `--no-save` | Removes specified packages from `node_modules` and `package.json`. |
| `bolt update` | `[packages...]` | Updates packages to the latest matching semver versions. |
| `bolt run` | `<script> [-- <args>...]` | Executes lifecycle scripts or scripts defined in `package.json`. |
| `bolt exec` | `<command> [-- <args>...]`, `-y` | Runs binaries located in `node_modules/.bin` or fetches on the fly. |
| `bolt audit` | `--format <table\|json>`, `--audit-level <level>`, `--omit-dev` | Audits installed packages for known vulnerabilities. |
| `bolt sbom` | `--format <cyclonedx\|spdx>`, `-o <path>` | Produces a machine-readable SBOM specification file. |

### Global Flags
* `-C, --cwd <DIR>`: Set working directory.
* `-w, --workspace <PKG>`: Target specific workspace package.
* `--workspaces`: Target all workspace packages.
* `--offline`: Enforce offline mode using local cache only.
* `--registry <URL>`: Custom npm registry URL.
* `-v, --verbose`: Enable verbose debugging logs.

---

## 🔍 Compatibility Status

| Area | Support Status | Notes |
|---|---|---|
| **Package Manifest (`package.json`)** | ✅ Full | Unrecognized and custom fields are preserved during mutations. |
| **Lockfile (`package-lock.json`)** | ✅ Full | Reads v2 and v3 formats; serializes standard npm v3 format. |
| **npm Registry & Scoped Auth** | ✅ Full | Reads `.npmrc` scoped registries and `_authToken` configurations. |
| **Binary Shims (`node_modules/.bin`)** | ✅ Full | Generates `.cmd`, `.ps1` (Windows), and `/bin/sh` scripts. |
| **Monorepos & Workspaces** | ✅ Supported | Automatically discovers workspace packages and creates directory junctions / symlinks. |
| **Lifecycle Scripts** | ✅ Supported | Executes `preinstall`, `install`, `postinstall` with environment variables & extended PATH. |
| **Peer & Optional Dependencies** | ✅ Supported | Platform checks (`os`, `cpu`) and peer nesting handled. |
| **Git / Tarball URLs** | 🟡 Partial | Standard git/tarball URLs supported; complex commit/semver ranges in git URLs in active testing. |
| **npm-specific plugins / auth hooks** | ⏸️ Out of scope | Bolt uses `.npmrc` and standard token headers directly. |

---

## 🤝 Contributing

Contributions, bug reports, and feature proposals are welcome! Check out [CONTRIBUTING.md](CONTRIBUTING.md) to get started with the development and validation workflow.

## 📄 License

This project is licensed under the [MIT License](LICENSE).
