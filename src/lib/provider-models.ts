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
