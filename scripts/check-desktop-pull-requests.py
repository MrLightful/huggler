#!/usr/bin/env python3
"""Drive the native PR actions with scripted GitHub CLI and Codex peers."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile

from desktop_check import ROOT, isolated_env, launch_thread, run_window


GH = """#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
import time

args = sys.argv[1:]
assert args[:4] == ["api", "graphql", "--hostname", "github.com"]
fields = dict(arg.partition("=")[::2] for arg in args[5::2])
query = fields.pop("query")
fixture = Path(os.environ["PR_FIXTURE"])
pr = json.loads(fixture.read_text())
if pr == "timeout":
    time.sleep(120)
    sys.exit(1)
if isinstance(pr, dict) and "next" in pr:
    fixture.write_text(json.dumps(pr["next"]))
    pr = pr["current"]
multiple = isinstance(pr, list)
assert pr is not None or "mutation" not in query
if "mutation" in query:
    assert not multiple
    assert fields["id"] == pr["id"] and fields["head"] == pr["headRefOid"]
    assert "expectedHeadOid: $head" in query
    if "enablePullRequestAutoMerge" in query:
        operation = "enablePullRequestAutoMerge"
        if pr["isMergeQueueEnabled"]:
            assert "method" not in fields
        else:
            assert fields["method"] == "SQUASH"
        pr["autoMergeRequest"] = {"mergeMethod": "SQUASH"}
    elif "enqueuePullRequest" in query:
        operation = "enqueuePullRequest"
        assert "method" not in fields
        pr["mergeQueueEntry"] = {"id": "queue-entry"}
    else:
        operation = "mergePullRequest"
        assert fields["method"] == "SQUASH"
        pr["state"] = "MERGED"
    fixture.write_text(json.dumps(pr))
    receipt = "mergeQueueEntry" if operation == "enqueuePullRequest" else "pullRequest"
    print(json.dumps({"data": {operation: {receipt: {"id": pr["id"]}}}}))
else:
    assert fields == {"owner": "fixture", "repo": "project", "head": "review", "ref": "refs/heads/review"}
    opened = pr if multiple else [pr] if pr and pr["state"] == "OPEN" else []
    recent = [pr] if pr and not opened else []
    print(json.dumps({"data": {"repository": {
        "ref": {"associatedPullRequests": {"nodes": opened}, "recent": {"nodes": recent}},
        "recent": {"nodes": []}
    }}}))
"""


def check(binary):
    with tempfile.TemporaryDirectory(prefix="huggler desktop PR ") as tmp:
        root = Path(tmp)
        checkout = root / "checkout"
        state = root / "state"

        def git(*args):
            return subprocess.check_output(["git", "-C", checkout, *args], text=True).strip()

        checkout.mkdir()
        git("init", "-q", "-b", "review")
        git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
            "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", "Review head")
        head = git("rev-parse", "HEAD")
        git("remote", "add", "origin", "https://github.com/fixture/project.git")
        git("update-ref", "refs/remotes/origin/review", head)
        git("branch", "--set-upstream-to=origin/review", "review")
        env = isolated_env(root)
        env.update(PATH=f"{root}:{env['PATH']}",
                   HUGGLER_CODEX=str(ROOT / "tests/fixtures/desktop-coding-peer.py"),
                   PR_FIXTURE=str(root / "pr.json"))
        gh = root / "gh"
        gh.write_text(GH)
        gh.chmod(0o755)
        fixture = Path(env["PR_FIXTURE"])

        pr = {
            "id": "fixture-pr", "number": 12, "url": "https://github.com/fixture/project/pull/12",
            "state": "OPEN", "isDraft": False, "baseRefName": "main", "headRefOid": head,
            "mergeable": "MERGEABLE", "mergeStateStatus": "CLEAN", "canBeRebased": True, "reviewDecision": None,
            "viewerCanEnableAutoMerge": False, "autoMergeRequest": None, "isMergeQueueEnabled": False,
            "mergeQueueEntry": None, "isCrossRepository": False,
            "headRepository": {"nameWithOwner": "fixture/project"},
            "repository": {"nameWithOwner": "fixture/project", "viewerPermission": "WRITE",
                           "mergeCommitAllowed": False, "squashMergeAllowed": True, "rebaseMergeAllowed": True},
            "commits": {"nodes": [{"commit": {"statusCheckRollup": None}}]},
        }

        def show(data, *steps):
            fixture.write_text(json.dumps(data))
            run_window(binary, state, steps, env)

        show(None, *launch_thread(checkout, "Review", current=True), {"see": "Create PR"})
        show({"current": None, "next": pr}, {"click": "Create PR"},
             {"contains": "error/The checkout or pull request status changed"})
        show("timeout", {"see": "Checking PR…"}, {"see": "Create PR"})
        show(dict(pr, isDraft=True), {"see": "View draft"})
        show(dict(pr, mergeStateStatus="BLOCKED"), {"see": "View PR"})
        rebase_only = dict(pr["repository"], squashMergeAllowed=False)
        show(dict(pr, repository=rebase_only, canBeRebased=False), {"see": "View PR"})
        show(dict(pr, repository=rebase_only, canBeRebased=False, isMergeQueueEnabled=True,
                  mergeStateStatus="BLOCKED", viewerCanEnableAutoMerge=True),
             {"see": "Auto-merge"}, {"next_prompt_answer": 1}, {"click": "Auto-merge"},
             {"see": "Auto-merge on"})
        show(dict(pr, mergeStateStatus="BLOCKED", viewerCanEnableAutoMerge=True),
             {"see": "Auto-merge"}, {"next_prompt_answer": 1}, {"click": "Auto-merge"},
             {"see": "Auto-merge on"})
        show(dict(pr, isMergeQueueEnabled=True), {"resize": [640, 440]}, {"see": "Queue merge"},
             {"next_prompt_answer": 1}, {"click": "Queue merge"},
             {"right_of": ["Merge queued", "Open in"]}, {"right_of": ["Show changes", "Merge queued"]})
        show(pr, {"see": "Merge"}, {"next_prompt_answer": 0}, {"click": "Merge"}, {"see": "Merge"})
        assert json.loads(fixture.read_text())["state"] == "OPEN"
        changed_head = dict(pr, headRefOid="0" * 40)
        show({"current": pr, "next": changed_head}, {"see": "Merge"}, {"next_prompt_answer": 1},
             {"click": "Merge"}, {"contains": "error/The checkout or pull request status changed"})
        assert json.loads(fixture.read_text()) == changed_head
        show(pr, {"see": "Merge"}, {"next_prompt_answer": 1}, {"click": "Merge"}, {"see": "Merged"})
        show(dict(pr, state="CLOSED"), {"see": "PR closed"})
        show(dict(pr, mergeable="CONFLICTING"), {"see": "Resolve"}, {"click": "Resolve"},
             {"contains": "You/Resolve the merge conflicts for fixture/project #12"},
             {"see": "Codex/Received the pull request task."})
        failed = {"state": "PENDING", "contexts": {"nodes": [{"conclusion": "TIMED_OUT"}]}}
        commits = {"nodes": [{"commit": {"statusCheckRollup": failed}}]}
        show(dict(pr, commits=commits, mergeStateStatus="BLOCKED"),
             {"click": "Message"}, {"type": "Review the staged result."},
             {"see": "Fix"}, {"click": "Fix"},
             {"contains": "You/Fix the failed checks or address requested review changes"},
             {"click": "Send"}, {"see": "Codex/Reviewed the staged result."})
        show(dict(pr, reviewDecision="CHANGES_REQUESTED"), {"see": "Fix"})
        show(dict(pr, headRefOid="0" * 40), {"see": "View PR"})
        show([pr, dict(pr, id="another-pr", number=13, baseRefName="release")], {"see": "View PR"})
        blob = git("hash-object", "-w", "--stdin")
        subprocess.run(["git", "-C", checkout, "update-index", "--index-info"],
                       input=f"100644 {blob} 1\tconflict.txt\n100644 {blob} 2\tconflict.txt\n",
                       text=True, check=True)
        show(None, {"see": "Resolve"}, {"click": "Resolve"},
             {"contains": "You/Resolve the Git merge conflicts in this checkout."})
    print("Native PR actions passed with scripted GitHub CLI and Codex peers")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve())
