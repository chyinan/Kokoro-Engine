// Controlled OpenAI-compatible SSE fixture, not a real model.
import http from "node:http";
import fs from "node:fs";
import path from "node:path";
const directory = path.resolve(process.argv[2] ?? "output/playwright/chat-lifecycle");
fs.mkdirSync(directory, { recursive: true });
const calls = [];
const server = http.createServer(async (request, response) => {
  if (request.url === "/counts") { response.writeHead(200, { "content-type": "application/json" }); response.end(JSON.stringify(calls)); return; }
  let raw = ""; for await (const chunk of request) raw += chunk;
  let body; try { body = JSON.parse(raw); } catch { response.writeHead(400); response.end("invalid JSON"); return; }
  const user = [...(body.messages ?? [])].reverse().find(message => message.role === "user");
  const content = typeof user?.content === "string" ? user.content : JSON.stringify(user?.content);
  const match = content?.match(/\[case:([^\]]+)\]/);
  const name = match?.[1] ?? "system";
  const call = { index: calls.length + 1, name, streaming: body.stream === true, toolResult: body.messages?.some(message => message.role === "tool"), tools: body.tools?.map(tool => tool.function?.name) ?? [], startedAt: new Date().toISOString() };
  calls.push(call);
  const save = () => fs.writeFileSync(path.join(directory, "controlled-model-calls.json"), JSON.stringify(calls, null, 2));
  save();
  if (name === "provider-failure") { response.writeHead(400, { "content-type": "application/json" }); response.end(JSON.stringify({ error: { message: "controlled failure", type: "invalid_request_error" } })); return; }
  if (!body.stream) {
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ id: "fixture-system", object: "chat.completion", created: 1, model: "controlled-sse", choices: [{ index: 0, message: { role: "assistant", content: "{\"cues\":[]}" }, finish_reason: "stop" }], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })); return;
  }
  response.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  const chunk = delta => response.write(`data: ${JSON.stringify({ id: `fixture-${call.index}`, object: "chat.completion.chunk", created: 1, model: "controlled-sse", choices: [{ index: 0, delta, finish_reason: null }] })}\n\n`);
  let closed = false; response.on("close", () => { closed = true; call.closedAt = new Date().toISOString(); save(); });
  chunk({ role: "assistant", content: "" });
  const hasTextToolFeedback = body.messages?.some(message => message.role === "system" && message.content?.startsWith?.("[Tool results]"));
  if ((name === "text-tool" || name === "text-tool-only") && !hasTextToolFeedback) {
    chunk({ content: "[TOOL_CALL:get_time]" });
  } else if (name === "tool" && !call.toolResult) {
    chunk({ content: "Draft before get_time. " });
    const tool = call.tools.find(name => name.endsWith("get_time"));
    if (tool) chunk({ tool_calls: [{ index: 0, id: "fixture-tool-call", type: "function", function: { name: tool, arguments: "{}" } }] });
    else chunk({ content: "No get_time tool configured" });
  } else if (name !== "text-tool-only") {
    const chunks = [`Controlled `, `reply `, `for `, `[case:${name}]`, ` completed.`];
    for (const text of chunks) {
      if (closed) break;
      chunk({ content: text });
      await new Promise(resolve => setTimeout(resolve, name === "tool" || name.includes("slow") || name.includes("cancel") ? 500 : 160));
    }
  }
  if (!closed) { response.write("data: [DONE]\n\n"); response.end(); call.completed = true; save(); }
});
server.listen(19334, "127.0.0.1", () => process.stdout.write("Controlled model fixture listening on 127.0.0.1:19334\n"));
