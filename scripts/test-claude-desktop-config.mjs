// Verification for the Claude Desktop direct-connection feature.
//
// The card and dialog are TSX components, so this script stays DOM-free: it
// parses the locales, the Rust DTOs/commands and the TypeScript sources and
// asserts the contracts that tie them together. It runs standalone and is also
// part of the package.json "test:frontend" entry.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const read = relative => readFileSync(path.join(root, relative), "utf8");

const CARD = "src/components/newapi/claude-desktop-config-card.tsx";
const DIALOG = "src/components/newapi/claude-desktop-config-dialog.tsx";
const TYPES = "src/types/claude-desktop.ts";
const LIB = "src/lib/claude-desktop.ts";
const QUERY_KEYS = "src/lib/query-keys.ts";
const COMMANDS_FILE = "src-tauri/src/commands/claude_desktop.rs";
const RUST_DIR = "src-tauri/src/core/claude_desktop";
const RUST_TYPES = RUST_DIR + "/types.rs";
const LIB_RS = "src-tauri/src/lib.rs";

// Tauri commands of the feature, in registration order.
const COMMANDS = ["get_claude_desktop_status", "list_claude_desktop_candidates",
  "restart_claude_desktop",
  "preview_claude_desktop_change", "commit_claude_desktop_change", "probe_claude_desktop_provider"];
// Enum values the UI interpolates into locale keys at runtime.
const ACTIONS = ["applyDirect", "reapply", "restorePrevious", "switchOfficial", "release", "recoverRollback"];
const STATES = ["managed", "unmanaged", "stale", "orphaned", "conflict", "recoveryRequired"];
const PROBE_STATUSES = ["notChecked", "modelsAccessible", "authRejected", "endpointUnavailable",
  "networkFailed", "invalidResponse", "rateLimited"];
// `probe::MessagesStatus` 的变体；同样由后端在运行时选取。
const MESSAGES_STATUSES = ["notChecked", "messagesAccessible", "authRejected", "endpointUnavailable",
  "requestRejected", "rateLimited", "networkFailed", "serverError", "invalidResponse"];
const CONFIRM_SUFFIXES = ["restoreTitle", "restoreDescription", "reapplyTitle", "reapplyDescription", "switchOfficialTitle",
  "switchOfficialDescription", "releaseTitle", "releaseDescription", "recoverTitle", "recoverDescription"];
// A zh leaf may stay ASCII only when it is nothing but these product nouns.
const PROPER_NOUNS = ["Claude Desktop", "Claude Code", "AgentHarbor", "1M", "Bearer", "/v1/models", "3P", "URL", "JSON", "Desktop"];
const PROPER_NOUN_LEAVES = new Set(["title"]);
const SECRET = /sk-[A-Za-z0-9_-]{16,}/;
const PROVIDER_KEY_FIELDS = /\b(revealApiKey|accessToken|apiKey)\b/;
const CLAUDE_CODE_MODULES = /client-config|claude-model-config/;
// Rust view struct -> TS interface (the TS side prefixes the view names).
const DTO_PAIRS = {
  DesktopStatus: "ClaudeDesktopStatus",
  DesktopPreview: "ClaudeDesktopPreview",
  DesktopApplied: "ClaudeDesktopApplied",
  DesktopCandidates: "ClaudeDesktopCandidates",
  ModelCandidateView: "ClaudeDesktopModelCandidate",
  ProviderCandidateView: "ClaudeDesktopProviderCandidate",
  FileChangeView: "ClaudeDesktopFileChange",
  ManagedModelView: "ClaudeDesktopManagedModel",
  DesktopError: "ClaudeDesktopError",
  DesktopActionView: "ClaudeDesktopActionView",
};

const group = (name, body) => {
  try {
    body();
    console.log("PASS " + name);
  } catch (error) {
    console.error("FAIL " + name);
    console.error("     " + String(error.message ?? error).split("\n").join("\n     "));
    process.exit(1);
  }
};
const flatten = (value, prefix, out = new Map()) => {
  for (const [key, entry] of Object.entries(value ?? {})) {
    const full = prefix + "." + key;
    if (entry && typeof entry === "object") flatten(entry, full, out);
    else out.set(full, entry);
  }
  return out;
};
const camel = name => name.replace(/_([a-z0-9])/g, (_, char) => char.toUpperCase());
const assertEmpty = (problems, message) => assert.ok(problems.length === 0, message + ": " + problems.join("; "));
const fieldsOf = (source, pattern) => {
  const found = new Map();
  for (const [, name, body] of source.matchAll(pattern)) {
    found.set(name, [...body.matchAll(/^\s*(?:pub )?(\w+)\??\s*:/gm)].map(match => match[1]));
  }
  return found;
};

const zh = flatten(JSON.parse(read("src/locales/zh.json")).claudeDesktop, "claudeDesktop");
const en = flatten(JSON.parse(read("src/locales/en.json")).claudeDesktop, "claudeDesktop");

group("locales: zh/en claudeDesktop key sets are identical and every leaf is non-empty text", () => {
  assert.ok(zh.size > 0, "the claudeDesktop namespace is missing from src/locales/zh.json");
  const zhKeys = [...zh.keys()].sort();
  const enKeys = [...en.keys()].sort();
  assertEmpty([...zhKeys.filter(key => !en.has(key)).map(key => "zh only: " + key),
    ...enKeys.filter(key => !zh.has(key)).map(key => "en only: " + key)],
    "claudeDesktop key sets differ between zh.json and en.json");
  const bad = [...zh, ...en]
    .filter(([, value]) => typeof value !== "string" || value.trim() === "")
    .map(([key, value]) => key + "=" + JSON.stringify(value));
  assertEmpty(bad, "empty or non-string leaves");
});

group("locales: zh leaves contain CJK unless they are allow-listed product nouns", () => {
  const offenders = [];
  for (const [key, value] of zh) {
    if (/[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/.test(value)) continue;
    const residual = PROPER_NOUNS.reduce((text, noun) => text.split(noun).join(" "), value);
    const englishProse = /\b[a-z]{3,}\b/.test(en.get(key) ?? "");
    if (/[A-Za-z]/.test(residual) || (englishProse && !PROPER_NOUN_LEAVES.has(key))) {
      offenders.push(key + " = " + JSON.stringify(value) + " (en: " + JSON.stringify(en.get(key)) + ")");
    }
  }
  assertEmpty(offenders, "untranslated zh leaves");
});

const SOURCES = [
  { label: CARD, text: read(CARD) },
  { label: DIALOG, text: read(DIALOG) },
  ...readdirSync(path.join(root, RUST_DIR)).filter(name => name.endsWith(".rs")).sort()
    .map(name => ({ label: RUST_DIR + "/" + name, text: read(RUST_DIR + "/" + name) })),
];
const literalKeys = new Set();
const dynamicPrefixes = new Set();
for (const { text } of SOURCES) {
  // The lookbehind keeps api.claudeDesktop.status from looking like a locale key.
  for (const [raw] of text.matchAll(/(?<![\w.])claudeDesktop\.[A-Za-z0-9_.]*/g)) {
    if (raw.endsWith(".")) dynamicPrefixes.add(raw);
    else literalKeys.add(raw);
  }
}
// A null value set means the suffix is produced by the backend at runtime; only
// the static fallback of that expression can be asserted.
const DYNAMIC_SUFFIXES = new Map([
  ["claudeDesktop.actions.", ACTIONS],
  ["claudeDesktop.state.", STATES],
  ["claudeDesktop.probe.", PROBE_STATUSES],
  ["claudeDesktop.messages.", MESSAGES_STATUSES],
  ["claudeDesktop.dialog.", [...CONFIRM_SUFFIXES, "before", "after"]],
  ["claudeDesktop.errors.", null],
  ["claudeDesktop.fileRoles.", ["normalConfig", "thirdPartyConfig", "profile", "meta"]],
  ["claudeDesktop.fileKinds.", ["create", "update", "delete", "noop"]],
]);

group("locales: every claudeDesktop.* key literal in the card, dialog and Rust module exists", () => {
  const missing = [...literalKeys].filter(key => !zh.has(key)).sort();
  assertEmpty(missing, "referenced but not defined in the locales");
});

group("locales: interpolated claudeDesktop keys resolve for every known enum value", () => {
  const unresolved = [];
  for (const prefix of [...dynamicPrefixes].sort()) {
    assert.ok(DYNAMIC_SUFFIXES.has(prefix),
      "unhandled dynamic locale prefix " + prefix + "; teach this script its value set");
    const suffixes = DYNAMIC_SUFFIXES.get(prefix);
    if (!suffixes) {
      assert.ok(zh.has(prefix + "storage"), "no fallback key " + prefix + "storage for " + prefix);
      continue;
    }
    for (const suffix of suffixes) if (!zh.has(prefix + suffix)) unresolved.push(prefix + suffix);
  }
  assertEmpty(unresolved, "dynamic keys missing from the locales");
});

group("errors: every Rust reason/error code resolves to a camelCase locale key", () => {
  const codes = [...new Set([...read(RUST_TYPES).matchAll(/pub const [A-Z0-9_]+: &str = "([a-z0-9_]+)"/g)].map(m => m[1]))]
    // Only identifier-like codes: skip profile/auth/role literals that are not error or reason codes.
    .filter(code => /^[a-z]+(_[a-z0-9]+)+$/.test(code));
  assert.ok(codes.length > 15, "expected to find the reason/error code table, found " + codes.length);
  const unresolved = codes.filter(code => !zh.has("claudeDesktop.errors." + camel(code)));
  assertEmpty(unresolved, "Rust codes without a claudeDesktop.errors.<camelCase> locale entry");
  const unresolvedEn = codes.filter(code => !en.has("claudeDesktop.errors." + camel(code)));
  assertEmpty(unresolvedEn, "Rust codes without an English claudeDesktop.errors.<camelCase> locale entry");
  // The dialog must map snake_case to camelCase and must take the code before any ":".
  const dialog = read(DIALOG);
  assert.ok(/replace\(\/_\(\[a-z0-9\]\)\/g/.test(dialog), "dialog no longer converts snake_case reason codes to camelCase");
  assert.ok(!/reason\?\.split\(":"\)\.slice\(-1\)/.test(dialog), "dialog still takes the last colon segment of a reason code, which yields a URL parameter");
  assert.ok(/split\(":"\)\[0\]/.test(dialog), "dialog no longer takes the code before the colon");
});
group("DTOs: every Rust DTO field appears in the matching TS interface", () => {
  const structs = fieldsOf(read(RUST_TYPES), /pub struct (\w+)\s*\{([\s\S]*?)\n\}/g);
  const interfaces = fieldsOf(read(TYPES), /export interface (\w+)[^{]*\{([\s\S]*?)\n\}/g);
  const problems = [];
  for (const [structName, interfaceName] of Object.entries(DTO_PAIRS)) {
    const rustFields = structs.get(structName);
    const tsFields = interfaces.get(interfaceName);
    if (!rustFields) { problems.push("Rust struct " + structName + " not found"); continue; }
    if (!tsFields) { problems.push("TS interface " + interfaceName + " not found"); continue; }
    const missing = rustFields.map(camel).filter(field => !tsFields.includes(field));
    if (missing.length) problems.push(structName + " -> " + interfaceName + ": missing " + missing.join(", "));
  }
  assertEmpty(problems, "DTO field mismatches");
});

group("commands: every Desktop Tauri command is declared, registered and invoked by name", () => {
  const declared = [...read(COMMANDS_FILE).matchAll(/pub async fn (\w+)/g)].map(match => match[1]);
  const undeclared = COMMANDS.filter(name => !declared.includes(name));
  assertEmpty(undeclared, COMMANDS_FILE + " does not declare");

  const handler = read(LIB_RS).match(/generate_handler!\[([\s\S]*?)\n\s*\]/);
  assert.ok(handler, "no generate_handler! [ ... ] list found in " + LIB_RS);
  const registered = [...handler[1].matchAll(/commands::claude_desktop::(\w+)/g)].map(match => match[1]);
  assertEmpty(registered.slice().sort().filter((name, index, all) => all.indexOf(name) === index)
    .filter(name => !COMMANDS.includes(name)).concat(COMMANDS.filter(name => !registered.includes(name))),
  LIB_RS + " registers");

  const invoked = [...read(LIB).matchAll(/invokeDesktop<[^>]*>\(\s*"([a-z_]+)"/g)].map(match => match[1]);
  assertEmpty(invoked.filter(name => !COMMANDS.includes(name)).concat(COMMANDS.filter(name => !invoked.includes(name))),
    LIB + " invokes");
});

group("secrets: no key material and no provider credential fields in the Desktop flow", () => {
  const offenders = [];
  for (const [key, value] of [...zh, ...en]) if (SECRET.test(value)) offenders.push(key + " looks like an API key");
  for (const file of [TYPES, LIB, CARD, DIALOG]) {
    const text = read(file);
    if (SECRET.test(text)) offenders.push(file + " contains a hard-coded API key");
    const hit = text.match(PROVIDER_KEY_FIELDS);
    if (hit) offenders.push(file + " references provider field " + hit[1]);
  }
  assertEmpty(offenders, "secret-safety violations");
});

group("independence: the card owns a labelled section and neither component uses Claude Code config", () => {
  const card = read(CARD);
  const labelled = card.match(/<section[^>]*aria-labelledby="([^"]+)"/);
  assert.ok(labelled, CARD + " has no <section aria-labelledby=...>");
  assert.ok(card.includes('id="' + labelled[1] + '"'), CARD + " has no element with id=" + labelled[1]);
  const problems = [];
  for (const file of [CARD, DIALOG]) {
    for (const [, specifier] of read(file).matchAll(/^\s*import[\s\S]*?from\s+"([^"]+)";/gm)) {
      if (CLAUDE_CODE_MODULES.test(specifier)) problems.push(file + " imports " + specifier);
    }
  }
  assertEmpty(problems, "Claude Code coupling");
});

group("query keys: shared constants exist and the card uses them instead of raw literals", () => {
  const body = read(QUERY_KEYS).match(/export const queryKeys\s*=\s*\{([\s\S]*?)\n\}/);
  assert.ok(body, "no queryKeys object found in " + QUERY_KEYS);
  for (const name of ["claudeDesktopStatus", "claudeDesktopCandidates"]) {
    assert.match(body[1], new RegExp("\\b" + name + "\\s*:"), "queryKeys." + name + " is not exported");
  }
  const card = read(CARD);
  for (const name of ["claudeDesktopStatus", "claudeDesktopCandidates"]) {
    assert.ok(card.includes("queryKeys." + name), CARD + " does not use queryKeys." + name);
  }
  assert.doesNotMatch(card, /\[\s*["']claude-desktop["']/, CARD + " builds a raw claude-desktop query key");
});

console.log("claude-desktop-config: all check groups passed");
