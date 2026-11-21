// Keep the build, loader and offline consumer on the same package target.
function platformTarget() {
  if (process.platform === "darwin" && process.arch === "arm64") return "darwin-arm64"
  if (process.platform === "linux" && process.arch === "x64" &&
      process.report.getReport().header.glibcVersionRuntime) return "linux-x64-gnu"

  throw new Error(`Unsupported Huggler platform ${process.platform}/${process.arch}. Supported builds are macOS arm64 and Linux x64 (glibc).`)
}

module.exports = platformTarget
if (require.main === module) console.log(platformTarget())
