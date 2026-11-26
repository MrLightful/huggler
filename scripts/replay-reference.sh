#!/bin/sh
# Reproduce Stage 1 in a new directory, leaving the supplied T3 Code checkout alone.
set -eu
if [ "$#" -ne 2 ]; then
  echo "Usage: $0 PATH_TO_T3CODE NEW_OUTPUT_DIRECTORY" >&2
  exit 2
fi

revision=338352dee2f43079482e054e8229a2ba055641d9
fixture=$(cd "$(dirname "$0")/../knowledge/reference/codex-resume" && pwd)
upstream=$(cd "$1" && pwd)
git -C "$upstream" cat-file -e "$revision^{commit}"
mkdir "$2"
output=$(cd "$2" && pwd)
mkdir "$output/test-home" "$output/capture"

git clone --quiet --no-checkout "$upstream" "$output/source"
git -C "$output/source" checkout --quiet --detach "$revision"
cd "$output/source"
suite=src/orchestration-v2/testkit
cmp "$fixture/codex_transcript.ndjson" "apps/server/$suite/fixtures/provider_thread_resume/codex_transcript.ndjson"
npm install --prefix "$output/tools" --no-audit --no-fund node@26.13.1 pnpm@12.14.0
export PATH="$output/tools/node_modules/.bin:$PATH"
pnpm --filter @t3tools/monorepo --filter 't3...' install --frozen-lockfile

cd apps/server
run_tests() {
  env -i PATH="$output/source/node_modules/.bin:$PATH" HOME="$output/test-home" \
    TMPDIR="${TMPDIR:-/tmp}" LANG=en_US.UTF-8 CI=1 HUGGLER_REFERENCE_OUTPUT="$output/capture" \
    vp test run --maxWorkers 1 --reporter=verbose "$@"
}
recovery="$suite/OrchestratorReplayRecovery.integration.test.ts"
run_tests "$suite/CodexReplayFixtures.integration.test.ts" \
  src/orchestration-v2/Orchestrator.receipt-race.test.ts \
  src/mcp/OrchestratorMcpToolkit.integration.test.ts "$recovery"

git -C "$output/source" apply "$fixture/capture.patch"
run_tests "$recovery" -t 'resumes a provider-native Codex thread after recreating the orchestrator runtime'
sqlite3 -batch -init /dev/null -readonly "$output/capture/restart.sqlite" \
  < "$fixture/trace.sql" > "$output/capture/trace.txt"
diff -u "$fixture/cursors.json" "$output/capture/cursors.json"
diff -u "$fixture/expected.txt" "$output/capture/trace.txt"
echo "Stage 1 reference matched. Captured evidence in $output/capture"
