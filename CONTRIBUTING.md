# Contributing to Bolt

Thank you for your interest in contributing to **Bolt**! Bolt is built in Rust to provide a high-performance, secure, and fully npm-compatible package manager.

This guide outlines our development workflow, coding invariants, and testing standards.

---

## 🛠️ Development Setup

### Prerequisites
- [Rust & Cargo](https://rustup.rs/) (version 1.75 or later recommended)
- [Node.js](https://nodejs.org/) (v18+ recommended, needed for ecosystem tests and benchmarks)
- Git

### Initializing the Workspace

Clone the repository and run the test suite to ensure everything builds and passes:

```bash
git clone https://github.com/LamoDev/Bolt.git
cd Bolt
cargo test
```

---

## 📐 Architecture & Subsystems

Before modifying the codebase, refer to [`AGENTS.md`](AGENTS.md) for detailed architecture documentation. The core subsystems are:

* `src/resolver/`: BFS greedy resolution, catalog resolution, dependency hoisting, conflict nesting, and cycle detection.
* `src/engine/installer.rs`: Concurrent worker pool for downloading, checksum validation, and extracting packages.
* `src/network/`: HTTP registry client with exponential retries, tarball caching, and npm advisory audits.
* `src/package/`: `package.json`, `package-lock.json`, and semver calculation modules.
* `src/workspace/`: Monorepo discovery and junction/symlink linking.

---

## 🔒 Invariants & Security Rules

All contributions must respect Bolt's core invariants:

1. **Manifest Field Preservation:** Never drop or alter unknown fields in `package.json` (such as `prettier`, `browserslist`, custom tooling configurations).
2. **Safe Archive Extraction:** Never extract tarballs containing path traversal sequences (`..`), absolute paths, or unsafe symlinks. Checksum validation (`sha512` or `sha1`) is mandatory before unpacking.
3. **Lockfile Interoperability:** Lockfiles must strictly follow standard npm v3 specification to ensure zero friction when switching between `bolt` and `npm`.
4. **Cross-Platform Compatibility:** Features must work cleanly across Linux, macOS, and Windows. Windows junction links must be used instead of symlinks that require Administrator privileges.

---

## 🧪 Verification & Quality Workflow

Before opening a pull request, make sure all the following checks pass locally:

### 1. Code Formatting
```bash
cargo fmt --check
```
To auto-format code:
```bash
cargo fmt
```

### 2. Linting & Static Analysis
```bash
cargo clippy --all-targets -- -D warnings
```

### 3. Compilation & Typechecks
```bash
cargo check --all-targets
```

### 4. Running Tests
Run the entire test suite (unit and integration tests):
```bash
cargo test
```

### 5. Running Benchmarks
To test performance regressions on release binaries:
```bash
cargo build --release
node benches/benchmark.mjs
```

---

## 🚢 Submitting Pull Requests

1. Create a descriptive feature branch:
   ```bash
   git checkout -b fix/resolver-peer-nesting
   ```
2. Commit your changes with clear, succinct messages.
3. Push your branch and open a Pull Request against `main`.
4. Ensure the GitHub Actions CI pipeline passes across Ubuntu, macOS, and Windows.
