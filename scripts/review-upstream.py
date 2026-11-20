#!/usr/bin/env python3
"""Review an upstream Git range against the extraction's component map."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

DOCS = Path(__file__).resolve().parents[1] / "docs"
SUPPORTING = "Supporting changes"


def read_sections():
    sections = {}
    for line in (DOCS / "upstream-watch.paths").read_text().splitlines():
        line = line.strip()
        if line.startswith("# [") and line.endswith("]"):
            name = line[3:-1]
            sections[name] = []
        elif line and not line.startswith("#"):
            sections[name].append(line)
    return sections


def split_paths(output):
    return [os.fsdecode(field) for field in output.split(b"\0") if field]


def display_path(path):
    return path if path.isprintable() else repr(path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("repo", help="path to the local T3 Code clone")
    parser.add_argument("--base", help="previous revision (defaults to the review record)")
    parser.add_argument("--to", default="origin/main", help="candidate revision (default: origin/main)")
    parser.add_argument("--all", action="store_true", help="include supporting file paths and all other repo changes")
    args = parser.parse_args()
    state = json.loads((DOCS / "upstream-review.json").read_text())
    repo = args.repo

    def git(*arguments):
        return subprocess.check_output(["git", "-C", repo, "--no-pager", "-c", "color.ui=false", *arguments])

    repo = os.fsdecode(git("rev-parse", "--show-toplevel").rstrip(b"\n"))

    def commit(ref):
        return git("rev-parse", "--verify", "--end-of-options", ref + "^{commit}").decode().strip()

    base = commit(args.base or state["reviewedThrough"])
    tip = commit(args.to)
    ancestry = subprocess.run(["git", "-C", repo, "merge-base", "--is-ancestor", base, tip])
    if ancestry.returncode == 1:
        sys.exit("Base is not an ancestor of the candidate. Check the refs and repository history.")
    ancestry.check_returncode()
    print(f"Upstream review {base}..{tip}")
    for item in state["deferred"]:
        print(f"Deferred from an earlier review: {item['upstream']}  {item['followUp']}")
    if base == tip:
        print("The revisions are identical.")
        return

    # Match both sides independently. Git can then detect renames over the
    # entire tree even when the destination has moved outside the watched area.
    sections = read_sections()
    owners = {}
    for name, specs in sections.items():
        paths = git("diff", "--no-renames", "--name-only", "-z", base, tip, "--", *specs)
        for path in split_paths(paths):
            owners.setdefault(path, name)

    outside = "Outside the watch list"
    changes = {name: [] for name in [*sections, outside]}
    rank = {name: i for i, name in enumerate(changes)}
    fields = iter(split_paths(git("diff", "--find-renames", "--name-status", "-z", base, tip)))
    for status in fields:
        paths = [next(fields)]
        if status.startswith(("R", "C")):
            paths.append(next(fields))
        name = min((owners.get(p, outside) for p in paths), key=rank.get)
        changes[name].append(status + "  " + " -> ".join(map(display_path, paths)))

    for name, rows in changes.items():
        print(f"\n{name} ({len(rows)} files or renames)")
        if args.all or name not in (SUPPORTING, outside):
            for row in rows:
                print("  " + row)

    def component_dirs(revision):
        # Spot new top-level trees and modules even when their names are novel.
        roots = ["apps/", "apps/server/src/", "packages/", "infra/", "evals/"]
        top = git("ls-tree", "-d", "-z", "--name-only", revision)
        modules = git("ls-tree", "-d", "-z", "--name-only", revision, "--", *roots)
        return set(split_paths(top) + split_paths(modules))

    dirs = component_dirs(tip) - component_dirs(base)
    if dirs:
        print("\nNew component directories to classify")
        for path in sorted(dirs):
            print("  " + display_path(path))

    # List commits separately from endpoint differences to retain changes that
    # were introduced and subsequently reverted within the review interval.
    def history(specs):
        log = git("log", "--simplify-merges", "--no-show-signature", "--reverse", "--topo-order", "--format=%H %s",
                  base + ".." + tip, "--", *specs)
        return [line.split(" ", 1) for line in log.decode(errors="replace").splitlines()]

    shown = set()
    focused = [spec for name, specs in sections.items() if name != SUPPORTING for spec in specs]
    groups = [("Focused commits", focused), ("Additional supporting commits", sections[SUPPORTING])]
    if args.all:
        groups.append(("Other upstream commits", []))
    for heading, specs in groups:
        remaining = [(sha, subject) for sha, subject in history(specs) if sha not in shown]
        print(f"\n{heading} ({len(remaining)})")
        for sha, subject in remaining:
            print(f"  {sha[:12]}  {subject}")
            shown.add(sha)

    if not args.all:
        print("\nUse --all for every path and commit. Use git show <sha> in T3 Code for one full patch.")


if __name__ == "__main__":
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(error.returncode)
