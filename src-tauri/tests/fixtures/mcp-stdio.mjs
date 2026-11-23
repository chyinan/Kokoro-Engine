import { writeFileSync } from "node:fs";
import readline from "node:readline";

if (process.env.KOKORO_MCP_TEST_PID_FILE) {
  writeFileSync(process.env.KOKORO_MCP_TEST_PID_FILE, String(process.pid));
}
const secret = process.env.KOKORO_MCP_TEST_VALUE;
process.stderr.write("x".repeat(256 * 1024));

const lines = readline.createInterface({ input: process.stdin });
const send = (body) => process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...body })}\n`);
const tool = (name) => ({ name, inputSchema: { type: "object" } });
let changed = false;
for await (const line of lines) {
  const { method, id, params } = JSON.parse(line);
  switch (method) {
    case "server/discover":
      if (process.env.KOKORO_MCP_TEST_HOLD_STARTUP) break;
      send({ id, error: { code: -32601, message: "legacy peer" } });
      break;
    case "initialize":
      send({ id, result: { protocolVersion: "2024-11-05", capabilities: { tools: { listChanged: true } }, serverInfo: { name: "stdio-fixture", version: "1" } } });
      break;
    case "tools/list":
      if (changed) send({ id, result: { tools: [tool("replacement")] } });
      else if (params?.cursor === "next") send({ id, result: { tools: [tool("second")] } });
      else send({ id, result: { tools: [tool("first")], nextCursor: "next" } });
      break;
    case "tools/call":
      send({ id, result: { content: [{ type: "text", text: secret }] } });
      if (!changed) {
        changed = true;
        send({ method: "notifications/tools/list_changed" });
      }
      break;
  }
}
