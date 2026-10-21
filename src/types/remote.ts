import type { ClientStatus } from "./clients";

export type RemoteAuth = "key" | "password";

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
