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
