import { execSync } from "child_process";
import fs from "fs";
import path from "path";
import os from "os";

const BOLT_BIN = path.resolve("target/release/bolt.exe");

// Test fixture dependencies
const FIXTURE_PKG = {
  name: "bench-fixture",
  version: "1.0.0",
  dependencies: {
    "is-number": "^7.0.0",
    "is-plain-object": "^5.0.0",
    "kind-of": "^6.0.3",
    "chalk": "^4.1.2",
    "ms": "^2.1.3"
  }
};

function runTimed(fn) {
  const start = performance.now();
  fn();
  return performance.now() - start;
}

function median(arr) {
  const s = [...arr].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 !== 0 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

function runBenchmark() {
  const runs = 5;
  console.log(`Running benchmarks (repeated runs = ${runs})...`);

  const results = {
    npm: { cold: [], warm: [], locked: [], noop: [] },
    bolt: { cold: [], warm: [], locked: [], noop: [] }
  };

  // 1. Bolt Benchmarks
  for (let i = 0; i < runs; i++) {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "bolt-bench-"));
    fs.writeFileSync(path.join(tempDir, "package.json"), JSON.stringify(FIXTURE_PKG, null, 2));

    // Clear bolt local cache for cold
    const cacheDir = path.join(process.env.LOCALAPPDATA || os.tmpdir(), "bolt", "cache");
    if (fs.existsSync(cacheDir)) {
      try { fs.rmSync(cacheDir, { recursive: true, force: true }); } catch (e) {}
    }

    // Cold
    const tCold = runTimed(() => {
      execSync(`"${BOLT_BIN}" install`, { cwd: tempDir, stdio: "ignore" });
    });
    results.bolt.cold.push(tCold);

    // No-op
    const tNoop = runTimed(() => {
      execSync(`"${BOLT_BIN}" install`, { cwd: tempDir, stdio: "ignore" });
    });
    results.bolt.noop.push(tNoop);

    // Warm (remove node_modules and re-install with cached files)
    fs.rmSync(path.join(tempDir, "node_modules"), { recursive: true, force: true });
    const tWarm = runTimed(() => {
      execSync(`"${BOLT_BIN}" install`, { cwd: tempDir, stdio: "ignore" });
    });
    results.bolt.warm.push(tWarm);

    // Clean locked install (ci)
    const tLocked = runTimed(() => {
      execSync(`"${BOLT_BIN}" ci`, { cwd: tempDir, stdio: "ignore" });
    });
    results.bolt.locked.push(tLocked);

    fs.rmSync(tempDir, { recursive: true, force: true });
  }

  // 2. npm Benchmarks
  for (let i = 0; i < runs; i++) {
    const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "npm-bench-"));
    fs.writeFileSync(path.join(tempDir, "package.json"), JSON.stringify(FIXTURE_PKG, null, 2));

    // Cold
    execSync("npm cache clean --force", { stdio: "ignore" });
    const tCold = runTimed(() => {
      execSync("npm install --prefer-online", { cwd: tempDir, stdio: "ignore" });
    });
    results.npm.cold.push(tCold);

    // No-op
    const tNoop = runTimed(() => {
      execSync("npm install", { cwd: tempDir, stdio: "ignore" });
    });
    results.npm.noop.push(tNoop);

    // Warm
    fs.rmSync(path.join(tempDir, "node_modules"), { recursive: true, force: true });
    const tWarm = runTimed(() => {
      execSync("npm install --prefer-offline", { cwd: tempDir, stdio: "ignore" });
    });
    results.npm.warm.push(tWarm);

    // Clean locked install (ci)
    const tLocked = runTimed(() => {
      execSync("npm ci", { cwd: tempDir, stdio: "ignore" });
    });
    results.npm.locked.push(tLocked);

    fs.rmSync(tempDir, { recursive: true, force: true });
  }

  console.log("\n=== BENCHMARK REPORT ===");
  console.log(`OS: ${os.type()} ${os.release()} (${os.arch()})`);
  console.log(`CPU: ${os.cpus()[0]?.model}`);
  console.log(`Runs per scenario: ${runs}\n`);

  console.log("| Scenario | Bolt (Median ms) | npm (Median ms) | Speedup |");
  console.log("|---|---|---|---|");

  const scenarios = [
    ["Cold Cache", "cold"],
    ["Warm Cache", "warm"],
    ["Clean Locked Install (ci)", "locked"],
    ["No-Op Install", "noop"]
  ];

  const dashboard = {
    timestamp: new Date().toISOString(),
    environment: {
      os: `${os.type()} ${os.release()} (${os.arch()})`,
      cpu: os.cpus()[0]?.model
    },
    scenarios: {}
  };

  for (const [name, key] of scenarios) {
    const boltMed = median(results.bolt[key]);
    const npmMed = median(results.npm[key]);
    const speedup = (npmMed / boltMed).toFixed(2) + "x";
    dashboard.scenarios[key] = {
      name,
      boltMs: Math.round(boltMed),
      npmMs: Math.round(npmMed),
      speedup
    };
    console.log(`| ${name} | ${boltMed.toFixed(0)} ms | ${npmMed.toFixed(0)} ms | ${speedup} |`);
  }

  // Save dashboard output to benches/results.json
  const resultsPath = path.resolve("benches/results.json");
  let history = [];
  if (fs.existsSync(resultsPath)) {
    try {
      history = JSON.parse(fs.readFileSync(resultsPath, "utf-8"));
      if (!Array.isArray(history)) history = [history];
    } catch (e) {
      history = [];
    }
  }
  history.push(dashboard);
  fs.writeFileSync(resultsPath, JSON.stringify(history, null, 2));
  console.log(`\nBenchmark results appended to ${resultsPath}`);
}

runBenchmark();
