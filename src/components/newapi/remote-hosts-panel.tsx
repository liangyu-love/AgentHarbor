import { useEffect, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Fingerprint, KeyRound, Loader2, LockKeyhole, Pencil, Plus, RefreshCw, RotateCcw, Server, Settings2, ShieldAlert, Trash2, Unlink } from "lucide-react";
import { api } from "@/lib/api";
import { errorMessage } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import type { ClientStatus } from "@/types/clients";
import type { NewApiProvider } from "@/types/newapi";
import type { RemoteClients, RemoteHost } from "@/types/remote";
import { ConfigDialog } from "./client-config-dialog";
import { ClientGlyph } from "./configuration-shortcuts";
import { RemoteCodexDialog } from "./remote-codex-dialog";
import { RemoteHostDialog, remoteErrorKey } from "./remote-host-dialog";

const SELECTED_KEY = "agentharbor.remote-host.v1";

const target = (host: RemoteHost) => {
  const user = host.user ?? host.resolved.user;
  return `${user ? user + "@" : ""}${host.resolved.hostName}${host.resolved.port === 22 ? "" : ":" + host.resolved.port}`;
};

// Remote hosts on the injection page: the hosts in a rail, the chosen one's
// Codex and Claude Code beside it. Every remote client is configured direct.
export function RemoteHostsPanel({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const hosts = useQuery({ queryKey: queryKeys.remoteHosts, queryFn: api.remote.hosts });
  const [selectedId, setSelectedId] = useState<string | null>(() => {
    try { return localStorage.getItem(SELECTED_KEY); } catch { return null; }
  });
  const [editing, setEditing] = useState<RemoteHost | "new" | null>(null);
  const list = hosts.data ?? [];
  const selected = list.find(host => host.id === selectedId) ?? list[0];
  const select = (id: string) => {
    setSelectedId(id);
    try { localStorage.setItem(SELECTED_KEY, id); } catch { /* the choice lasts for this visit */ }
  };

  return (
    <section aria-labelledby="remote-hosts-title" data-remote-hosts className="space-y-4 rounded-2xl border border-border/70 bg-card/80 p-4 shadow-sm sm:p-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
        <div className="min-w-0 space-y-1.5">
          <h2 id="remote-hosts-title" className="flex items-center gap-2 font-display text-xl font-semibold tracking-tight">
            <Server aria-hidden className="h-5 w-5 text-primary" />{t("remote.title")}
          </h2>
          <p className="max-w-3xl text-xs text-muted-foreground">{t("remote.description")}</p>
        </div>
        <Button size="sm" className="shrink-0 gap-1.5" onClick={() => setEditing("new")}><Plus className="h-4 w-4" />{t("remote.add")}</Button>
      </div>

      {hosts.isError && <p role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">{t(remoteErrorKey(hosts.error))}</p>}

      {hosts.isSuccess && list.length === 0 && (
        <div className="harbor-well flex flex-col items-center gap-3 px-4 py-10 text-center" data-remote-empty>
          <Server aria-hidden className="h-8 w-8 text-muted-foreground/70" />
          <p className="max-w-md text-sm text-muted-foreground">{t("remote.empty")}</p>
          <Button size="sm" variant="outline" className="gap-1.5" onClick={() => setEditing("new")}><Plus className="h-4 w-4" />{t("remote.add")}</Button>
        </div>
      )}

      {selected && (
        <div className="grid gap-4 md:grid-cols-[15rem_minmax(0,1fr)]">
          <div role="tablist" aria-orientation="vertical" aria-label={t("remote.title")} className="flex min-w-0 gap-2 overflow-x-auto md:flex-col md:overflow-visible">
            {list.map(host => {
              const active = host.id === selected.id;
              return (
                <button key={host.id} type="button" role="tab" aria-selected={active} data-remote-host={host.id}
                  onClick={() => select(host.id)}
                  className={cn("min-w-[11rem] rounded-2xl px-3.5 py-2.5 text-left transition-colors md:min-w-0",
                    active ? "bg-primary/10 shadow-[inset_0_0_0_1px_hsl(var(--primary)/0.45)]" : "hover:bg-muted/60")}>
                  <span className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate text-sm font-semibold">{host.name}</span>
                    {host.inUse && <span aria-label={t("remote.inUse")} title={t("remote.inUse")} className="h-2 w-2 shrink-0 rounded-full bg-emerald-500" />}
                  </span>
                  <span className="mt-0.5 block truncate font-mono text-[11px] text-muted-foreground">{target(host)}</span>
                </button>
              );
            })}
          </div>
          <RemoteHostDetail key={selected.id} host={selected} providers={providers} onEdit={() => setEditing(selected)} />
        </div>
      )}

      {editing && <RemoteHostDialog host={editing === "new" ? null : editing} onClose={() => setEditing(null)}
        onSaved={async saved => {
          setEditing(null);
          select(saved.id);
          await queryClient.invalidateQueries({ queryKey: queryKeys.remoteHosts });
        }} />}
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
  const [dialog, setDialog] = useState<"codex" | "claudeCode" | null>(null);
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
  const claudeName = data?.claudeCode.name ?? "Claude Code";
  return (
    <div className="min-w-0 space-y-4" role="tabpanel" data-remote-host-detail>
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 space-y-1">
          <p className="font-display text-lg font-semibold tracking-tight">{host.name}</p>
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
            <span className="font-mono">{target(host)}</span>
            <span className="kind-chip">
              {host.auth === "key" ? <KeyRound aria-hidden className="h-3 w-3" /> : <LockKeyhole aria-hidden className="h-3 w-3" />}
              {t(host.auth === "key" ? "remote.authKey" : "remote.authPassword")}
            </span>
            {data && <span className="font-mono">~ {data.home}</span>}
          </p>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          <Button size="sm" variant="outline" className="gap-1.5" disabled={clients.isFetching} onClick={() => void refresh()}>
            <RefreshCw className={cn("h-3.5 w-3.5", clients.isFetching && "animate-spin")} />{t("remote.refresh")}
          </Button>
          <Button size="sm" variant="outline" className="gap-1.5" onClick={onEdit}><Pencil className="h-3.5 w-3.5" />{t("remote.edit")}</Button>
          <Button size="sm" variant="ghost" className="gap-1.5 text-destructive hover:text-destructive" disabled={host.inUse}
            title={host.inUse ? t("remote.errors.hostInUse") : undefined} onClick={remove}>
            <Trash2 className="h-3.5 w-3.5" />{t("remote.delete")}
          </Button>
        </div>
      </div>

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
        <div className="space-y-2.5" data-remote-clients>
          <ClientRow glyph="codex" name="Codex" path={data.codex.path} configured={data.codex.configured}
            provider={providers.find(entry => entry.id === data.codex.request?.providerId)?.name ?? data.codex.request?.providerId}
            model={data.codex.request?.model} conflictFields={data.codex.conflictFields} stale={data.codex.stale} error={data.codex.error}
            hint={data.codex.configured ? data.codex.request?.profile ? t("remote.clients.profileUse", { name: data.codex.request.profile }) : t("remote.clients.defaultUse") : undefined}
            busy={busy} onConfigure={() => setDialog("codex")}
            onRestore={() => act(t("remote.clients.restoreTitle", { name: "Codex" }),
              t(data.codex.conflict ? "remote.clients.restoreConflict" : "remote.clients.restoreDescription", { name: "Codex" }), t("remote.clients.restore"),
              () => api.remote.restoreCodex(host.id, data.codex.conflict), t("remote.clients.restored"))}
            onRelease={() => act(t("remote.clients.releaseTitle", { name: "Codex" }), t("remote.clients.releaseDescription"), t("remote.clients.release"),
              () => api.remote.releaseCodex(host.id), t("remote.clients.released"))} />
          <ClientRow glyph="claudeCode" name={claudeName} path={data.claudeCode.path} configured={data.claudeCode.configured}
            provider={providers.find(entry => entry.id === data.claudeCode.providerId)?.name ?? data.claudeCode.providerId ?? undefined}
            model={data.claudeCode.model ?? undefined} conflictFields={data.claudeCode.conflictFields ?? []} stale={!!data.claudeCode.stale}
            error={data.claudeCode.error} busy={busy} onConfigure={() => setDialog("claudeCode")}
            onRestore={() => act(t("remote.clients.restoreTitle", { name: claudeName }),
              t(data.claudeCode.conflict ? "remote.clients.restoreConflict" : "remote.clients.restoreDescription", { name: claudeName }), t("remote.clients.restore"),
              () => api.remote.restoreClaude(host.id, data.claudeCode.conflict), t("remote.clients.restored"))}
            onRelease={() => act(t("remote.clients.releaseTitle", { name: claudeName }), t("remote.clients.releaseDescription"), t("remote.clients.release"),
              () => api.remote.releaseClaude(host.id), t("remote.clients.released"))} />
        </div>
      )}

      {dialog === "codex" && data && <RemoteCodexDialog host={host} status={data.codex} providers={providers}
        onClose={() => setDialog(null)} onApplied={refresh} />}
      {dialog === "claudeCode" && data && <ConfigDialog client={remoteClaudeStatus(data.claudeCode)} providers={providers}
        onClose={() => setDialog(null)} onApplied={refresh}
        remote={{
          hostName: host.name,
          preview: (request, replace) => api.remote.previewClaude(host.id, request, replace),
          apply: (request, revision, replace) => api.remote.applyClaude(host.id, request, revision, replace),
        }} />}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}

/** A remote Claude Code is configured direct: a saved router request is not offered again. */
const remoteClaudeStatus = (status: ClientStatus): ClientStatus =>
  status.request?.accessMode === "router" ? { ...status, request: null } : status;

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

function ClientRow({ glyph, name, path, configured, provider, model, hint, conflictFields, stale, error, busy, onConfigure, onRestore, onRelease }: {
  glyph: "codex" | "claudeCode";
  name: string;
  path: string;
  configured: boolean;
  provider?: string;
  model?: string;
  /** How to run the client as configured. */
  hint?: string;
  conflictFields: string[];
  stale: boolean;
  error: string | null;
  busy: boolean;
  onConfigure: () => void;
  onRestore: () => void;
  onRelease: () => void;
}) {
  const { t } = useTranslation();
  const [pending, setPending] = useState(false);
  useEffect(() => { if (!busy) setPending(false); }, [busy]);
  return (
    <div className={cn("harbor-card flex flex-wrap items-center gap-3 !p-3.5", configured && "harbor-card-live")} data-remote-client={glyph}>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-muted/70">
        <ClientGlyph client={glyph} className="h-[18px] w-[18px]" />
      </span>
      <div className="min-w-0 flex-1 basis-60 space-y-0.5">
        <p className="flex items-center gap-2 text-sm font-semibold">
          {name}
          <span className={cn("text-[11px] font-normal", configured ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground")}>
            {configured ? t("remote.clients.configured", { provider: provider ?? "?", model: model ?? "?" }) : t("remote.clients.notConfigured")}
          </span>
        </p>
        <p className="truncate font-mono text-[11px] text-muted-foreground" title={path}>{path}</p>
        {hint && <p className="text-xs text-foreground/80" data-remote-client-hint>{hint}</p>}
        {!!conflictFields.length && <p role="status" className="text-xs text-amber-700 dark:text-amber-300">{t("remote.clients.conflict", { fields: conflictFields.join(", ") })}</p>}
        {stale && <p role="status" className="text-xs text-amber-700 dark:text-amber-300">{t("remote.clients.stale")}</p>}
        {error && <p role="alert" className="text-xs text-destructive">{t(error)}</p>}
      </div>
      <div className="flex shrink-0 flex-wrap gap-2">
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
  );
}
