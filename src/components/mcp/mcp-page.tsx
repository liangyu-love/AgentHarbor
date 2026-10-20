import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { AlertTriangle, Copy, Download, Info, MoreHorizontal, Plus, RefreshCcw, Server } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { PageHero } from "@/components/layout/page-hero";
import { AGENT_GLYPHS } from "@/components/newapi/configuration-shortcuts";
import { AgentChannels, type AgentChannel } from "@/components/ui/agent-channels";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { SearchField } from "@/components/ui/search-field";
import type { McpApp, McpCellState, McpKind, McpServerView } from "@/types";
import { McpEditor } from "./mcp-editor";
import { McpMatrix, copyText } from "./mcp-matrix";
import { MCP_APPS, mcpServersJson } from "./mcp-templates";
import { useMcpLibrary } from "./use-mcp-library";

type Channel = "all" | McpApp;
const TRANSPORTS: McpKind[] = ["stdio", "http", "sse"];
/** A cell whose agent holds the server but not as the library has it. */
const needsSync = (state: McpCellState) => state === "missing" || state === "differs";

/** Every agent's MCP servers in one list: a switch per server and agent. */
export function McpPage() {
  const { t } = useTranslation();
  const library = useMcpLibrary();
  const { view } = library;
  const [query, setQuery] = useState("");
  const [channel, setChannel] = useState<Channel>("all");
  const [hiddenTransports, setHiddenTransports] = useState<McpKind[]>([]);
  const [editing, setEditing] = useState<McpServerView | "new" | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const needle = query.trim().toLowerCase();

  // Search and channel first; the transport chips count what those leave.
  const found = useMemo(() => (view?.servers ?? []).filter(server =>
    (!needle || [server.name, server.note, server.spec.command, ...(server.spec.args ?? []), server.spec.url]
      .some(text => text?.toLowerCase().includes(needle)))
    && (channel === "all" || server.cells.some(cell => cell.app === channel && cell.on))),
  [view, needle, channel]);
  const servers = found.filter(server => !hiddenTransports.includes(server.spec.transport));

  if (library.error) {
    return (
      <div role="alert" className="space-y-3 p-4">
        <p>{t("baseline.readFailed")}</p>
        <p className="text-sm text-muted-foreground">{formatInvokeError(library.error)}</p>
        <Button onClick={() => void library.refetch()}>{t("common.refresh")}</Button>
      </div>
    );
  }
  if (!view) return <p role="status" className="p-4 text-muted-foreground">{t("baseline.loading")}</p>;

  const importAll = () => void library.run(api.importMcpServers,
    next => next.added.length > 0 ? t("mcp.imported", { count: next.added.length }) : t("mcp.nothingImported"));
  const notes = view.apps.filter(app => app.error || app.unmanaged.length > 0);
  const clearFilters = () => {
    setQuery("");
    setChannel("all");
    setHiddenTransports([]);
  };
  const toggleTransport = (kind: McpKind) =>
    setHiddenTransports(current => current.includes(kind) ? current.filter(item => item !== kind) : [...current, kind]);

  const remove = (server: McpServerView) => setConfirm({
    title: t("mcp.confirmDelete", { name: server.name }),
    description: t("mcp.confirmDeleteDesc"),
    confirmLabel: t("mcp.delete"),
    destructive: true,
    onConfirm: () => {
      setConfirm(null);
      void library.run(() => api.removeMcpServer(server.name));
    },
  });

  const channels: AgentChannel<Channel>[] = [
    { id: "all", label: t("mcp.channels.all"), count: view.servers.length },
    ...MCP_APPS.map(app => {
      const target = view.apps.find(item => item.app === app);
      const pending = view.servers.filter(server => server.cells.some(cell => cell.app === app && cell.on && needsSync(cell.state))).length;
      const note = !target?.installed ? t("mcp.column.notInstalled")
        : target.error ? t("mcp.channels.unreadable")
          : pending > 0 ? t("mcp.channels.pending", { count: pending })
            : target.unmanaged.length > 0 ? t("mcp.channels.unmanaged", { count: target.unmanaged.length }) : undefined;
      return {
        id: app, label: t(`mcp.apps.${app}`), glyph: AGENT_GLYPHS[app], note, muted: !target?.installed,
        count: view.servers.filter(server => server.cells.some(cell => cell.app === app && cell.on)).length,
        title: target?.installed ? target.path : t("mcp.column.notInstalled"),
      };
    }),
  ];

  return (
    <div className="space-y-5" data-mcp-page>
      <PageHero kicker="MCP Servers" icon={<Server />} title={t("mcp.title")} description={t("mcp.description")} actions={<>
        <Button size="sm" className="h-9 gap-1.5 rounded-lg" disabled={library.busy} onClick={() => setEditing("new")}>
          <Plus className="h-4 w-4" /> {t("mcp.addServer")}
        </Button>
        <Button variant="outline" size="sm" className="h-9 gap-1.5" disabled={library.busy} onClick={importAll}>
          <Download className="h-4 w-4" /> {t("mcp.import")}
        </Button>
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="icon" className="h-9 w-9" aria-label={t("mcp.more")} title={t("mcp.more")} disabled={library.busy}>
              <MoreHorizontal className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onSelect={() => void library.run(api.resyncMcpServers)}>
              <RefreshCcw className="h-3.5 w-3.5" /> {t("mcp.resync")}
            </DropdownMenuItem>
            <DropdownMenuItem disabled={view.servers.length === 0}
              onSelect={() => void copyText(mcpServersJson(view.servers), t("mcp.copied"))}>
              <Copy className="h-3.5 w-3.5" /> {t("mcp.copyAll")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </>}>
        <AgentChannels label={t("mcp.channels.label")} channels={channels} value={channel} onChange={setChannel} />
      </PageHero>

      {notes.length > 0 && (
        <div className="harbor-card route-rise space-y-1.5 !py-3 text-xs text-muted-foreground" data-mcp-notes>
          {notes.map(app => (
            <p key={app.app} className={app.error ? "flex items-start gap-2 text-destructive" : "flex items-start gap-2"}>
              {app.error ? <AlertTriangle className="mt-px h-3.5 w-3.5 shrink-0" /> : <Info className="mt-px h-3.5 w-3.5 shrink-0 text-primary" />}
              <span>
                {app.error
                  ? `${t(`mcp.apps.${app.app}`)}：${formatInvokeError(app.error)}`
                  : t("mcp.unmanaged", { app: t(`mcp.apps.${app.app}`), count: app.unmanaged.length, names: app.unmanaged.join("、") })}
              </span>
            </p>
          ))}
        </div>
      )}

      {view.servers.length === 0 ? (
        <section className="harbor-card route-rise">
          <div className="flex h-56 flex-col items-center justify-center p-6 text-center">
            <span className="flex h-12 w-12 items-center justify-center rounded-2xl bg-foreground text-background">
              <Server className="h-5 w-5" />
            </span>
            <p className="mt-3.5 text-base font-semibold text-foreground">{t("mcp.empty")}</p>
            <p className="mt-1 max-w-sm text-xs text-muted-foreground">{t("mcp.emptyHint")}</p>
            <div className="mt-4 flex gap-2">
              <Button size="sm" className="gap-1.5 rounded-lg" onClick={() => setEditing("new")}>
                <Plus className="h-3.5 w-3.5" /> {t("mcp.addServer")}
              </Button>
            </div>
          </div>
        </section>
      ) : (
        <>
          <div className="route-rise flex flex-col gap-3 lg:flex-row lg:items-center lg:justify-between">
            <SearchField value={query} onChange={setQuery} placeholder={t("mcp.searchPlaceholder")}
              clearLabel={t("mcp.clearSearch")} className="lg:max-w-md" />
            <div className="flex flex-wrap items-center gap-1.5" role="group" aria-label={t("mcp.transports")} data-mcp-transports>
              <span className="mr-1 text-[11px] text-muted-foreground">{t("mcp.transports")}</span>
              {TRANSPORTS.map(kind => {
                const off = hiddenTransports.includes(kind);
                return (
                  <button key={kind} type="button" className="kind-chip font-mono uppercase" aria-pressed={!off} data-mcp-transport={kind}
                    title={off ? t("mcp.showTransport") : t("mcp.hideTransport")} onClick={() => toggleTransport(kind)}>
                    {kind}<span className="tabular-nums opacity-70">{found.filter(server => server.spec.transport === kind).length}</span>
                  </button>
                );
              })}
            </div>
          </div>
          <section className="harbor-card route-rise overflow-hidden !p-0">
            <div className="flex items-center justify-between gap-3 border-b border-border/60 px-4 py-3 sm:px-5">
              <p className="poster-kicker text-[10px] text-primary">Servers</p>
              <span className="text-xs text-muted-foreground" data-mcp-shown>
                {servers.length === view.servers.length
                  ? t("mcp.count", { count: view.servers.length })
                  : t("mcp.shown", { shown: servers.length, count: view.servers.length })}
              </span>
            </div>
            {servers.length === 0 ? (
              <div className="flex h-40 flex-col items-center justify-center text-center">
                <p className="text-sm text-muted-foreground">{t("mcp.noFilterResults")}</p>
                <Button variant="outline" size="sm" className="mt-3" onClick={clearFilters}>{t("mcp.clearFilters")}</Button>
              </div>
            ) : (
              <McpMatrix view={view} servers={servers} library={library} onEdit={setEditing} onDelete={remove} />
            )}
          </section>
        </>
      )}

      {editing !== null && (
        <McpEditor key={editing === "new" ? "__new__" : editing.name} server={editing === "new" ? null : editing}
          view={view} library={library} onClose={() => setEditing(null)} />
      )}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}
