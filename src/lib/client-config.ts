import { providerModelProtocol, supportsProtocol } from "./provider-models";
import type { ClaudeModels, ClaudeOneM, ClientAccessMode, ClientStatus } from "@/types/clients";
import type { ModelProtocol, NewApiProvider } from "@/types/newapi";

export type Exclusion = "protocol" | "disabled" | "noApiKey" | "noModel";
export const claudeRoles: (keyof ClaudeModels)[] = ["sonnet", "opus", "fable", "haiku", "subagent"];
export const emptyClaudeRoles = (): ClaudeModels => ({ sonnet: "", opus: "", fable: "", haiku: "", subagent: "" });
// Missing fields adopt the safe default (unmarked); explicit values survive saving.
export const claudeOneMDefaults = (saved?: Partial<ClaudeOneM> | null): ClaudeOneM => ({
  startup: saved?.startup ?? false, sonnet: saved?.sonnet ?? false,
  opus: saved?.opus ?? false, fable: saved?.fable ?? false,
  haiku: saved?.haiku ?? false, subagent: saved?.subagent ?? false,
});

// Mirrors the backend's deterministic auto-match: segment-boundary role
// matches only, the startup model wins when it fits the role, then the
// numerically newest version, then the provider's storage order.
const roleSegments = (id: string) => id.toLowerCase().split(/[-_.]/);
const dateLike = (id: string) => id.split(/[-_.]/).some(segment => segment.length >= 8 && /^\d+$/.test(segment));
const newerVersion = (a: string, b: string): number => {
  const datedA = dateLike(a), datedB = dateLike(b);
  if (datedA !== datedB) return datedA ? 1 : -1; // rolling aliases beat dated snapshots
  const segmentsA = roleSegments(a), segmentsB = roleSegments(b);
  for (let i = 0; i < Math.min(segmentsA.length, segmentsB.length); i++) {
    if (segmentsA[i] === segmentsB[i]) continue;
    const numeric = (segment: string) => (/^\d+$/.test(segment) && segment.length < 8 ? Number(segment) : null);
    const first = numeric(segmentsA[i]), second = numeric(segmentsB[i]);
    if (first !== null && second !== null) return second - first;
    return segmentsA[i] < segmentsB[i] ? 1 : -1;
  }
  return segmentsB.length - segmentsA.length; // claude-opus-4-6 > claude-opus-4
};
export const autoRoleModel = (models: string[], role: string, startup: string): string | null => {
  if (!role) return null;
  const matches = (id: string) => roleSegments(id).includes(role);
  if (startup && models.includes(startup) && matches(startup)) return startup;
  const candidates = models.filter(matches);
  return candidates.length > 1 ? [...candidates].sort(newerVersion)[0] : candidates[0] ?? null;
};

// Mirrors the backend's [1m] guard: a known context below 1M must not be
// marked (Claude Code would ask for a context the upstream refuses), while
// unknown models follow the explicit opt-in.
export const MIN_ONE_M_CONTEXT = 1_000_000;
export const effectiveContext = (provider: NewApiProvider | undefined, id: string | null | undefined): number | null => {
  if (!provider || !id) return null;
  if (provider.contextWindow != null) return provider.contextWindow;
  const model = provider.models.find(entry => entry.id === id);
  return model ? model.overrides?.contextWindow ?? model.contextWindow ?? null : null;
};
export const knownSmallContext = (context: number | null): boolean =>
  context != null && context < MIN_ONE_M_CONTEXT;
export const clientProtocols = (client: ClientStatus, mode: ClientAccessMode): ModelProtocol[] =>
  client.targetId === "claudeCode" && mode === "router"
    ? ["anthropicMessages", "responses", "chatCompletions"] : client.protocols;

export const clientModels = (provider: NewApiProvider, protocols: ModelProtocol[]) => {
  const ids = Array.from(new Set([provider.model, ...provider.models.filter(model => model.included).map(model => model.id)])).filter(Boolean);
  return ids.filter(id => {
    const protocol = providerModelProtocol(provider, id);
    const metadata = provider.models.find(model => model.id === id);
    return protocols.includes(protocol) && (!metadata || supportsProtocol(metadata, protocol));
  });
};

export const exclusionOf = (provider: NewApiProvider, protocols: ModelProtocol[]): Exclusion | null => {
  if (!provider.enabled) return "disabled";
  if (!provider.hasApiKey) return "noApiKey";
  if (!provider.model && !provider.models.some(model => model.included)) return "noModel";
  return clientModels(provider, protocols).length ? null : "protocol";
};

/**
 * Saved user limits, only for the dialog's placeholder and restore flow.
 * Effective values are resolved by the backend (user overrides, provider
 * setting, upstream declaration, models.dev catalog, then the shared
 * fallback); the UI never derives them from model names.
 */
export const clientDefaults = (provider: NewApiProvider | undefined, model: string) => {
  const metadata = provider?.models.find(entry => entry.id === model);
  const overrides = metadata?.overrides ?? null;
  const contextWindow =
    overrides?.contextWindow
    ?? (provider?.model === model ? provider?.contextWindow ?? null : null);
  const maxOutputTokens =
    overrides?.maxOutputTokens ?? null;
  return { contextWindow, maxOutputTokens };
};
