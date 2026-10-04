import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

// Tauri on Windows uses the MSVC toolchain. Keep this local to the child
// process so a user's global GNU toolchain selection remains untouched.
const env = { ...process.env };
if (process.platform === "win32" && !env.RUSTUP_TOOLCHAIN) {
  env.RUSTUP_TOOLCHAIN = "stable-x86_64-pc-windows-msvc";
}
const args = process.argv.slice(2);
const testing = args[0] === "--test-rust";
const result = testing
  ? spawnSync("cargo", ["test", "--manifest-path", "src-tauri/Cargo.toml", "--lib", "--test", "local_workflows", "--test", "claude_desktop_workflows", ...args.slice(1)], { env, stdio: "inherit" })
  : spawnSync(process.execPath, [fileURLToPath(new URL("../node_modules/@tauri-apps/cli/tauri.js", import.meta.url)), ...args], { env, stdio: "inherit" });
if (result.error) console.error(result.error.message);
process.exit(result.status ?? 1);
