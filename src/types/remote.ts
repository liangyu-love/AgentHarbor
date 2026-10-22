import type { ClientStatus } from "./clients";

export type RemoteAuth = "key" | "password";

/** A reverse tunnel from a host back to this machine's router. */
export type TunnelState =
  | { state: "connecting" }
  | { state: "connected"; since: number }
  /** The last attempt failed (an i18n key); another follows. */
  | { state: "retrying"; error: string };

/** What `~/.ssh/config` and the defaults make of a host. */
export interface ResolvedRemoteHost {
  hostName: string;
  port: number;
  user: string | null;
  identityFiles: string[];
}

/** A remote host. Its password and key passphrase are never sent back. */
export interface RemoteHost {
  id: string;
  name: string;
  /** A `~/.ssh/config` alias, a host name or an address. */
  host: string;
  port: number | null;
  user: string | null;
  auth: RemoteAuth;
  identityFile: string | null;
  hasPassword: boolean;
  hasPassphrase: boolean;
  resolved: ResolvedRemoteHost;
  /** Clients on this host are configured by AgentHarbor. */
  inUse: boolean;
  /** The server port the tunnel listens on. */
  tunnelPort: number;
  /** A client on this host goes through this machine's router. */
  routed: boolean;
  /** The tunnel, while one runs. */
  tunnel: TunnelState | null;
}

/** A host as edited; empty secrets keep what is stored. */
export interface RemoteHostInput {
  id?: string | null;
  name: string;
  host: string;
  port?: number | null;
  user?: string | null;
  auth: RemoteAuth;
  identityFile?: string | null;
  password?: string | null;
  passphrase?: string | null;
  clearPassphrase?: boolean;
  tunnelPort?: number | null;
}

export type RemoteConnectionTest =
  | { status: "connected"; home: string }
  /** A host seen for the first time: its key is trusted once the user confirms this fingerprint. */
  | { status: "unknownHost"; fingerprint: string; algorithm: string }
  | { status: "hostKeyChanged"; fingerprint: string }
  | { status: "failed"; error: string };

export interface RemoteCodexRequest {
  providerId: string;
  model: string;
  /** Codex's web_search tool; off writes `web_search = "disabled"`, since a relay may refuse the tool. */
  webSearch: boolean;
  /** Written to `<name>.config.toml` (run `codex -p <name>`), leaving `config.toml` as it was; null makes the relay the default. */
  profile?: string | null;
  /** Through this machine's router over the host's tunnel; `model` is then a router model id. */
  router?: boolean;
}

/** One model of the router's Codex menu, as a routed remote Codex is offered it. */
export interface RouterModel {
  id: string;
  label: string;
}

export interface RemoteCodexStatus {
  path: string;
  configured: boolean;
  request: RemoteCodexRequest | null;
  conflict: boolean;
  conflictFields: string[];
  stale: boolean;
  error: string | null;
}

export interface RemoteCodexPreview {
  path: string;
  revision: string;
  content: string;
  conflictFields: string[];
}

export interface RemoteClients {
  home: string;
  codex: RemoteCodexStatus;
  claudeCode: ClientStatus;
}
