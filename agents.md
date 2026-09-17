# Bolt - Agents & Subsystem Architecture

This document describes the architectural domains, module boundaries, invariants, and operational guidance for AI agents and human contributors working on **Bolt**.

---

## 1. Architectural Overview

Bolt is structured as a high-throughput, modular Rust project. The codebase deliberately isolates network I/O, resolution logic, filesystem manipulation, and configuration into cohesive subsystems:

```
src/
├── bin_shim.rs    # Cross-platform CLI binary wrapper generator (.cmd, .ps1, sh)
├── cli.rs         # Command-line interface definitions and flag parsing (clap)
├── config.rs      # .npmrc parser, environment flags, and scoped registry auth
├── installer.rs   # Concurrent worker pool for tarball downloads, unpacking, and lifecycle hooks
├── lib.rs         # Public library interface exposing core engine abstractions
├── lockfile.rs    # package-lock.json v2/v3 reader, parser, and v3 serializer
├── main.rs        # CLI entry point routing commands to subsystems
├── manifest.rs    # package.json model preserving unrecognized properties
├── platform.rs    # Node/npm OS and CPU architecture constraint matching
├── registry.rs    # HTTP npm registry client with disk caching and retry logic
├── resolver.rs    # BFS greedy dependency resolution, alias support, cycles, and conflict nesting
├── scripts.rs     # Script runner and binary execution with extended PATH
├── semver.rs      # npm-compatible semver parsing, range matching, and specifier decoding
├── tarball.rs     # Security-hardened tarball extraction and SHA integrity validation
└── workspace.rs   # Monorepo discovery and junction/symlink workspace linking
```

---

## 2. Core Invariants

When modifying or extending Bolt, agents MUST maintain the following invariants:

### A. Manifest Field Preservation
- Modifying `package.json` must **never** drop or rearrange unrelated fields (e.g. `prettier`, `eslintConfig`, `browserslist`, comments or custom tooling metadata).
- Handled through `serde_json::Map` flatten fields in `src/manifest.rs`.

### B. Filesystem Security & Archive Extraction
- Tarball archives must **never** unpack entries containing directory traversal components (`..`), absolute paths (`/`, `C:\`), or unsafe symlinks.
- All archives must have their checksums validated (`sha512` or `sha1`) before unpacking onto the target filesystem.
- Safe path validation lives in `src/tarball.rs::extract_tarball_safe`.

### C. Lockfile Interoperability
- Bolt reads `package-lock.json` v2 and v3 and outputs standard v3 format.
- Package paths in `packages` must follow npm conventions:
  - Root: `""`
  - Hoisted package: `node_modules/<name>`
  - Nested package: `node_modules/<parent>/node_modules/<dep>`
- Running `bolt ci` must fail immediately if the manifest dependencies drift from the lockfile without mutating either file.

### D. Cross-Platform Compatibility
- Executable binary shims generated in `node_modules/.bin` must work across Windows PowerShell, Windows Command Prompt (`cmd.exe`), and POSIX shells (`/bin/sh`).
- Workspace directory links on Windows use directory junctions (`symlink_dir` / `mklink /j`) to avoid requiring elevated administrator privileges.

---

## 3. Subsystem Guide for Agents

### `resolver.rs` (Resolution Engine)
- **Goal:** Transform package declarations into a concrete, deterministic node graph.
- **Rules:**
  - Prefers top-level hoisting under `node_modules/<name>` unless a version conflict exists.
  - Resolves conflicts by nesting under the parent package path (`parent_path + "/node_modules/" + name`).
  - Supports npm aliases (`my-name: "npm:real-name@^1.0.0"`).
  - Evaluates platform conditions (`os`, `cpu`) and skips optional dependencies that do not target the host environment.
  - Detects cycles (`cycle_path`) to prevent infinite recursion.

### `installer.rs` (Installation & Extraction Pipeline)
- **Goal:** Materialize the resolved dependency graph onto disk.
- **Rules:**
  - Employs a Tokio semaphore (default 16 concurrent workers) to saturate network bandwidth without exhausting OS file descriptors.
  - Checks integrity hashes prior to extraction.
  - Generates binary shims in `node_modules/.bin` after extraction.
  - Executes lifecycle scripts (`preinstall`, `install`, `postinstall`) unless `--ignore-scripts` is passed.

### `registry.rs` (HTTP & Cache Subsystem)
- **Goal:** Communicate with npm-compatible registries reliably.
- **Rules:**
  - Implements exponential backoff retries (3 attempts).
  - Sanitizes scoped package names in HTTP paths (`@scope/pkg` $\to$ `@scope%2fpkg`).
  - Implements local disk caching for tarballs and metadata to allow instant warm re-installs.
  - Queries `config.rs` to attach scoped `Authorization: Bearer <token>` headers when configured.

---

## 4. Verification Workflow

Before submitting changes, every agent must execute the following validation chain:

1. **Format Check:**
   ```bash
   cargo fmt --check
   ```
2. **Typecheck & Compilation:**
   ```bash
   cargo check --all-targets
   ```
3. **Linter:**
   ```bash
   cargo clippy --all-targets -- -D warnings
   ```
4. **Integration & Unit Tests:**
   ```bash
   cargo test
   ```
5. **Release Build & Benchmarks:**
   ```bash
   cargo build --release
   node benches/benchmark.mjs
   ```
