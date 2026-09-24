import type { ModelProtocol, NewApiModel } from "@/types/newapi";

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

export const includedModelsForProtocol = (models: NewApiModel[], protocol: ModelProtocol) =>
  models.filter(model => model.included && supportsProtocol(model, protocol));

/**
 * Guess the protocol a relay serves from the capabilities it advertises.
 * Mirrors `core::protocol::infer_protocol` so the dialog and the router agree.
 */
export const inferProtocol = (endpointTypes: string[]): ModelProtocol => {
  const has = (needle: string) => endpointTypes.some(type => type.toLowerCase().includes(needle));
  if (has("response")) return "responses";
  if (has("anthropic") || has("message")) return "anthropicMessages";
  if (has("chat") || has("completion")) return "chatCompletions";
  return "chatCompletions";
};

export const inferProtocolFromModels = (models: NewApiModel[]): ModelProtocol => {
  const advertised = models.find(model => model.endpointTypes.length > 0);
  return advertised ? inferProtocol(advertised.endpointTypes) : "chatCompletions";
};
