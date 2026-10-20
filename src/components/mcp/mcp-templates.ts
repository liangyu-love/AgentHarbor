import type { McpApp, McpSpec } from "@/types";

/** The agents in the order the matrix shows them. */
export const MCP_APPS: McpApp[] = ["codex", "claudeCode", "openCode", "pi", "cursor", "harness", "claudeDesktop"];

/** Common servers to start from; the form can change anything before saving. */
export const MCP_TEMPLATES: { name: string; label: string; spec: McpSpec }[] = [
  { name: "context7", label: "Context7", spec: { transport: "stdio", command: "npx", args: ["-y", "@upstash/context7-mcp"] } },
  { name: "fetch", label: "Fetch", spec: { transport: "stdio", command: "uvx", args: ["mcp-server-fetch"] } },
  { name: "time", label: "Time", spec: { transport: "stdio", command: "uvx", args: ["mcp-server-time"] } },
  { name: "memory", label: "Memory", spec: { transport: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-memory"] } },
  {
    name: "sequential-thinking", label: "Sequential Thinking",
    spec: { transport: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-sequential-thinking"] },
  },
  {
    name: "filesystem", label: "Filesystem",
    spec: { transport: "stdio", command: "npx", args: ["-y", "@modelcontextprotocol/server-filesystem", "D:/workspace"] },
  },
  { name: "playwright", label: "Playwright", spec: { transport: "stdio", command: "npx", args: ["@playwright/mcp@latest"] } },
  {
    name: "github", label: "GitHub",
    spec: { transport: "http", url: "https://api.githubcopilot.com/mcp/", headers: { Authorization: "Bearer <GITHUB_TOKEN>" } },
  },
];

/** One line per entry, `KEY=value` (or `Name: value` for headers). */
export function linesOf(map: Record<string, string> | undefined, separator: "=" | ": ") {
  return Object.entries(map ?? {}).map(([key, value]) => `${key}${separator}${value}`).join("\n");
}

export function mapOf(text: string, separator: "=" | ":") {
  const map: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const index = line.indexOf(separator);
    if (index > 0) map[line.slice(0, index).trim()] = line.slice(index + 1).trim();
  }
  return map;
}

/** The server in the shape most agents and documentation use. */
export function mcpServersJson(entries: { name: string; spec: McpSpec }[]) {
  const servers = Object.fromEntries(entries.map(({ name, spec }) => [name, spec.transport === "stdio"
    ? { command: spec.command, args: spec.args ?? [], ...(spec.env && Object.keys(spec.env).length ? { env: spec.env } : {}) }
    : { type: spec.transport, url: spec.url, ...(spec.headers && Object.keys(spec.headers).length ? { headers: spec.headers } : {}) }]));
  return JSON.stringify({ mcpServers: servers }, null, 2);
}
