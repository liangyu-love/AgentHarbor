// Builds the committed models.dev snapshot consumed by the app when it is
// offline. Run explicitly for a release or when catalog data should be
// refreshed; normal builds only read the committed file.
//
//   node scripts/build-models-dev-snapshot.mjs [--source <path-or-url>]
//
// Default source is https://models.dev/api.json. The transform keeps the
// fields the resolver consumes (limits, reasoning options, cost, modalities,
// status, dates, canonical id) and drops descriptions, logos and benchmarks.
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { fileURLToPath } from "node:url";

const SOURCE_URL = "https://models.dev/api.json";
const SCHEMA_VERSION = 1;
const target = fileURLToPath(new URL("../src-tauri/assets/models-dev.snapshot.json", import.meta.url));

function argument(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

async function readSource() {
  const source = argument("--source") ?? SOURCE_URL;
  if (!/^https?:\/\//.test(source)) {
    return { text: readFileSync(source, "utf8"), sourceUrl: SOURCE_URL };
  }
  const response = await fetch(source, { redirect: "follow" });
  if (!response.ok) {
    throw new Error(`models.dev responded with HTTP ${response.status}`);
  }
  return { text: await response.text(), sourceUrl: source };
}

function number(value) {
  return typeof value === "number" && Number.isFinite(value) ? value : undefined;
}

function rates(cost) {
  const result = {};
  for (const [key, value] of [
    ["input", number(cost.input)],
    ["output", number(cost.output)],
    ["cacheRead", number(cost.cache_read)],
    ["cacheWrite", number(cost.cache_write)],
    ["reasoning", number(cost.reasoning)],
    ["inputAudio", number(cost.input_audio)],
    ["outputAudio", number(cost.output_audio)],
  ]) {
    if (value !== undefined) result[key] = value;
  }
  return result;
}

function transformCost(cost) {
  if (!cost || typeof cost !== "object") return undefined;
  const result = rates(cost);
  if (Array.isArray(cost.tiers)) {
    const tiers = cost.tiers
      .map((tier) => {
        const entry = rates(tier);
        const condition = tier.tier ?? {};
        if (typeof condition.type !== "string") return undefined;
        entry.tier = { type: condition.type };
        const size = number(condition.size);
        if (size !== undefined && size > 0) entry.tier.size = size;
        return entry;
      })
      .filter(Boolean);
    if (tiers.length > 0) result.tiers = tiers;
  }
  if (cost.context_over_200k && typeof cost.context_over_200k === "object") {
    result.contextOver200k = rates(cost.context_over_200k);
  }
  return Object.keys(result).length > 0 ? result : undefined;
}

function transformOptions(options) {
  if (!Array.isArray(options)) return [];
  return options
    .map((option) => {
      if (!option || typeof option.type !== "string") return undefined;
      if (option.type === "effort") {
        const values = Array.isArray(option.values)
          ? option.values.filter((value) => typeof value === "string" && value.length > 0)
          : [];
        return values.length > 0 ? { type: "effort", values } : undefined;
      }
      if (option.type === "toggle") return { type: "toggle" };
      if (option.type === "budget_tokens") {
        const entry = { type: "budget_tokens" };
        const min = number(option.min);
        const max = number(option.max);
        // Upstream uses -1 for "unset"; omit it instead of publishing it.
        if (min !== undefined && min > 0) entry.min = min;
        if (max !== undefined && max > 0) entry.max = max;
        return entry;
      }
      return undefined;
    })
    .filter(Boolean);
}

function transformModel(model) {
  if (!model || typeof model.id !== "string" || model.id.length === 0) return undefined;
  const transformed = { id: model.id };
  if (typeof model.name === "string" && model.name.length > 0) transformed.name = model.name;
  if (typeof model.family === "string" && model.family.length > 0) transformed.family = model.family;
  if (typeof model.reasoning === "boolean") transformed.reasoning = model.reasoning;
  const options = transformOptions(model.reasoning_options);
  if (options.length > 0) transformed.reasoningOptions = options;
  const limit = model.limit ?? {};
  const limits = {};
  for (const key of ["context", "input", "output"]) {
    const value = number(limit[key]);
    if (value !== undefined && value > 0) limits[key] = value;
  }
  if (Object.keys(limits).length > 0) transformed.limit = limits;
  const cost = transformCost(model.cost);
  if (cost) transformed.cost = cost;
  const modalities = model.modalities;
  if (modalities && typeof modalities === "object") {
    const entry = {};
    if (Array.isArray(modalities.input)) entry.input = modalities.input.filter((value) => typeof value === "string");
    if (Array.isArray(modalities.output)) entry.output = modalities.output.filter((value) => typeof value === "string");
    if (Object.keys(entry).length > 0) transformed.modalities = entry;
  }
  for (const [key, from] of [
    ["attachment", "attachment"],
    ["toolCall", "tool_call"],
    ["structuredOutput", "structured_output"],
    ["temperature", "temperature"],
    ["openWeights", "open_weights"],
    ["interleaved", "interleaved"],
  ]) {
    if (typeof model[from] === "boolean") transformed[key] = model[from];
  }
  if (typeof model.knowledge === "string" && model.knowledge.length > 0) transformed.knowledge = model.knowledge;
  if (typeof model.release_date === "string") transformed.releaseDate = model.release_date;
  if (typeof model.last_updated === "string") transformed.lastUpdated = model.last_updated;
  if (typeof model.canonical_model_id === "string" && model.canonical_model_id.length > 0) {
    transformed.canonicalModelId = model.canonical_model_id;
  }
  if (typeof model.status === "string" && model.status.length > 0) transformed.status = model.status;
  // Diagnostics-only hints. Both are opaque to the app and never drive
  // protocol selection or request shapes.
  if (model.provider && typeof model.provider === "object") transformed.providerHint = model.provider;
  if (model.experimental && typeof model.experimental === "object") transformed.experimental = model.experimental;
  return transformed;
}

function transformProvider(provider) {
  if (!provider || typeof provider.id !== "string" || provider.id.length === 0) return undefined;
  const models = Object.values(provider.models ?? {})
    .map(transformModel)
    .filter(Boolean)
    .sort((left, right) => left.id.localeCompare(right.id));
  const transformed = { id: provider.id };
  if (typeof provider.name === "string" && provider.name.length > 0) transformed.name = provider.name;
  if (typeof provider.npm === "string" && provider.npm.length > 0) transformed.npm = provider.npm;
  if (typeof provider.api === "string" && provider.api.length > 0) transformed.api = provider.api;
  if (Array.isArray(provider.env)) {
    const env = provider.env.filter((value) => typeof value === "string" && value.length > 0);
    if (env.length > 0) transformed.env = env;
  }
  if (typeof provider.doc === "string" && provider.doc.length > 0) transformed.doc = provider.doc;
  transformed.models = models;
  return transformed;
}

const { text, sourceUrl } = await readSource();
const raw = JSON.parse(text);
const providers = Object.values(raw)
  .map(transformProvider)
  .filter(Boolean)
  .sort((left, right) => left.id.localeCompare(right.id));

const modelCount = providers.reduce((sum, provider) => sum + provider.models.length, 0);
if (providers.length === 0 || modelCount === 0) {
  throw new Error("models.dev response contained no providers or models");
}

const body = {
  schemaVersion: SCHEMA_VERSION,
  sourceUrl,
  fetchedAt: Math.floor(Date.now() / 1000),
  providers,
};
const canonical = JSON.stringify(body);
body.revision = createHash("sha256").update(canonical).digest("hex");

mkdirSync(dirname(target), { recursive: true });
writeFileSync(target, JSON.stringify(body));
const stats = {
  snapshot: target,
  bytes: Buffer.byteLength(JSON.stringify(body)),
  providers: providers.length,
  models: modelCount,
  revision: body.revision,
};
console.log(JSON.stringify(stats, null, 2));
