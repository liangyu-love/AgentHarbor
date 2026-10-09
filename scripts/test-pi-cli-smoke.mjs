// Uses the actual Rust adapter output and an explicitly supplied local Pi CLI.
// All credentials are synthetic and every inference endpoint is loopback.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const workspace = process.cwd();
const cli = process.env.PI_SMOKE_CLI;
assert.ok(cli && path.isAbsolute(cli), "Set PI_SMOKE_CLI to an installed Pi dist/cli.js");
const root = path.join(workspace, ".runtime-test/pi-integration/cli");
await mkdir(root, { recursive: true });
const inputFile = path.join(root, "read-fixture.txt");
await writeFile(inputFile, "pi local tool fixture", "utf8");
const requests = [];
const upstreamKey = "!synthetic-$NO_SUCH_PI_SECRET-${NO_SUCH_PI_SECRET}";
const answer = "pi mock completed";
const server = http.createServer(async (request, response) => {
  try {
    let raw = "";
    for await (const chunk of request) raw += chunk;
    const body = JSON.parse(raw);
    requests.push({ path: request.url, model: body.model, body });
    assert.ok(request.url.startsWith("/gateway/") || request.url.startsWith("/clients/pi/"));
    const native = request.url.startsWith("/gateway/");
    const expected = native ? upstreamKey : "local-secret";
    const messages = request.url.endsWith("/messages");
    if (native && messages) assert.equal(request.headers["x-api-key"], expected);
    else assert.equal(request.headers.authorization, `Bearer ${expected}`);
    assert.equal(body.stream, true);
    response.writeHead(200, { "content-type": "text/event-stream" });
    const data = (value, event) => response.write(`${event ? `event: ${event}\n` : ""}data: ${JSON.stringify(value)}\n\n`);
    if (request.url.endsWith("/chat/completions")) {
      assert.ok(!body.messages.some(message => message.role === "developer"));
      assert.equal(body.max_tokens, 8192);
      assert.ok(!("reasoning_effort" in body), "unknown effort support must not generate an effort parameter");
      const chunk = (delta, finish_reason = null) => data({ id: "chat-pi", object: "chat.completion.chunk", created: 1,
        model: body.model, choices: [{ index: 0, delta, finish_reason }], ...(finish_reason ? { usage: { prompt_tokens: 10, completion_tokens: 3, total_tokens: 13 } } : {}) });
      if (native && !body.messages.some(message => message.role === "tool")) {
        assert.ok(body.tools.some(tool => tool.function.name === "read"));
        chunk({ role: "assistant", tool_calls: [{ index: 0, id: "call-read", type: "function", function: { name: "read", arguments: JSON.stringify({ path: inputFile }) } }] });
        chunk({}, "tool_calls");
      } else {
        if (native) assert.ok(body.messages.some(message => message.role === "tool" && JSON.stringify(message.content).includes("pi local tool fixture")));
        chunk({ role: "assistant", content: answer }); chunk({}, "stop");
      }
      response.write("data: [DONE]\n\n");
    } else if (messages) {
      const event = value => data(value, value.type);
      event({ type: "message_start", message: { id: "msg-pi", type: "message", role: "assistant", model: body.model,
        content: [], stop_reason: null, stop_sequence: null, usage: { input_tokens: 10, output_tokens: 0 } } });
      event({ type: "content_block_start", index: 0, content_block: { type: "text", text: "" } });
      event({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: answer } });
      event({ type: "content_block_stop", index: 0 });
      event({ type: "message_delta", delta: { stop_reason: "end_turn", stop_sequence: null }, usage: { output_tokens: 3 } });
      event({ type: "message_stop" });
    } else {
      assert.ok(request.url.endsWith("/responses"));
      const item = { type: "message", id: "msg-pi", role: "assistant", status: "completed", content: [{ type: "output_text", text: answer, annotations: [] }] };
      const result = { id: "resp-pi", object: "response", model: body.model, status: "completed", output: [item], usage: { input_tokens: 10, output_tokens: 3, total_tokens: 13 } };
      let sequence = 0;
      const event = value => data({ ...value, sequence_number: sequence++ }, value.type);
      event({ type: "response.created", response: { ...result, status: "in_progress", output: [], usage: null } });
      event({ type: "response.output_item.added", output_index: 0, item: { ...item, status: "in_progress", content: [] } });
      event({ type: "response.content_part.added", item_id: item.id, output_index: 0, content_index: 0, part: { type: "output_text", text: "", annotations: [] } });
      event({ type: "response.output_text.delta", item_id: item.id, output_index: 0, content_index: 0, delta: answer });
      event({ type: "response.output_text.done", item_id: item.id, output_index: 0, content_index: 0, text: answer });
      event({ type: "response.output_item.done", output_index: 0, item });
      event({ type: "response.completed", response: result });
    }
    response.end();
  } catch (error) {
    response.destroy(error);
    process.exitCode = 1;
    console.error(error);
  }
});
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));

const run = (command, args, options = {}) => new Promise((resolve, reject) => {
  const child = spawn(command, args, { cwd: workspace, ...options });
  child.stdin.end();
  let output = "";
  child.stdout.on("data", chunk => { output += chunk; });
  child.stderr.on("data", chunk => { output += chunk; });
  const deadline = setTimeout(() => { child.kill(); reject(new Error(`Timed out: ${args.join(" ")}\n${output}`)); }, 60000);
  child.on("error", error => { clearTimeout(deadline); reject(error); });
  child.on("close", code => { clearTimeout(deadline); code === 0 ? resolve(output) : reject(new Error(`Exit ${code}: ${output}`)); });
});
try {
  const exportLog = await run(process.execPath, ["scripts/tauri.mjs", "--test-rust", "pi_export_cli_smoke_fixtures", "--", "--ignored", "--test-threads=1"], {
    env: { ...process.env, AGENT_HARBOR_PI_SMOKE_ROOT: root, AGENT_HARBOR_PI_SMOKE_PORT: String(server.address().port) },
  });
  await writeFile(path.join(root, "fixture-export.log"), exportLog);
  for (const mode of ["native", "router", "aggregate"]) {
    const agentDir = path.join(root, mode);
    // Explicit CLI selections leave auth/settings files unnecessary. Extensions
    // and project discovery cannot reach the user's Pi configuration here.
    const env = { ...process.env, PI_CODING_AGENT_DIR: agentDir, HTTP_PROXY: "", HTTPS_PROXY: "", ALL_PROXY: "", NO_PROXY: "127.0.0.1,localhost" };
    const models = JSON.parse(await readFile(path.join(agentDir, "models.json"), "utf8"));
    const isolatedArgs = ["--no-extensions", "--no-skills", "--no-prompt-templates", "--no-session"];
    const list = await run(process.execPath, [cli, ...isolatedArgs, "--list-models", "agentharbor"], { cwd: agentDir, env });
    await writeFile(path.join(agentDir, "catalog.log"), list);
    assert.match(list, /agentharbor-/);
    for (const [provider, definition] of Object.entries(models.providers)) {
      const model = definition.models[0].id;
      const output = await run(process.execPath, [cli, ...isolatedArgs, "--provider", provider, "--model", model,
        "--thinking", "off", "-p", "Run the local mock smoke test."], { cwd: agentDir, env });
      await writeFile(path.join(agentDir, `${definition.api}.log`), output);
      assert.ok(output.includes(answer), `${mode}/${definition.api} produces the actual streamed final text`);
      console.log(`PASS installed Pi CLI ${mode}/${definition.api}: catalog, URL, literal auth, SSE and model ID`);
    }
  }
  assert.equal(requests.length, 8, "three native (one tool continuation), three router and one aggregate request");
  assert.ok(requests.some(request => request.model === "public-pi-model" && request.path === "/clients/pi/aggregate/v1/responses"));
  assert.ok(requests.every(request => !request.model.includes("::")));
  await writeFile(path.join(root, "requests.json"), JSON.stringify(requests.map(({ path, model }) => ({ path, model })), null, 2));
  console.log("PASS local read tool continuation and aggregate public identity; no real provider traffic");
} finally { await new Promise(resolve => server.close(resolve)); }
