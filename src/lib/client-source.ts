export const CLIENT_SOURCES = [
  { value: "codex", labelKey: "newapi.routeSourceCodex" },
  { value: "claudeCode", labelKey: "newapi.routeSourceClaudeCode" },
  { value: "opencode", labelKey: "newapi.routeSourceOpenCode" },
  { value: "pi", labelKey: "newapi.routeSourcePi" },
  { value: "harness", labelKey: "newapi.routeSourceHarness" },
  { value: "claudeDesktop", labelKey: "newapi.routeSourceClaudeDesktop" },
  { value: "cursor", labelKey: "newapi.routeSourceCursor" },
  { value: "imageWorkbench", labelKey: "newapi.routeSourceImageWorkbench" },
  { value: "aggregate", labelKey: "newapi.routeSourceAggregate" },
  { value: "unknown", labelKey: "newapi.routeSourceUnknown" },
] as const;

export type ClientSource = typeof CLIENT_SOURCES[number]["value"];
export type ClientSourceFilter = "all" | ClientSource;

export function normalizeClientSource(value: string | null | undefined): ClientSource {
  return CLIENT_SOURCES.find(source => source.value === value)?.value ?? "unknown";
}

export function matchesClientSource(value: string | null | undefined, filter: ClientSourceFilter): boolean {
  return filter === "all" || normalizeClientSource(value) === filter;
}

export function clientSourceLabelKey(value: string | null | undefined): string {
  return CLIENT_SOURCES.find(option => option.value === value)?.labelKey ?? "newapi.routeSourceUnknown";
}
