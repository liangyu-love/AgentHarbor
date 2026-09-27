import type { ModelProtocol, NewApiModel, NewApiProvider } from "@/types/newapi";

const declaredModelProtocol = (model: NewApiModel | undefined): ModelProtocol | undefined => {
  const endpoints = model?.endpointTypes.map(type => type.toLowerCase()) ?? [];
  if (endpoints.some(type => type.includes("response"))) return "responses";
  if (endpoints.some(type => type.includes("anthropic") || type.includes("message"))) return "anthropicMessages";
  if (endpoints.some(type => type.includes("chat") || type.includes("completion"))) return "chatCompletions";
  return undefined;
};

export const effectiveModelProtocol = (model: NewApiModel | undefined, defaultProtocol: ModelProtocol): ModelProtocol =>
  model?.protocol ?? (model?.id.toLowerCase().includes("claude")
    ? declaredModelProtocol(model) ?? "anthropicMessages" : defaultProtocol);

export const providerModelProtocol = (provider: NewApiProvider, modelId = provider.model): ModelProtocol => {
  const model = provider.models.find(entry => entry.id === modelId);
  return model?.protocol ?? (modelId.toLowerCase().includes("claude")
    ? declaredModelProtocol(model) ?? "anthropicMessages" : provider.protocol);
};

export const includedModels = (models: NewApiModel[]) => models.filter(model => model.included);

/** Keep catalog refreshes from replacing local inclusion, aliases, limits or protocol choices. */
export const mergeModelCatalog = (current: NewApiModel[], fetched: NewApiModel[]): NewApiModel[] => {
  const existingById = new Map(current.map(model => [model.id, model]));
  const fetchedIds = new Set(fetched.map(model => model.id));
  const merged = fetched.map(model => {
    const existing = existingById.get(model.id);
    if (!existing) return model;
    const edited = existing.metadataSource === "manual";
    const renamed = existing.displayName.trim() !== "" && existing.displayName.trim() !== existing.id;
    return {
      ...model,
      protocol: existing.protocol ?? null,
      included: existing.included,
      displayName: edited || renamed ? existing.displayName : model.displayName,
      contextWindow: edited ? existing.contextWindow : model.contextWindow,
      maxOutputTokens: edited ? existing.maxOutputTokens : model.maxOutputTokens,
      metadataSource: edited ? "manual" as const : model.metadataSource,
    };
  });
  return [...merged, ...current.filter(model => !fetchedIds.has(model.id))];
};

export const supportsProtocol = (model: NewApiModel, protocol: ModelProtocol) => {
  const endpoints = model.endpointTypes.map(type => type.toLowerCase());
  if (endpoints.length === 0) return true;
  if (protocol === "responses") {
    return endpoints.some(type => type.includes("response"))
      || !endpoints.some(type => type.includes("chat") || type.includes("completion") || type.includes("anthropic") || type.includes("message"));
  }
  if (protocol === "chatCompletions") {
    return endpoints.some(type => type.includes("chat") || type.includes("completion"))
      || !endpoints.some(type => type.includes("response") || type.includes("anthropic") || type.includes("message"));
  }
  return endpoints.some(type => type.includes("anthropic") || type.includes("message"))
    || !endpoints.some(type => type.includes("response") || type.includes("chat") || type.includes("completion"));
};
