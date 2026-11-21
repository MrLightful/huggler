import { execFileSync } from "node:child_process"
import { chmodSync, copyFileSync, cpSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

import platformTarget from "../platform.cjs"

const sdk = fileURLToPath(new URL("../", import.meta.url))
const repo = resolve(sdk, "../..")
const dir = mkdtempSync(join(tmpdir(), "huggler-consumer-"))
const target = platformTarget()
const run = (bin, args, cwd = dir, env = process.env) => execFileSync(bin, args, { cwd, env, stdio: "inherit", timeout: 90000 })
function pack(cwd) {
  const result = execFileSync("npm", ["pack", "--json", "--ignore-scripts", "--pack-destination", dir], { cwd })
  return join(dir, JSON.parse(result)[0].filename)
}

try {
  writeFileSync(join(dir, "package.json"), JSON.stringify({ name: "sdk-consumer", private: true }))
  const supplied = process.argv.slice(2)
  if (supplied.length !== 0 && supplied.length !== 2) throw new Error("Expected SDK and platform tarballs")
  const packages = supplied.length ? supplied.map(file => resolve(file)) : [sdk, join(sdk, "npm", target)].map(pack)
  run("npm", ["install", "--ignore-scripts", "--offline", "--no-audit", "--no-fund", ...packages])
  cpSync(join(sdk, "tests/consumer"), dir, { recursive: true })
  const compiler = [join(sdk, "node_modules/typescript/bin/tsc"), "--strict", "--target", "ES2022",
    "--module", "Node16", "--types", "node", "--typeRoots", join(sdk, "node_modules/@types")]
  run(process.execPath, [...compiler, "consumer.mts", "common.cts"])
  run("git", ["init", "--quiet", "project with spaces"])
  copyFileSync(join(repo, "tests/fixtures/replay-codex.py"), join(dir, "codex"))
  chmodSync(join(dir, "codex"), 0o755)
  run(process.execPath, ["common.cjs"])
  const testHome = join(dir, "home")
  mkdirSync(testHome)
  for (const phase of ["responsive", "first", "second", "requests", "diagnostics", "worker", "idle"]) {
    const env = phase === "requests" ? { ...process.env, HOME: testHome } : process.env
    run(process.execPath, ["consumer.mjs", phase, repo], dir, env)
    if (phase === "first") {
      execFileSync("cargo", ["run", "--quiet", "--locked", "--manifest-path", "native/Cargo.toml",
        "--example", "handoff", "--", dir], { cwd: sdk, stdio: "inherit" })
    }
  }

  // Seed only the disposable consumer database while its owners are closed.
  run("python3", ["-c", `import sqlite3
with sqlite3.connect("custom.sqlite") as db:
    db.execute("UPDATE event SET sequence=sequence+9007199254740992")
    db.execute("UPDATE thread SET replay_floor=1 WHERE id='conversation'")`])
  run(process.execPath, ["consumer.mjs", "precision", repo])
  run(process.execPath, [...compiler, "--noEmit", "wire-contract.mts"])
  rmSync(join(dir, "node_modules/@huggler", "sdk-" + target), { recursive: true })
  run(process.execPath, ["-e", `require('node:assert/strict').throws(() => require('@huggler/sdk'), /Missing Huggler binary package/)`])
  run(process.execPath, ["-e", `Object.defineProperty(process, 'arch', {value:'unsupported'}); require('node:assert/strict').throws(() => require('@huggler/sdk'), /Unsupported Huggler platform/)`])
  console.log("Packed consumer passed on", process.platform, process.arch, process.version)
} finally {
  rmSync(dir, { recursive: true, force: true })
}
