"""Build a release wheel with the locked Rust dependency notices."""

import argparse
import os
from pathlib import Path
import shutil
import subprocess
import sys
from tempfile import TemporaryDirectory

BINDING = Path(__file__).resolve().parents[1]
ROOT = BINDING.parents[1]
MANIFEST = BINDING.relative_to(ROOT) / "Cargo.toml"
sys.path.insert(0, str(ROOT / "scripts"))
from cargo_notices import write_notices


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, default=BINDING / "dist")
    parser.add_argument("--compatibility", help="Maturin platform compatibility tag")
    options = parser.parse_args()

    target = subprocess.check_output(["rustc", "--print", "host-tuple"], text=True).strip()
    environment = os.environ.copy()
    rustc_args = []
    if target == "aarch64-apple-darwin":
        # Use the same Mach-O linker and SDK as the Node distribution.
        target_lib = Path(subprocess.check_output(
            ["rustc", "--print", "target-libdir"], text=True
        ).strip())
        rustc_args = ["--", "-C", f"link-arg=-fuse-ld={target_lib.parent / 'bin/gcc-ld/ld64.lld'}"]
        environment.setdefault("MACOSX_DEPLOYMENT_TARGET", "14.0")
        compatible_sdk = Path("/Library/Developer/CommandLineTools/SDKs/MacOSX26.sdk")
        if compatible_sdk.exists():
            environment.setdefault("SDKROOT", str(compatible_sdk))

    subprocess.run(["cargo", "fetch", "--locked", "--manifest-path", MANIFEST], cwd=ROOT, check=True)
    with TemporaryDirectory(prefix=".python-wheel-", dir=BINDING) as temporary:
        stage = Path(temporary)
        for file in ("Cargo.lock", "pyproject.toml", "LICENSE"):
            shutil.copy2(BINDING / file, stage / file)
        source_manifest = (BINDING / "Cargo.toml").read_text()
        (stage / "Cargo.toml").write_text(source_manifest.replace(
            'path = "../.."', 'path = "../../.."'
        ))
        for directory in ("src", "python"):
            shutil.copytree(BINDING / directory, stage / directory, ignore=shutil.ignore_patterns("__pycache__"))
        write_notices(ROOT, stage / "python/huggler/notices", target,
                      ["--features", "extension-module"], manifest=MANIFEST, component="Python SDK")
        environment.setdefault("CARGO_TARGET_DIR", str(BINDING / "target"))
        command = [sys.executable, "-m", "maturin", "build", "--locked", "--release",
                   "--interpreter", sys.executable, "--out", str(options.out.resolve())]
        if options.compatibility:
            command += ["--compatibility", options.compatibility]
        subprocess.run(command + rustc_args, cwd=stage, env=environment, check=True)


if __name__ == "__main__":
    main()
