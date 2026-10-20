import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Download, MoreHorizontal, Plus, RefreshCcw, Search, Server, X } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { PageHero } from "@/components/layout/page-hero";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import type { McpServerView } from "@/types";
import { McpEditor } from "./mcp-editor";
import { McpMatrix, copyText } from "./mcp-matrix";
import { mcpServersJson } from "./mcp-templates";
import { useMcpLibrary } from "./use-mcp-library";

/** Every agent's MCP servers in one list: a switch per server and agent. */
export function McpPage() {
  const { t } = useTranslation();
  const library = useMcpLibrary();
  const { view } = library;
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<McpServerView | "new" | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

  const servers = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!view || !needle) return view?.servers ?? [];
    return view.servers.filter(server => [server.name, server.note, server.spec.command, ...(server.spec.args ?? []), server.spec.url]
      .some(text => text?.toLowerCase().includes(needle)));
  }, [view, query]);

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

  return (
    <div className="space-y-6" data-mcp-page>
      <PageHero kicker="MCP Servers" icon={<Server />} title={t("mcp.title")} description={t("mcp.description")} actions={<>
        <RainbowButton size="sm" className="h-9 rounded-lg font-medium" disabled={library.busy} onClick={() => setEditing("new")}>
          <Plus className="h-4 w-4" /> {t("mcp.addServer")}
        </RainbowButton>
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
      </>} />

      {notes.length > 0 && (
        <div className="space-y-1 rounded-xl border border-border/60 bg-muted/30 px-4 py-2.5 text-xs text-muted-foreground" data-mcp-notes>
          {notes.map(app => (
            <p key={app.app} className={app.error ? "text-destructive" : undefined}>
              {app.error
                ? `${t(`mcp.apps.${app.app}`)}：${formatInvokeError(app.error)}`
                : t("mcp.unmanaged", { app: t(`mcp.apps.${app.app}`), count: app.unmanaged.length, names: app.unmanaged.join("、") })}
            </p>
          ))}
        </div>
      )}

      {view.servers.length === 0 ? (
        <BentoCard>
          <div className="flex h-56 flex-col items-center justify-center p-6 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-2xl bg-gradient-to-br from-primary/20 to-primary/5 ring-1 ring-inset ring-primary/15">
              <Server className="h-6 w-6 text-primary" />
            </div>
            <p className="mt-3.5 text-base font-semibold text-foreground">{t("mcp.empty")}</p>
            <p className="mt-1 max-w-sm text-xs text-muted-foreground">{t("mcp.emptyHint")}</p>
            <div className="mt-4 flex gap-2">
              <RainbowButton size="sm" className="rounded-lg" onClick={() => setEditing("new")}>
                <Plus className="h-3.5 w-3.5" /> {t("mcp.addServer")}
              </RainbowButton>
              <Button variant="outline" size="sm" disabled={library.busy} onClick={importAll}>{t("mcp.import")}</Button>
            </div>
          </div>
        </BentoCard>
      ) : (
        <BentoCard className="p-0">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border/50 px-4 py-3">
            <div className="relative w-full max-w-xs">
              <Search className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
              <Input value={query} onChange={event => setQuery(event.target.value)} placeholder={t("mcp.searchPlaceholder")}
                aria-label={t("mcp.searchPlaceholder")} className="h-8 pl-8 pr-8 text-xs" />
              {query && (
                <button type="button" aria-label={t("mcp.clearSearch")} onClick={() => setQuery("")}
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground">
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            <span className="text-xs text-muted-foreground">{t("mcp.count", { count: view.servers.length })}</span>
          </div>
          {servers.length === 0 ? (
            <div className="flex h-40 flex-col items-center justify-center text-center">
              <p className="text-sm text-muted-foreground">{t("mcp.noFilterResults")}</p>
              <Button variant="outline" size="sm" className="mt-3" onClick={() => setQuery("")}>{t("mcp.clearSearch")}</Button>
            </div>
          ) : (
            <McpMatrix view={view} servers={servers} library={library} onEdit={setEditing} onDelete={remove} />
          )}
        </BentoCard>
      )}

      {editing !== null && (
        <McpEditor key={editing === "new" ? "__new__" : editing.name} server={editing === "new" ? null : editing}
          view={view} library={library} onClose={() => setEditing(null)} />
      )}
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}
