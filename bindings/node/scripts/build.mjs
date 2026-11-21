import { execFileSync } from "node:child_process"
import { copyFileSync, existsSync, renameSync } from "node:fs"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"

import platformTarget from "../platform.cjs"

const sdk = fileURLToPath(new URL("../", import.meta.url))
const release = process.argv.includes("--release")
const platform = platformTarget()
const mac = platform === "darwin-arm64"

const manifest = ["--manifest-path", "native/Cargo.toml", "--locked"]
const args = ["rustc", "--lib", ...manifest]
if (release) args.push("--release")
const env = { ...process.env }
if (mac) {
  // Apple's linker can emit an unaligned Mach-O string table rejected by dyld.
  const lib = execFileSync("rustc", ["--print", "target-libdir"], { encoding: "utf8" }).trim()
  args.push("--", "-C", "link-arg=-fuse-ld=" + resolve(lib, "../bin/gcc-ld/ld64.lld"))
  env.MACOSX_DEPLOYMENT_TARGET ??= "14.0"
  // Rust 1.94's LLD supports the Xcode 26 SDK stubs. Xcode 27 adds arm64e.x1.
  if (!env.SDKROOT) {
    const version = execFileSync("xcrun", ["--sdk", "macosx", "--show-sdk-version"], { encoding: "utf8" })
    const compatible = "/Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk"
    if (parseInt(version, 10) >= 27 && existsSync(compatible)) env.SDKROOT = compatible
  }
}
execFileSync("cargo", args, { cwd: sdk, env, stdio: "inherit" })
const metadata = execFileSync("cargo", ["metadata", "--format-version", "1", "--no-deps", ...manifest], { cwd: sdk })
const targetDir = JSON.parse(metadata).target_directory
const library = mac ? "libhuggler_node.dylib" : "libhuggler_node.so"
const binary = resolve(sdk, "npm", platform, "huggler.node")
// Replace the inode so macOS never reuses the previous binary's cached code signature.
const staging = binary + "." + process.pid
copyFileSync(resolve(targetDir, release ? "release" : "debug", library), staging)
renameSync(staging, binary)
execFileSync(process.execPath, ["-e", "require(process.argv[1])", binary], { stdio: "inherit" })
copyFileSync(resolve(sdk, "../../LICENSE"), resolve(sdk, "LICENSE"))
