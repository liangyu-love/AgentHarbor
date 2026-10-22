import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle, Cable, Check, Fingerprint, KeyRound, Loader2, LockKeyhole, Pencil, Plus, RefreshCw, RotateCcw,
  Send, Server, Settings2, ShieldAlert, Trash2, Unlink, X,
} from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import type { NewApiProvider } from "@/types/newapi";
import type { RemoteClients, RemoteHost, RemoteManagedClient, RemoteProbe } from "@/types/remote";
import { ConfigDialog } from "./client-config-dialog";
import { ConsoleHeader } from "./client-console";
import { ClientGlyph } from "./configuration-shortcuts";
import { RemoteCodexDialog } from "./remote-codex-dialog";
import { RemoteHostDialog, remoteErrorKey } from "./remote-host-dialog";
import { RemoteWebSearch } from "./remote-web-search";

const SELECTED_KEY = "agentharbor.remote-host.v1";
/** The clients set up through the shared configuration dialog, in the console's order. */
const MANAGED: RemoteManagedClient[] = ["claudeCode", "opencode", "pi"];

const target = (host: RemoteHost) => {
  const user = host.user ?? host.resolved.user;
  return `${user ? user + "@" : ""}${host.resolved.hostName}${host.resolved.port === 22 ? "" : ":" + host.resolved.port}`;
};

/** A host's state as its rail shows it: its tunnel's when routed. */
const hostState = (host: RemoteHost) =>
  host.routed ? host.tunnel?.state ?? "stopped" : host.inUse ? "direct" : "idle";

const stateTone: Record<string, string> = {
  connected: "bg-emerald-500", direct: "bg-emerald-500", connecting: "animate-pulse bg-amber-500",
  retrying: "bg-destructive", stopped: "bg-destructive", idle: "bg-muted-foreground/40",
};

// Remote hosts on the injection page, laid out like the client console: the
// hosts in a rail, the chosen one's Codex, Claude Code, OpenCode and Pi beside it, each
// direct or through this machine's router over the host's tunnel.
export function RemoteHostsPanel({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  // A routed host's tunnel is followed: quickly while it connects.
  const hosts = useQuery({
    queryKey: queryKeys.remoteHosts,
    queryFn: api.remote.hosts,
    refetchInterval: query => {
      const routed = (query.state.data ?? []).filter(host => host.routed);
      if (!routed.length) return false;
      return routed.some(host => host.tunnel?.state !== "connected") ? 3_000 : 15_000;
    },
  });
  const [selectedId, setSelectedId] = useState<string | null>(() => {
    try { return localStorage.getItem(SELECTED_KEY); } catch { return null; }
  });
  const [editing, setEditing] = useState<RemoteHost | "new" | null>(null);
  const tabs = useRef<Record<string, HTMLButtonElement | null>>({});
  const list = hosts.data ?? [];
  const selected = list.find(host => host.id === selectedId) ?? list[0];
  const select = (id: string) => {
    setSelectedId(id);
    try { localStorage.setItem(SELECTED_KEY, id); } catch { /* the choice lasts for this visit */ }
  };
  const move = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = event.key === "ArrowDown" || event.key === "ArrowRight" ? index + 1
      : event.key === "ArrowUp" || event.key === "ArrowLeft" ? index - 1
      : event.key === "Home" ? 0 : event.key === "End" ? list.length - 1 : null;
    if (next === null || !list.length) return;
    event.preventDefault();
    const host = list[(next + list.length) % list.length];
    select(host.id);
    tabs.current[host.id]?.focus();
  };

  const dialog = editing && <RemoteHostDialog host={editing === "new" ? null : editing} onClose={() => setEditing(null)}
    onSaved={async saved => {
      setEditing(null);
      select(saved.id);
      await queryClient.invalidateQueries({ queryKey: queryKeys.remoteHosts });
    }} />;

  if (hosts.isError) {
    return <p role="alert" data-remote-hosts className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
      {t(remoteErrorKey(hosts.error))}
    </p>;
  }
  if (hosts.isSuccess && !selected) {
    return (
      <section data-remote-hosts aria-label={t("remote.title")} className="harbor-card">
        <div className="flex flex-col items-center gap-3 px-4 py-10 text-center" data-remote-empty>
          <span className="inject-tab-glyph !h-12 !w-12"><Server aria-hidden className="h-5 w-5 text-primary" /></span>
          <p className="poster-kicker text-[10px] text-muted-foreground">Remote hosts</p>
          <p className="max-w-md text-sm text-muted-foreground">{t("remote.empty")}</p>
          <Button size="sm" className="gap-1.5" onClick={() => setEditing("new")}><Plus className="h-4 w-4" />{t("remote.add")}</Button>
        </div>
        {dialog}
      </section>
    );
  }
  if (!selected) return null;

  return (
    <section className="inject-console harbor-card !p-0" aria-label={t("remote.title")} data-remote-hosts>
      <div role="tablist" aria-orientation="vertical" aria-label={t("remote.title")} className="inject-rail">
        {list.map((host, index) => {
          const current = host.id === selected.id;
          const state = hostState(host);
          return (
            <button key={host.id} ref={node => { tabs.current[host.id] = node; }} type="button" role="tab"
              aria-selected={current} tabIndex={current ? 0 : -1} data-remote-host={host.id}
              onClick={() => select(host.id)} onKeyDown={event => move(event, index)} className="inject-tab group">
              <span className="inject-tab-glyph"><Server aria-hidden className="h-[17px] w-[17px]" /></span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-display text-[14px] font-semibold tracking-tight text-foreground">{host.name}</span>
                <span className="block truncate font-mono text-[11px] text-muted-foreground">{target(host)}</span>
              </span>
              <span aria-label={t(`remote.state.${state}`)} title={t(`remote.state.${state}`)} data-remote-host-dot={state}
                className={cn("h-2 w-2 shrink-0 rounded-full", stateTone[state])} />
            </button>
          );
        })}
        <button type="button" onClick={() => setEditing("new")} data-remote-add
          className="inject-tab !min-w-0 justify-center gap-1.5 text-xs font-medium text-muted-foreground hover:text-foreground lg:justify-start">
          <Plus aria-hidden className="h-4 w-4" />{t("remote.add")}
        </button>
      </div>
      <div role="tabpanel" className="inject-panel min-w-0">
        <RemoteHostDetail key={selected.id} host={selected} providers={providers} onEdit={() => setEditing(selected)} />
      </div>
      {dialog}
    </section>
  );
}

function RemoteHostDetail({ host, providers, onEdit }: { host: RemoteHost; providers: NewApiProvider[]; onEdit: () => void }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const clients = useQuery({
    queryKey: queryKeys.remoteClients(host.id),
    queryFn: () => api.remote.clients(host.id),
    retry: false,
    staleTime: 60_000,
    refetchOnWindowFocus: false,
  });
  const failure = clients.isError ? errorMessage(clients.error) : null;
  const [trusting, setTrusting] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const [dialog, setDialog] = useState<"codex" | RemoteManagedClient | null>(null);
  const [busy, setBusy] = useState(false);
  // A new host's key is fetched so the user can check it before trusting it.
  const hostKey = useQuery({
    queryKey: [...queryKeys.remoteClients(host.id), "host-key"],
    queryFn: () => api.remote.test(host.id),
    enabled: failure === "remote.errors.unknownHost" || failure === "remote.errors.hostKeyChanged",
    retry: false,
    staleTime: 0,
  });
  // The host list and this host's clients; the host-key check is left alone.
  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: queryKeys.remoteHosts, exact: true }),
      queryClient.invalidateQueries({ queryKey: queryKeys.remoteClients(host.id), exact: true }),
    ]);
  };

  const trust = async (fingerprint: string) => {
    setTrusting(true);
    try {
      const result = await api.remote.test(host.id, fingerprint);
      if (result.status !== "connected") throw new Error(result.status === "failed" ? result.error : "remote.errors.unknownHost");
      toast({ title: t("remote.trusted"), description: t("remote.connected", { home: result.home }), variant: "success" });
      await refresh();
    } catch (cause) {
      toast({ title: t("remote.trustFailed"), description: t(remoteErrorKey(cause)), variant: "destructive" });
    } finally {
      setTrusting(false);
    }
  };

  const act = (title: string, description: string, confirmLabel: string, work: () => Promise<unknown>, done: string, destructive = false) =>
    setConfirm({
      title, description, confirmLabel, destructive,
      onConfirm: () => {
        void (async () => {
          setBusy(true);
          try {
            await work();
            toast({ title: done, variant: "success" });
          } catch (cause) {
            toast({ title: t("remote.actionFailed"), description: t(remoteErrorKey(cause)), variant: "destructive" });
          } finally {
            setBusy(false);
            await refresh();
          }
        })();
      },
    });

  const remove = () => act(t("remote.deleteTitle", { name: host.name }), t("remote.deleteDescription"), t("remote.delete"),
    () => api.remote.remove(host.id), t("remote.deleted"), true);

  const data: RemoteClients | undefined = clients.data;
  const managed = data ? MANAGED.map(id => ({ id, status: data[id], version: data.installed[id] })) : [];
  const editing = managed.find(entry => entry.id === dialog);
  return (
    <div className="min-w-0 space-y-5" data-remote-host-detail>
      <ConsoleHeader titleId={`remote-host-title-${host.id}`} title={host.name}
        meta={<span className="flex min-w-0 flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span className="truncate font-mono">{target(host)}</span>
          <span className="kind-chip">
            {host.auth === "key" ? <KeyRound aria-hidden className="h-3 w-3" /> : <LockKeyhole aria-hidden className="h-3 w-3" />}
            {t(host.auth === "key" ? "remote.authKey" : "remote.authPassword")}
          </span>
          {data && <span className="truncate font-mono">~ {data.home}</span>}
        </span>}
        actions={<>
          <Button size="sm" variant="outline" className="gap-1.5" disabled={clients.isFetching} onClick={() => void refresh()}>
            <RefreshCw className={cn("h-3.5 w-3.5", clients.isFetching && "animate-spin")} />{t("remote.refresh")}
          </Button>
          <Button size="sm" variant="outline" className="gap-1.5" onClick={onEdit}><Pencil className="h-3.5 w-3.5" />{t("remote.edit")}</Button>
          <Button size="sm" variant="ghost" className="gap-1.5 text-destructive hover:text-destructive" disabled={host.inUse}
            title={host.inUse ? t("remote.errors.hostInUse") : undefined} onClick={remove}>
            <Trash2 className="h-3.5 w-3.5" />{t("remote.delete")}
          </Button>
        </>} />

      {host.routed && <TunnelStrip host={host} />}

      {clients.isLoading && <p className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />{t("remote.connecting")}</p>}

      {failure === "remote.errors.unknownHost" && (
        <HostKeyCard tone="info" icon={<Fingerprint className="h-5 w-5" />} title={t("remote.hostKey.unknownTitle")}
          description={t("remote.hostKey.unknownDescription")}
          fingerprint={hostKey.data?.status === "unknownHost" ? hostKey.data.fingerprint : null}
          algorithm={hostKey.data?.status === "unknownHost" ? hostKey.data.algorithm : null}
          loading={hostKey.isFetching}
          action={hostKey.data?.status === "unknownHost" && (
            <Button size="sm" className="gap-1.5" disabled={trusting} onClick={() => hostKey.data?.status === "unknownHost" && void trust(hostKey.data.fingerprint)}>
              {trusting && <Loader2 className="h-3.5 w-3.5 animate-spin" />}{t("remote.hostKey.trust")}
            </Button>
          )} />
      )}
      {failure === "remote.errors.hostKeyChanged" && (
        <HostKeyCard tone="danger" icon={<ShieldAlert className="h-5 w-5" />} title={t("remote.hostKey.changedTitle")}
          description={t("remote.hostKey.changedDescription", { host: host.resolved.hostName })}
          fingerprint={hostKey.data?.status === "hostKeyChanged" ? hostKey.data.fingerprint : null} algorithm={null}
          loading={hostKey.isFetching} />
      )}
      {failure && failure !== "remote.errors.unknownHost" && failure !== "remote.errors.hostKeyChanged" && (
        <p role="alert" className="flex items-start gap-2 rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2.5 text-sm text-destructive">
          <AlertTriangle aria-hidden className="mt-0.5 h-4 w-4 shrink-0" />{t(remoteErrorKey(clients.error))}
        </p>
      )}

      {data && (
        <div className="space-y-3" data-remote-clients>
          <p className="poster-kicker text-[10px] text-muted-foreground">Clients</p>
          {!!data.envMoved.length && <p role="status" data-remote-env-moved
            className="flex items-start gap-2 rounded-xl bg-amber-500/[0.07] px-3 py-2 text-xs text-amber-800 shadow-[inset_0_0_0_1px_rgb(245_158_11/0.3)] dark:text-amber-200">
            <AlertTriangle aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0" />{t("remote.envMoved", { names: data.envMoved.join(", ") })}
          </p>}
          <ClientRow glyph="codex" name="Codex" version={data.installed.codex} path={data.codex.path} configured={data.codex.configured}
            routed={!!data.codex.request?.router}
            provider={providers.find(entry => entry.id === data.codex.request?.providerId)?.name ?? data.codex.request?.providerId}
            model={data.codex.request ? data.codex.request.router ? routerLabel(data.codex.request.model, providers) : data.codex.request.model : undefined}
            conflictFields={data.codex.conflictFields} stale={data.codex.stale} error={data.codex.error}
            hint={data.codex.configured ? data.codex.request?.profile ? t("remote.clients.profileUse", { name: data.codex.request.profile }) : t("remote.clients.defaultUse") : undefined}
            busy={busy} onConfigure={() => setDialog("codex")} onProbe={() => api.remote.probe(host.id, "codex")}
            onRestore={() => act(t("remote.clients.restoreTitle", { name: "Codex" }),
              t(data.codex.conflict ? "remote.clients.restoreConflict" : "remote.clients.restoreDescription", { name: "Codex" }), t("remote.clients.restore"),
              () => api.remote.restoreCodex(host.id, data.codex.conflict), t("remote.clients.restored"))}
            onRelease={() => act(t("remote.clients.releaseTitle", { name: "Codex" }), t("remote.clients.releaseDescription"), t("remote.clients.release"),
              () => api.remote.releaseCodex(host.id), t("remote.clients.released"))} />
          {managed.map(({ id, status, version }) => (
            <ClientRow key={id} glyph={id} name={status.name} version={version} path={status.path}
              configured={status.configured} routed={status.request?.accessMode === "router"}
              provider={providers.find(entry => entry.id === status.providerId)?.name ?? status.providerId ?? undefined}
              model={status.model ?? undefined} conflictFields={status.conflictFields ?? []} stale={!!status.stale}
              error={status.error} busy={busy} onConfigure={() => setDialog(id)}
              onProbe={() => api.remote.probe(host.id, id)}
              onRestore={() => act(t("remote.clients.restoreTitle", { name: status.name }),
                t(status.conflict ? "remote.clients.restoreConflict" : "remote.clients.restoreDescription", { name: status.name }), t("remote.clients.restore"),
                () => api.remote.restoreClient(host.id, id, status.conflict), t("remote.clients.restored"))}
              onRelease={() => act(t("remote.clients.releaseTitle", { name: status.name }), t("remote.clients.releaseDescription"), t("remote.clients.release"),
                () => api.remote.releaseClient(host.id, id), t("remote.clients.released"))} />
          ))}
          <p className="poster-kicker pt-2 text-[10px] text-muted-foreground">Tools</p>
          <RemoteWebSearch key={`${data.webSearch.request?.providerId}:${data.webSearch.request?.model}`} hostId={host.id} status={data.webSearch} onChanged={refresh} />
        </div>
      )}

      {dialog === "codex" && data && <RemoteCodexDialog host={host} status={data.codex} providers={providers}
        onClose={() => setDialog(null)} onApplied={refresh} />}
      {editing && <ConfigDialog client={editing.status} providers={providers}
        onClose={() => setDialog(null)} onApplied={refresh}
        remote={{
          hostName: host.name,
          preview: (request, replace) => api.remote.previewClient(host.id, request, replace),
          apply: (request, revision, replace) => api.remote.applyClient(host.id, request, revision, replace),
        }} />}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}

/** A router model id, `<provider id>::<model>`, with the provider's name. */
const routerLabel = (model: string, providers: NewApiProvider[]) => {
  const [id, rest] = model.split("::");
  const provider = rest ? providers.find(entry => entry.id === id) : undefined;
  return provider ? `${provider.name} / ${rest}` : model;
};

/** How a routed host reaches this machine's router. */
function TunnelStrip({ host }: { host: RemoteHost }) {
  const { t } = useTranslation();
  const tunnel = host.tunnel;
  const state = tunnel?.state ?? "stopped";
  return (
    <div role="status" data-remote-tunnel={state}
      className={cn("flex flex-wrap items-center gap-x-3 gap-y-1 rounded-2xl px-4 py-2.5 text-xs",
        state === "connected" ? "bg-emerald-500/[0.07] shadow-[inset_0_0_0_1px_rgb(16_185_129/0.3)]"
          : state === "connecting" ? "bg-amber-500/[0.07] shadow-[inset_0_0_0_1px_rgb(245_158_11/0.3)]"
          : "bg-destructive/5 shadow-[inset_0_0_0_1px_hsl(var(--destructive)/0.3)]")}>
      <span className="flex items-center gap-1.5 font-semibold">
        {state === "connecting" ? <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" /> : <Cable aria-hidden className="h-3.5 w-3.5" />}
        {t(`remote.tunnel.${state}`)}
      </span>
      <span className="font-mono text-muted-foreground">{t("remote.tunnel.route", { port: host.tunnelPort })}</span>
      {tunnel?.state === "retrying" && <span className="text-destructive">{t(tunnel.error)}</span>}
      {state === "stopped" && <span className="text-muted-foreground">{t("remote.tunnel.stoppedHint")}</span>}
    </div>
  );
}

function HostKeyCard({ tone, icon, title, description, fingerprint, algorithm, loading, action }: {
  tone: "info" | "danger";
  icon: ReactNode;
  title: string;
  description: string;
  fingerprint: string | null;
  algorithm: string | null;
  loading: boolean;
  action?: ReactNode;
}) {
  const { t } = useTranslation();
  return (
    <div role="status" data-remote-host-key={tone}
      className={cn("flex flex-col gap-3 rounded-2xl px-4 py-3.5 sm:flex-row sm:items-center",
        tone === "danger" ? "bg-destructive/5 shadow-[inset_0_0_0_1px_hsl(var(--destructive)/0.35)]" : "bg-primary/[0.06] shadow-[inset_0_0_0_1px_hsl(var(--primary)/0.3)]")}>
      <span aria-hidden className={cn("shrink-0", tone === "danger" ? "text-destructive" : "text-primary")}>{icon}</span>
      <div className="min-w-0 flex-1 space-y-1">
        <p className="text-sm font-semibold">{title}</p>
        <p className="text-xs leading-relaxed text-muted-foreground">{description}</p>
        {loading && !fingerprint && <p className="flex items-center gap-1.5 text-xs text-muted-foreground"><Loader2 className="h-3 w-3 animate-spin" />{t("remote.connecting")}</p>}
        {fingerprint && <p className="break-all font-mono text-xs">
          <span className="text-muted-foreground">{t("remote.hostKey.fingerprint")}{algorithm ? ` (${algorithm})` : ""}: </span>{fingerprint}
        </p>}
      </div>
      {action && <div className="shrink-0">{action}</div>}
    </div>
  );
}

function ClientRow({ glyph, name, version, path, configured, routed, provider, model, hint, conflictFields, stale, error, busy, onConfigure, onProbe, onRestore, onRelease }: {
  glyph: "codex" | RemoteManagedClient;
  name: string;
  /** The version installed on the host; null when it is not found. */
  version: string | null;
  path: string;
  configured: boolean;
  /** Configured through this machine's router. */
  routed: boolean;
  provider?: string;
  model?: string;
  /** How to run the client as configured. */
  hint?: string;
  conflictFields: string[];
  stale: boolean;
  error: string | null;
  busy: boolean;
  onConfigure: () => void;
  onProbe: () => Promise<RemoteProbe>;
  onRestore: () => void;
  onRelease: () => void;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  const [probing, setProbing] = useState(false);
  const [probe, setProbe] = useState<RemoteProbe | { failed: string } | null>(null);
  useEffect(() => { if (!busy) setPending(false); }, [busy]);
  // A changed configuration makes an earlier answer stale.
  useEffect(() => { setProbe(null); }, [configured, routed, model]);
  const runProbe = async () => {
    setProbing(true);
    setProbe(null);
    try {
      setProbe(await onProbe());
    } catch (cause) {
      setProbe({ failed: remoteErrorKey(cause) });
    } finally {
      setProbing(false);
    }
  };
  const state = !configured ? "idle" : routed ? "routed" : "direct";
  return (
    <div className={cn("harbor-well relative space-y-2 px-4 py-3.5",
      configured && "before:absolute before:inset-y-3 before:left-0 before:w-[3px] before:rounded-full",
      state === "routed" ? "before:bg-primary" : state === "direct" && "before:bg-emerald-500")} data-remote-client={glyph}>
      <div className="flex flex-wrap items-center gap-3">
        <span className="inject-tab-glyph"><ClientGlyph client={glyph} className="h-[18px] w-[18px]" /></span>
        <div className="min-w-0 flex-1 basis-56">
          <p className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
            <span className="font-display text-[15px] font-semibold tracking-tight">{name}</span>
            {version
              ? <span className="font-mono text-[11px] text-muted-foreground" data-remote-client-version>{version}</span>
              : <span className="text-[11px] text-amber-700 dark:text-amber-300" data-remote-client-version>{t("remote.clients.notInstalled")}</span>}
          </p>
          <p className={cn("truncate text-xs", state === "routed" ? "text-primary" : state === "direct" ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground")}>
            {state === "idle" ? t("remote.clients.notConfigured")
              : state === "routed" ? t("remote.clients.routed", { model: model ?? "?" })
              : t("remote.clients.configured", { provider: provider ?? "?", model: model ?? "?" })}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {configured && <Button size="sm" variant="outline" className="gap-1.5" disabled={busy || probing || !version} onClick={() => void runProbe()}
            title={t("remote.clients.probeHint")}>
            {probing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Send className="h-3.5 w-3.5" />}{t("remote.clients.probe")}
          </Button>}
          <Button size="sm" className="gap-1.5" disabled={busy} onClick={onConfigure}>
            <Settings2 className="h-3.5 w-3.5" />{t(configured ? "remote.clients.reconfigure" : "remote.clients.configure")}
          </Button>
          {configured && <Button size="sm" variant="outline" className="gap-1.5" disabled={busy} onClick={() => { setPending(true); onRestore(); }}>
            {busy && pending ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RotateCcw className="h-3.5 w-3.5" />}{t("remote.clients.restore")}
          </Button>}
          {configured && (!!conflictFields.length || !!error) && <Button size="sm" variant="ghost" className="gap-1.5" disabled={busy} onClick={onRelease}>
            <Unlink className="h-3.5 w-3.5" />{t("remote.clients.release")}
          </Button>}
        </div>
      </div>
      <div className="space-y-1 sm:pl-[2.8rem]">
        <p className="truncate font-mono text-[11px] text-muted-foreground" title={path}>{path}</p>
        {hint && <p className="text-xs text-foreground/80" data-remote-client-hint>{hint}</p>}
        {!!conflictFields.length && <p role="status" className="text-xs text-amber-700 dark:text-amber-300">{t("remote.clients.conflict", { fields: conflictFields.join(", ") })}</p>}
        {stale && <p role="status" className="text-xs text-amber-700 dark:text-amber-300">{t("remote.clients.stale")}</p>}
        {error && <p role="alert" className="text-xs text-destructive">{t(error)}</p>}
        {probe && <ProbeResult probe={probe} />}
      </div>
    </div>
  );
}

function ProbeResult({ probe }: { probe: RemoteProbe | { failed: string } }) {
  const { t } = useTranslation();
  if ("failed" in probe) {
    return <p role="alert" className="flex items-center gap-1.5 text-xs text-destructive" data-remote-probe="failed"><X className="h-3.5 w-3.5" />{t(probe.failed)}</p>;
  }
  const seconds = (probe.elapsedMs / 1000).toFixed(1);
  return (
    <div role="status" data-remote-probe={probe.ok ? "ok" : "no-answer"} className="space-y-1 text-xs">
      <p className={cn("flex items-center gap-1.5 font-medium", probe.ok ? "text-emerald-600 dark:text-emerald-400" : "text-destructive")}>
        {probe.ok ? <Check className="h-3.5 w-3.5" /> : <X className="h-3.5 w-3.5" />}
        {t(probe.ok ? "remote.clients.probeOk" : "remote.clients.probeFailed", { seconds })}
      </p>
      {!probe.ok && probe.output && <pre className="max-h-40 overflow-auto rounded-lg bg-background/70 p-2.5 font-mono text-[11px] leading-relaxed text-muted-foreground">{probe.output}</pre>}
    </div>
  );
}
