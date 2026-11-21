// Loaded only by Pi. Huggler's HTTP MCP endpoint uses JSON responses.
export default async function huggler(pi) {
  const url = process.env.HUGGLER_MCP_URL;
  const token = process.env.HUGGLER_MCP_TOKEN;
  const headers = {
    Authorization: `Bearer ${token}`,
    Accept: "application/json, text/event-stream",
    "Content-Type": "application/json",
  };
  let sequence = 0;
  async function request(method, params, signal, notification = false) {
    const id = notification ? undefined : ++sequence;
    const response = await fetch(url, {
      method: "POST", headers, signal, redirect: "error",
      body: JSON.stringify({ jsonrpc: "2.0", id, method, params }),
    });
    if (!response.ok) throw new Error(`Huggler MCP returned HTTP ${response.status}`);
    const session = response.headers.get("mcp-session-id");
    if (session) headers["mcp-session-id"] = session;
    if (notification) return;
    const frame = await response.json();
    if (frame.id !== id || frame.error) throw new Error(frame.error?.message ?? "Unexpected MCP reply");
    return frame.result;
  }
  const init = await request("initialize", {
    protocolVersion: "2025-06-18", capabilities: {},
    clientInfo: { name: "huggler-pi", version: "1" },
  }, AbortSignal.timeout(10000));
  headers["mcp-protocol-version"] = init.protocolVersion;
  await request("notifications/initialized", {}, AbortSignal.timeout(10000), true);
  const { tools } = await request("tools/list", {}, AbortSignal.timeout(10000));
  for (const tool of tools) {
    pi.registerTool({
      name: tool.name, label: tool.name, description: tool.description,
      parameters: tool.inputSchema,
      async execute(_id, args, signal) {
        const result = await request("tools/call", { name: tool.name, arguments: args }, signal);
        if (result.isError) throw new Error(result.content?.[0]?.text ?? "Huggler tool failed");
        return {content: result.content, details: result.structuredContent ?? {}};
      },
    });
  }
}
