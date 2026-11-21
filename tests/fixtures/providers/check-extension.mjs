// Load the exact file passed to Pi; exercise discovery and a caller-scoped read.
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const extension = await import(pathToFileURL(process.argv[2]));
const tools = new Map();
await extension.default({ registerTool(tool) { tools.set(tool.name, tool); } });
const custom = JSON.parse(process.env.HUGGLER_MCP_PRESENTATION ?? "{}").clientInfo?.name === "atlas";
const directory = tools.get(custom ? "directory" : "thread_list");
if (custom) {
    assert.equal(directory.label, "Find collaborators");
    assert.ok(!tools.has("thread_list"));
}
const result = await directory.execute("call", {}, AbortSignal.timeout(5000));
assert.ok(Array.isArray(result.details.threads));
