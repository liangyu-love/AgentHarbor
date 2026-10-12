import assert from "node:assert/strict";
import { manifest } from "./release.mjs";

// The updater manifest a GitHub release carries; no files or network.
const latest = manifest({ version: "1.0.1", notes: "Notes", signature: "c2ln\n", date: new Date("2026-10-12T09:30:15.123Z") });
assert.deepEqual(latest, {
  version: "1.0.1",
  notes: "Notes",
  pub_date: "2026-10-12T09:30:15Z",
  platforms: {
    "windows-x86_64": {
      signature: "c2ln",
      url: "https://github.com/liangyu-love/AgentHarbor/releases/download/v1.0.1/AgentHarbor_1.0.1_x64-setup.exe",
    },
  },
});
console.log("PASS the release manifest points the updater at the version's installer with its signature");
