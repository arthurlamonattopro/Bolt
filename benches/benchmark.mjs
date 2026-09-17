import { execSync } from "child_process";
import fs from "fs";
import path from "path";
import os from "os";

const isWindows = process.platform === "win32";
const BOLT_BIN = path.resolve(isWindows ? "target/release/bolt.exe" : "target/release/bolt");

// Fixtures of different sizes
const FIXTURES = {
  small: {
    name: "bench-small",
    version: "1.0.0",
    dependencies: {
      "is-number": "^7.0.0",
      "is-plain-object": "^5.0.0",
      "kind-of": "^6.0.3",
      "ms": "^2.1.3"
    }
  },
  medium: {
    name: "bench-medium",
    version: "1.0.0",
    dependencies: {
      "chalk": "^4.1.2",
      "glob": "^8.1.0",
      "mkdirp": "^1.0.4",
      "minimist": "^1.2.8",
      "semver": "^7.5.4",
      "debug": "^4.3.4",
      "uuid": "^9.0.1",
      "dotenv": "^16.3.1",
      "commander": "^11.1.0",
      "yargs": "^17.7.2"
    }
  }
};

function hasTool(cmd) {
  try {
    execSync(isWindows ? `where ${cmd}` : `which ${cmd}`, { stdio: "ignore" });
    return true;
  } catch (e) {
    return false;
  }
}

function runTimed(fn) {
  const start = performance.now();
  fn();
  return performance.now() - start;
}

function median(arr) {
  if (!arr || arr.length === 0) return 0;
  const s = [...arr].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 !== 0 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}

function runBenchmark() {
  const runs = 3;
  const tools = ["bolt", "npm"];
  if (hasTool("pnpm")) tools.push("pnpm");
  if (hasTool("bun")) tools.push("bun");

  console.log(`\n=== BOLT BENCHMARK SUITE ===`);
  console.log(`OS: ${os.type()} ${os.release()} (${os.arch()})`);
  console.log(`CPU: ${os.cpus()[0]?.model}`);
  console.log(`Available Package Managers: ${tools.join(", ")}`);
  console.log(`Runs per scenario: ${runs}\n`);

  const dashboard = {
    timestamp: new Date().toISOString(),
    environment: {
      os: `${os.type()} ${os.release()} (${os.arch()})`,
      cpu: os.cpus()[0]?.model,
      availableTools: tools
    },
    fixtures: {}
  };

  for (const [fixtureSize, fixturePkg] of Object.entries(FIXTURES)) {
    console.log(`\n-----------------------------------------`);
    console.log(`Testing Fixture: [${fixtureSize.toUpperCase()}] (~${Object.keys(fixturePkg.dependencies).length} direct deps)`);
    console.log(`-----------------------------------------`);

    const fixtureResults = {};
    for (const tool of tools) {
      fixtureResults[tool] = { cold: [], warm: [], locked: [], noop: [] };
    }

    // 1. Bolt Benchmarks
    for (let i = 0; i < runs; i++) {
      const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "bolt-bench-"));
      fs.writeFileSync(path.join(tempDir, "package.json"), JSON.stringify(fixturePkg, null, 2));

      // Clear bolt local cache for cold
      const cacheDir = path.join(process.env.LOCALAPPDATA || os.tmpdir(), "bolt", "cache");
      if (fs.existsSync(cacheDir)) {
        try { fs.rmSync(cacheDir, { recursive: true, force: true }); } catch (e) {}
      }

      // Cold
      const tCold = runTimed(() => {
        execSync(`"${BOLT_BIN}" install`, { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.bolt.cold.push(tCold);

      // No-op
      const tNoop = runTimed(() => {
        execSync(`"${BOLT_BIN}" install`, { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.bolt.noop.push(tNoop);

      // Warm
      fs.rmSync(path.join(tempDir, "node_modules"), { recursive: true, force: true });
      const tWarm = runTimed(() => {
        execSync(`"${BOLT_BIN}" install`, { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.bolt.warm.push(tWarm);

      // Clean locked install (ci)
      const tLocked = runTimed(() => {
        execSync(`"${BOLT_BIN}" ci`, { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.bolt.locked.push(tLocked);

      fs.rmSync(tempDir, { recursive: true, force: true });
    }

    // 2. npm Benchmarks
    for (let i = 0; i < runs; i++) {
      const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "npm-bench-"));
      fs.writeFileSync(path.join(tempDir, "package.json"), JSON.stringify(fixturePkg, null, 2));

      // Cold
      execSync("npm cache clean --force", { stdio: "ignore" });
      const tCold = runTimed(() => {
        execSync("npm install --prefer-online", { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.npm.cold.push(tCold);

      // No-op
      const tNoop = runTimed(() => {
        execSync("npm install", { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.npm.noop.push(tNoop);

      // Warm
      fs.rmSync(path.join(tempDir, "node_modules"), { recursive: true, force: true });
      const tWarm = runTimed(() => {
        execSync("npm install --prefer-offline", { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.npm.warm.push(tWarm);

      // Clean locked install (ci)
      const tLocked = runTimed(() => {
        execSync("npm ci", { cwd: tempDir, stdio: "ignore" });
      });
      fixtureResults.npm.locked.push(tLocked);

      fs.rmSync(tempDir, { recursive: true, force: true });
    }

    // 3. pnpm Benchmarks if installed
    if (tools.includes("pnpm")) {
      for (let i = 0; i < runs; i++) {
        const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "pnpm-bench-"));
        fs.writeFileSync(path.join(tempDir, "package.json"), JSON.stringify(fixturePkg, null, 2));

        const tCold = runTimed(() => {
          execSync("pnpm install --force", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.pnpm.cold.push(tCold);

        const tNoop = runTimed(() => {
          execSync("pnpm install", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.pnpm.noop.push(tNoop);

        fs.rmSync(path.join(tempDir, "node_modules"), { recursive: true, force: true });
        const tWarm = runTimed(() => {
          execSync("pnpm install", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.pnpm.warm.push(tWarm);

        const tLocked = runTimed(() => {
          execSync("pnpm install --frozen-lockfile", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.pnpm.locked.push(tLocked);

        fs.rmSync(tempDir, { recursive: true, force: true });
      }
    }

    // 4. bun Benchmarks if installed
    if (tools.includes("bun")) {
      for (let i = 0; i < runs; i++) {
        const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "bun-bench-"));
        fs.writeFileSync(path.join(tempDir, "package.json"), JSON.stringify(fixturePkg, null, 2));

        const tCold = runTimed(() => {
          execSync("bun install --force", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.bun.cold.push(tCold);

        const tNoop = runTimed(() => {
          execSync("bun install", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.bun.noop.push(tNoop);

        fs.rmSync(path.join(tempDir, "node_modules"), { recursive: true, force: true });
        const tWarm = runTimed(() => {
          execSync("bun install", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.bun.warm.push(tWarm);

        const tLocked = runTimed(() => {
          execSync("bun install --frozen-lockfile", { cwd: tempDir, stdio: "ignore" });
        });
        fixtureResults.bun.locked.push(tLocked);

        fs.rmSync(tempDir, { recursive: true, force: true });
      }
    }

    // Print summary table
    const scenarios = [
      ["Cold Cache", "cold"],
      ["Warm Cache", "warm"],
      ["Clean Locked (ci)", "locked"],
      ["No-Op Install", "noop"]
    ];

    console.log(`| Scenario | Bolt (Median) | npm (Median) | Speedup vs npm |`);
    console.log(`|---|---|---|---|`);

    dashboard.fixtures[fixtureSize] = {};
    for (const [name, key] of scenarios) {
      const boltMed = median(fixtureResults.bolt[key]);
      const npmMed = median(fixtureResults.npm[key]);
      const speedup = npmMed > 0 ? (npmMed / boltMed).toFixed(2) + "x" : "N/A";

      dashboard.fixtures[fixtureSize][key] = {
        name,
        boltMs: Math.round(boltMed),
        npmMs: Math.round(npmMed),
        speedupVsNpm: speedup
      };

      console.log(`| ${name} | ${boltMed.toFixed(0)} ms | ${npmMed.toFixed(0)} ms | ${speedup} |`);
    }
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
