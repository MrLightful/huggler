"""Build the SDK and host binary and pack both for offline installation."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory

sdk = Path(__file__).resolve().parents[1]
repo = sdk.parents[1]
sys.path.insert(0, str(repo / "scripts"))
from cargo_notices import write_notices

package_name = subprocess.check_output(["node", sdk / "platform.cjs"], text=True).strip()
triple = subprocess.check_output(["rustc", "--print", "host-tuple"], text=True).strip()
binary = sdk / "npm" / package_name
destination = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else sdk / "packages"
destination.mkdir(parents=True, exist_ok=True)
manifest = "bindings/node/native/Cargo.toml"
subprocess.run(["npm", "run", "build:release"], cwd=sdk, check=True)
subprocess.run(["cargo", "fetch", "--locked", "--manifest-path", manifest], cwd=repo, check=True)


def pack(location, required):
    packed = json.loads(subprocess.check_output(
        ["npm", "pack", "--ignore-scripts", "--json", "--pack-destination", destination], cwd=location
    ))[0]
    files = {file["path"] for file in packed["files"]}
    if not required <= files:
        raise RuntimeError(f"Missing files in {packed['filename']}: {required - files}")
    print(destination / packed["filename"])


# Pack in a fresh directory so files from a previous build cannot enter the release.
with TemporaryDirectory() as temp:
    staging = Path(temp)
    for name in ("package.json", "huggler.node"):
        shutil.copy2(binary / name, staging / name)
    write_notices(repo, staging / "notices", triple, [], manifest=manifest, component="Node SDK")
    pack(staging, {"huggler.node", "notices/LICENSE.txt", "notices/Rust-library.html", "notices/Third-party notices.txt"})
pack(sdk, {"dist/index.js", "dist/index.d.ts", "platform.cjs", "LICENSE"})
