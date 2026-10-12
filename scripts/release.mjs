// Builds the signed Windows installer and the files a GitHub release carries:
// the installer, its updater signature, latest.json and SHA256SUMS.txt.
//
//   node scripts/release.mjs [--notes notes.md] [--skip-build]
//
// The updater's private key stays on this machine: TAURI_SIGNING_PRIVATE_KEY,
// TAURI_SIGNING_PRIVATE_KEY_PATH, or ~/.tauri/agentharbor-updater.key.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REPOSITORY = "liangyu-love/AgentHarbor";
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const option = name => {
  const index = args.indexOf(name);
  return index >= 0 ? args[index + 1] : undefined;
};

/** The updater manifest GitHub serves at releases/latest/download/latest.json. */
export function manifest({ version, notes, signature, date = new Date() }) {
  const installer = `AgentHarbor_${version}_x64-setup.exe`;
  return {
    version,
    notes,
    pub_date: date.toISOString().replace(/\.\d{3}Z$/, "Z"),
    platforms: {
      "windows-x86_64": {
        signature: signature.trim(),
        url: `https://github.com/${REPOSITORY}/releases/download/v${version}/${installer}`,
      },
    },
  };
}

function run(command, commandArgs, env = process.env) {
  const result = spawnSync(command, commandArgs, { cwd: root, env, stdio: "inherit", shell: process.platform === "win32" && !path.isAbsolute(command) });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function signingEnv() {
  const env = { ...process.env };
  if (!env.TAURI_SIGNING_PRIVATE_KEY) {
    const keyPath = env.TAURI_SIGNING_PRIVATE_KEY_PATH ?? path.join(homedir(), ".tauri", "agentharbor-updater.key");
    if (!existsSync(keyPath)) {
      console.error(`The updater signing key is missing: ${keyPath}\nRestore it from your backup; a new key cannot update installs signed with the old one.`);
      process.exit(1);
    }
    env.TAURI_SIGNING_PRIVATE_KEY = readFileSync(keyPath, "utf8").trim();
  }
  env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ??= "";
  return env;
}

function main() {
  const version = JSON.parse(readFileSync(path.join(root, "src-tauri/tauri.conf.json"), "utf8")).version;
  if (!args.includes("--skip-build")) {
    const env = signingEnv();
    run(process.execPath, [path.join(root, "node_modules/typescript/bin/tsc")], env);
    run(process.execPath, [path.join(root, "node_modules/vite/bin/vite.js"), "build"], env);
    run(process.execPath, [path.join(root, "scripts/tauri.mjs"), "build", "--config", JSON.stringify({ build: { beforeBuildCommand: "" } })], env);
  }
  const bundle = path.join(root, "src-tauri/target/release/bundle/nsis");
  const installer = path.join(bundle, `AgentHarbor_${version}_x64-setup.exe`);
  const signaturePath = `${installer}.sig`;
  if (!existsSync(installer) || !existsSync(signaturePath)) {
    console.error(`Missing ${path.basename(installer)} or its .sig in ${bundle}; build with the signing key first.`);
    process.exit(1);
  }
  const notesPath = option("--notes");
  const notes = notesPath ? readFileSync(notesPath, "utf8").trim() : `AgentHarbor v${version}`;
  const latest = manifest({ version, notes, signature: readFileSync(signaturePath, "utf8") });
  writeFileSync(path.join(bundle, "latest.json"), JSON.stringify(latest, null, 2) + "\n");
  const digest = createHash("sha256").update(readFileSync(installer)).digest("hex");
  writeFileSync(path.join(bundle, "SHA256SUMS.txt"), `${digest}  ${path.basename(installer)}\n`);
  console.log(`Release files for v${version} in ${bundle}:`);
  for (const name of [path.basename(installer), path.basename(signaturePath), "latest.json", "SHA256SUMS.txt"]) console.log(`  ${name}`);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
