"""Collect license texts for a locked Cargo target included in a binary distribution."""
from collections import defaultdict
import json
from pathlib import Path
import re
import shutil
import subprocess
import tomllib


def write_notices(root, resources, target, features, *, manifest="desktop/Cargo.toml", component="desktop"):
    resources.mkdir()
    shutil.copyfile(root / "LICENSE", resources / "LICENSE.txt")
    rust_docs = Path(subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip())
    shutil.copyfile(rust_docs / "share/doc/rust/COPYRIGHT-library.html", resources / "Rust-library.html")
    report = json.loads(subprocess.check_output([
        "cargo", "about", "generate", "--frozen", "--fail", "--format", "json",
        "--config", root / "desktop/about.toml", "--manifest-path", root / manifest, "--target", target, *features,
    ], cwd=root))
    # cargo-about selects a license from each SPDX expression. Keep upstream
    # notices as well, including nested notices for vendored native sources.
    texts = defaultdict(set)
    for license in report["licenses"]:
        for use in license["used_by"]:
            package = use["crate"]
            if package["source"]:
                source = "" if license["source_path"] else ", SPDX standard text"
                texts[license["text"].strip()].add(f"{package['name']} {package['version']} ({license['id']}{source})")
    packages = {}
    notice_name = re.compile(r"(licen[cs]es?|copying|copyright|notices?|patents|ofl)([-.].*)?", re.I)
    for item in report["crates"]:
        package = item["package"]
        name, version = package["name"], package["version"]
        if package["source"] is None:
            continue
        if package["source"] != "registry+https://github.com/rust-lang/crates.io-index":
            raise ValueError(f"Verify source distribution for {name} {version}")
        label = f"{name} {version}"
        authors = ", ".join(package["authors"])
        packages[label] = (f"{label} — {item['license']}\n"
                           f"  https://crates.io/api/v1/crates/{name}/{version}/download\n"
                           + (f"  Manifest authors: {authors}\n" if authors else ""))
        folder = Path(package["manifest_path"]).parent
        for path in folder.rglob("*"):
            if notice_name.fullmatch(path.name) and path.suffix != ".rs" and path.is_file():
                # rav1e distributes PATENTS with Windows-1252 quotation marks.
                encoding = "cp1252" if name == "rav1e" and version == "0.8.1" and path.name == "PATENTS" else "utf-8"
                texts[path.read_text(encoding=encoding).strip()].add(f"{name} {version} / {path.relative_to(folder)}")
    supplement = tomllib.loads((root / "desktop/upstream-notices.toml").read_text())
    for notice in supplement["notice"]:
        for label, url in notice["sources"].items():
            if label in packages:
                texts[notice["text"].strip()].add(f"{label} / {url}")
    if component == "desktop":
        texts[(root / "desktop/vendor/gpui-component/LICENSE-APACHE").read_text().strip()].add(
            "gpui-component 0.5.1 (modified input rendering) / "
            "https://crates.io/api/v1/crates/gpui-component/0.5.1/download"
        )
        texts[(root / "desktop/vendor/ghostty-vt/LICENSE").read_text().strip()].add(
            "libghostty-vt 1.0.1 / https://github.com/ghostty-org/ghostty/tree/vt-1.0.1"
        )
        texts[(root / "desktop/assets/dm-sans/OFL.txt").read_text().strip()].add(
            "DM Sans / https://github.com/googlefonts/dm-fonts/tree/4412393b7d2de9fe7a92064c2dce9b5af5d7fd26/Sans"
        )
        texts[(root / "desktop/assets/provider-logos/LICENSE").read_text().strip()].add(
            "Provider marks from theSVG / https://github.com/GLINCKER/thesvg/tree/551229c188b4ab6e2d8b5763b9633ec91206d04c"
        )
        texts[(root / "desktop/assets/icons/mcp.LICENSE").read_text().strip()].add(
            "Model Context Protocol mark / "
            "https://github.com/modelcontextprotocol/docs/blob/573dc60c2e7aab2605b29d0bf27194aa7b02e4fb/favicon.svg"
        )
        for name in ("LICENSE.md", "NOTICE.md"):
            texts[(root / "desktop/assets/file-icons" / name).read_text().strip()].add(
                f"File icons adapted from @pierre/trees 1.0.0-beta.6 / {name} / "
                "https://registry.npmjs.org/@pierre/trees/-/trees-1.0.0-beta.6.tgz"
            )
    with (resources / "Third-party notices.txt").open("w", encoding="utf-8") as out:
        out.write(f"Locked {component} dependencies, including build tools\n\n"
                  "The following URLs provide the corresponding unmodified sources, including "
                  "components licensed under MPL-2.0. License texts and notices follow.\n"
                  "SQLite's bundled amalgamation is public domain (https://www.sqlite.org/copyright.html).\n\n")
        out.writelines(packages[label] for label in sorted(packages))
        for body, names in sorted(texts.items(), key=lambda pair: sorted(pair[1])):
            out.write("\n" + "\n".join(sorted(names)) + "\n\n" + body + "\n")
