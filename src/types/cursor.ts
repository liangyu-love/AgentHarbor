import type { ModelSources } from "@/types/gateway";

/** Whether this machine trusts the local certificate Cursor IDE is read with. */
export type CursorTrust = "trusted" | "untrusted" | "manual";
export type CursorCommitLocale = "english" | "chinese";

export interface CursorModel {
  /** The gateway id; settings name a model by it. */
  id: string;
  /** The id Cursor lists the model under. */
  cursorId: string;
  label: string;
  sourceName: string;
  kind: "relay" | "account" | "aggregate";
}

export interface CursorStatus {
  ideInstalled: boolean;
  cliInstalled: boolean;
  /** The user-wide CURSOR_API_ENDPOINT can be managed here (Windows). */
  cliSupported: boolean;
  settingsPath: string | null;
  ide: boolean;
  cli: boolean;
  running: boolean;
  /** Cursor IDE runs now: its button restarts it rather than starting it. */
  ideRunning?: boolean;
  port: number | null;
  url: string | null;
  /** Cursor's settings.json points at the local port right now. */
  ideApplied: boolean;
  /** The user variable points at the local port right now. */
  cliApplied: boolean;
  cliCommand: string | null;
  certificate: CursorTrust;
  certificatePath: string | null;
  trustCommand: string | null;
  sources: ModelSources;
  models: CursorModel[];
  tabModel: string | null;
  commitModel: string | null;
  commitLocale: CursorCommitLocale;
  webSearchReady: boolean;
  routerRunning: boolean;
}
