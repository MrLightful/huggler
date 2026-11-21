// Load the exact file passed to Pi; exercise discovery and a caller-scoped read.
import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
const extension = await import(pathToFileURL(process.argv[2]));
const tools = new Map();
await extension.default({ registerTool(tool) { tools.set(tool.name, tool); } });
const result = await tools.get("thread_list").execute("call", {}, AbortSignal.timeout(5000));
assert.ok(Array.isArray(result.details.threads));
