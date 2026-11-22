#!/usr/bin/env python3
"""Drive the native PR actions with a local GitHub fixture and a scripted Codex peer."""
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading

from desktop_check import ROOT, isolated_env, launch_thread, run_window


GH = """#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys
import time

assert sys.argv[1:] == ["auth", "token", "--hostname", "github.com"]
scenario = json.loads(Path(os.environ["PR_FIXTURE"]).read_text())
if scenario == "offline":
    sys.exit(1)
if scenario == "timeout":
    time.sleep(120)
    sys.exit(1)
print("fixture-token")
"""

SSH = """#!/usr/bin/env python3
import os
from pathlib import Path
import sys

if "-G" in sys.argv:
    sys.exit(0)
assert sys.argv[-2:] == ["git@github.com", "git-receive-pack '/fixture/project.git'"]
os.execvp("git", ["git", "receive-pack", str(Path(__file__).with_name("remote.git"))])
"""


def github_response(fixture, query, variables):
    if query.startswith("query PullRequestBase"):
        assert variables == {"owner": "fixture", "repo": "project"}
        return {"data": {"repository": {"defaultBranchRef": {"name": "main"}}}}
    pr = json.loads(fixture.read_text())
    if pr == "switch_branch":
        assert variables["head"] == "review"
        subprocess.run(
            ["git", "-C", fixture.parent / "checkout", "switch", "-qc", "other-topic"],
            check=True,
        )
        fixture.write_text("null")
        pr = None
    if isinstance(pr, dict) and "next" in pr:
        fixture.write_text(json.dumps(pr["next"]))
        pr = pr["current"]
    multiple = isinstance(pr, list)
    assert pr is not None or "mutation" not in query
    if "mutation" in query:
        assert not multiple
        assert variables["id"] == pr["id"] and variables["head"] == pr["headRefOid"]
        assert "expectedHeadOid: $head" in query
        if "enablePullRequestAutoMerge" in query:
            operation = "enablePullRequestAutoMerge"
            if pr["isMergeQueueEnabled"]:
                assert "method" not in variables
            else:
                assert variables["method"] == "SQUASH"
            pr["autoMergeRequest"] = {"mergeMethod": "SQUASH"}
        elif "enqueuePullRequest" in query:
            operation = "enqueuePullRequest"
            assert "method" not in variables
            pr["mergeQueueEntry"] = {"id": "queue-entry"}
        else:
            operation = "mergePullRequest"
            assert variables["method"] == "SQUASH"
            pr["state"] = "MERGED"
        fixture.write_text(json.dumps(pr))
        receipt = "mergeQueueEntry" if operation == "enqueuePullRequest" else "pullRequest"
        return {"data": {operation: {receipt: {"id": pr["id"]}}}}
    else:
        if variables["head"] == "main":
            assert variables == {"owner": "fixture", "repo": "project", "head": "main", "ref": "refs/heads/main"}
            assert pr is None
            return {"data": {"repository": {"ref": None, "recent": {"nodes": []}}}}
        assert variables["head"] in ("review", "other-topic")
        assert variables == {
            "owner": "fixture", "repo": "project",
            "head": variables["head"], "ref": f"refs/heads/{variables['head']}",
        }
        opened = pr if multiple else [pr] if pr and pr["state"] == "OPEN" else []
        recent = [pr] if pr and not opened else []
        return {"data": {"repository": {
            "ref": {"associatedPullRequests": {"nodes": opened}, "recent": {"nodes": recent}},
            "recent": {"nodes": []}
        }}}


class GitHubHandler(BaseHTTPRequestHandler):
    def do_POST(self):
        assert self.headers["Authorization"] == "Bearer fixture-token"
        payload = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        with self.server.lock:
            if self.path == "/graphql":
                response = github_response(self.server.fixture, payload["query"], payload["variables"])
                status = 200
            else:
                assert self.path == "/repos/fixture/project/pulls"
                response, status = create_pr(self.server.fixture, payload)
        data = json.dumps(response).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        pass


def create_pr(fixture, payload):
    scenario = json.loads(fixture.with_name("create.json").read_text())
    assert json.loads(fixture.read_text()) is None
    assert payload == scenario["payload"]
    remote = fixture.with_name("remote.git")
    published = subprocess.check_output(
        ["git", "--git-dir", remote, "rev-parse", "refs/heads/review"], text=True,
    ).strip()
    if "headRefOid" in scenario["pr"]:
        assert published == scenario["pr"]["headRefOid"]
    scenario["pr"]["headRefOid"] = published
    fixture.write_text(json.dumps(scenario["pr"]))
    if scenario.get("lost_receipt"):
        return {"message": "Simulated response failure after creation"}, 503
    return {"number": scenario["pr"]["number"]}, 201


@contextmanager
def github_api(fixture):
    with ThreadingHTTPServer(("127.0.0.1", 0), GitHubHandler) as server:
        server.fixture = fixture
        server.lock = threading.Lock()
        thread = threading.Thread(target=server.serve_forever)
        thread.start()
        try:
            yield f"http://127.0.0.1:{server.server_port}"
        finally:
            server.shutdown()
            thread.join()


def check(binary):
    with (
        tempfile.TemporaryDirectory(prefix="huggler desktop PR ") as tmp,
        github_api(Path(tmp) / "pr.json") as origin,
    ):
        root = Path(tmp)
        checkout = root / "repository"
        state = root / "state"
        remote = root / "remote.git"

        def git(*args):
            return subprocess.check_output(["git", "-C", checkout, *args], text=True).strip()

        def commit(message, *body):
            git("-c", "user.name=Fixture", "-c", "user.email=fixture@example.test",
                "-c", "commit.gpgsign=false", "commit", "--allow-empty", "-qm", message, *body)
            return git("rev-parse", "HEAD")

        def remote_ref(branch):
            return subprocess.check_output(
                ["git", "--git-dir", remote, "rev-parse", f"refs/heads/{branch}"], text=True,
            ).strip()

        checkout.mkdir()
        git("init", "-q", "-b", "main")
        git("init", "-q", "--bare", "-b", "main", str(remote))
        git("config", "user.name", "Fixture")
        git("config", "user.email", "fixture@example.test")
        git("config", "commit.gpgsign", "false")
        (checkout / ".gitignore").write_text("*.log\n")
        (checkout / "unstaged.txt").write_text("Original content\n")
        (checkout / "removed.txt").write_text("Remove this\n")
        git("add", "-A")
        base = commit("Base")
        git("push", str(remote), "main")
        git("remote", "add", "origin", "ssh://git@github.com/fixture/project.git")
        git("update-ref", "refs/remotes/origin/main", base)
        git("symbolic-ref", "refs/remotes/origin/HEAD", "refs/remotes/origin/main")
        worktree = root / "checkout"
        git("worktree", "add", "-qb", "review", str(worktree))
        checkout = worktree
        env = isolated_env(root)
        env.update(PATH=f"{root}:{env['PATH']}",
                   HUGGLER_CODEX=str(ROOT / "tests/fixtures/desktop-coding-peer.py"),
                   PR_FIXTURE=str(root / "pr.json"),
                   HUGGLER_QUALIFICATION_GITHUB_API=origin)
        ssh = root / "fixture-ssh"
        ssh.write_text(SSH)
        ssh.chmod(0o755)
        env["GIT_SSH_COMMAND"] = "'" + str(ssh) + "'"
        gh = root / "gh"
        gh.write_text(GH)
        gh.chmod(0o755)
        fixture = Path(env["PR_FIXTURE"])

        pr = {
            "id": "fixture-pr", "number": 12, "url": "https://github.com/fixture/project/pull/12",
            "state": "OPEN", "isDraft": False, "baseRefName": "main",
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

        def expect_creation(pr, title, body="", *, lost_receipt=False, base="main"):
            fixture.with_name("create.json").write_text(json.dumps({
                "pr": pr,
                "payload": {"head": "review", "base": base, "title": title, "body": body},
                "lost_receipt": lost_receipt,
            }))

        hidden = ({"absent": "Checking PR…"}, {"absent": "Create PR"})
        show(None, *launch_thread(checkout, "Review", current=True), {"see": "Open in"}, *hidden)
        (checkout / "ignored.log").write_text("Ignore this.\n")
        show(None, {"see": "Open in"}, *hidden)

        # Local changes alone make Create PR available. GitHub receives the commit made by the click.
        (checkout / "staged.txt").write_text("Staged content\n")
        git("add", "staged.txt")
        (checkout / "unstaged.txt").write_text("Updated content\n")
        (checkout / "removed.txt").unlink()
        (checkout / "new.txt").write_text("New content\n")
        status = git("--no-optional-locks", "status", "--porcelain")
        show("switch_branch", {"click": "Create PR"}, {"contains": "checkout changed since Create PR"})
        assert git("branch", "--show-current") == "other-topic"
        assert git("rev-parse", "HEAD") == base
        assert git("status", "--porcelain") == status
        git("switch", "-q", "review")

        git_dir = Path(git("rev-parse", "--absolute-git-dir"))
        index = git_dir / "index"
        index_before = index.read_bytes()
        commit_message = root / "commit-message.json"
        commit_message.write_text(json.dumps("invalid"))
        show(None, {"click": "Create PR"}, {"contains": "invalid commit message"})
        assert git("rev-parse", "HEAD") == base
        assert index.read_bytes() == index_before
        assert git("status", "--porcelain") == status
        assert json.loads(fixture.read_text()) is None
        commit_message.write_text(json.dumps("edit"))
        show(None, {"click": "Create PR"}, {"contains": "changed while writing the commit message"})
        assert git("rev-parse", "HEAD") == base
        assert git("status", "--porcelain") == status
        assert (checkout / "new.txt").read_text() == "New content, edited during generation.\n"
        (checkout / "new.txt").write_text("New content\n")

        show("offline", {"click": "Create PR"}, {"contains": "include uncommitted changes"})
        assert git("rev-parse", "HEAD") == base
        assert git("status", "--porcelain") == status
        commit_message.write_text(json.dumps("success"))
        hooks = root / "hooks"
        hooks.mkdir()
        hook = hooks / "pre-commit"
        hook.write_text("#!/bin/sh\ngit add -f -- ignored.log || exit 2\nexit 1\n")
        hook.chmod(0o755)
        git("config", "core.hooksPath", str(hooks))
        index_before = index.read_bytes()
        show(None, {"click": "Create PR"}, {"contains": "Commit changes for the pull request"})
        assert git("rev-parse", "HEAD") == base
        assert index.read_bytes() == index_before
        assert not index.with_name("index.lock").exists()
        hook.unlink()
        # A single-branch checkout can lack both copies of the base ref.
        git("update-ref", "-d", "refs/remotes/origin/main")
        git("update-ref", "-d", "refs/heads/main")
        expect_creation(pr, "Update review fixtures", "Cover staged, unstaged, new, and removed files.")
        show(None, {"see": "Create PR"}, {"right_of": ["Create PR", "Open in"]},
             {"click": "Create PR"}, {"opened_url": pr["url"]}, {"see": "Merge"})
        head = git("rev-parse", "HEAD")
        pr["headRefOid"] = head
        assert head == remote_ref("review") and git("rev-parse", "HEAD^") == base
        assert git("rev-parse", "@{upstream}") == head
        assert git("status", "--porcelain") == ""
        assert git("diff", "--name-status", base, head).splitlines() == [
            "A\tnew.txt", "D\tremoved.txt", "A\tstaged.txt", "M\tunstaged.txt",
        ]
        assert git("show", "HEAD:new.txt") == "New content"
        assert git("show", "HEAD:staged.txt") == "Staged content"
        assert git("show", "HEAD:unstaged.txt") == "Updated content"
        assert (checkout / "ignored.log").read_text() == "Ignore this.\n"
        git("update-ref", "refs/remotes/origin/main", base)
        git("update-ref", "refs/heads/main", base)
        show(None, {"see": "Create PR"})

        # A tracked branch can also be ahead. Reconcile a lost create receipt.
        git("config", "branch.review.gh-merge-base", "release")
        git("push", str(remote), f"{base}:refs/heads/release")
        git("update-ref", "refs/remotes/origin/release", base)
        # Status and creation both use the topic head if its upstream is the PR base.
        git("branch", "--set-upstream-to=origin/release")
        show(pr, {"see": "Merge"})
        head = commit("Second review")
        pr = dict(pr, headRefOid=head, baseRefName="release")
        expect_creation(pr, "Second review", lost_receipt=True, base="release")
        show(None, {"click": "Create PR"}, {"opened_url": pr["url"]}, {"see": "Merge"})
        assert git("rev-parse", "@{upstream}") == head
        show("offline", {"click": "Create PR"},
             {"opened_url": "https://github.com/fixture/project/compare/release...review?expand=1"})

        git("config", "branch.review.pushRemote", "fork")
        show(None, {"click": "Create PR"}, {"contains": "different Git push remote configured"})
        assert json.loads(fixture.read_text()) is None
        git("config", "--unset", "branch.review.pushRemote")

        # A stale tracking ref must not let Create PR overwrite newer remote work.
        git("switch", "-qc", "other-work")
        newer = commit("Another developer's commit")
        git("push", str(remote), "HEAD:refs/heads/review")
        git("switch", "-q", "review")
        show(None, {"click": "Create PR"}, {"contains": "Push the selected branch"})
        assert remote_ref("review") == newer
        assert git("rev-parse", "HEAD") == head
        assert json.loads(fixture.read_text()) is None

        # Single-branch clones can have an upstream configured outside the fetch refspec.
        git("config", "remote.origin.fetch", "+refs/heads/main:refs/remotes/origin/main")
        git("update-ref", "-d", "refs/remotes/origin/review")
        show(pr, {"see": "Merge"})
        assert git("config", "branch.review.merge") == "refs/heads/review"
        git("config", "remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*")
        git("update-ref", "refs/remotes/origin/review", head)

        show({"current": None, "next": pr}, {"click": "Create PR"},
             {"contains": "The checkout or pull request status changed"})
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
             {"click": "Merge"}, {"contains": "The checkout or pull request status changed"})
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

        # Local commits on the base branch do not make it a PR head.
        checkout = root / "repository"
        commit("Local commit on main")
        (checkout / "local.txt").write_text("Local changes on main.\n")
        show(None, *launch_thread(checkout, "Base checkout", current=True), {"see": "Open in"}, *hidden)
    print("Native PR actions passed with a local GitHub fixture and a scripted Codex peer")


if __name__ == "__main__":
    check(Path(sys.argv[1]).resolve())
