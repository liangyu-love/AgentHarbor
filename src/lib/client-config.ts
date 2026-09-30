import { providerModelProtocol, supportsProtocol } from "./provider-models";
import type { ClaudeModels, ClaudeOneM, ClientAccessMode, ClientStatus } from "@/types/clients";
import type { ModelProtocol, NewApiProvider } from "@/types/newapi";

export type Exclusion = "protocol" | "disabled" | "noApiKey" | "noModel";
export const claudeRoles: (keyof ClaudeModels)[] = ["sonnet", "opus", "fable", "haiku", "subagent"];
export const emptyClaudeRoles = (): ClaudeModels => ({ sonnet: "", opus: "", fable: "", haiku: "", subagent: "" });
// Missing fields adopt the new default; explicit false in old profiles stays off.
export const claudeOneMDefaults = (saved?: Partial<ClaudeOneM> | null): ClaudeOneM => ({
  startup: saved?.startup ?? true, sonnet: saved?.sonnet ?? true,
  opus: saved?.opus ?? true, fable: saved?.fable ?? true,
  haiku: saved?.haiku ?? true, subagent: saved?.subagent ?? true,
});
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

export const clientDefaults = (provider: NewApiProvider | undefined, model: string) => {
  const metadata = provider?.models.find(entry => entry.id === model);
  const compact = model.toLowerCase().replace(/[^a-z0-9]/g, "");
  const contextWindow = metadata?.contextWindow
    ?? (provider?.model === model ? provider.contextWindow : null)
    ?? (compact.includes("claude") ? 1_000_000 : 128_000);
  return { contextWindow, maxOutputTokens: Math.min(metadata?.maxOutputTokens ?? 8192, contextWindow) };
};
