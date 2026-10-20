import { useTranslation } from "react-i18next";
import { AlertTriangle, Check, CircleDot, Copy, Minus, MoreHorizontal, Pencil, Power, PowerOff, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { McpApp, McpCell, McpLibraryView, McpServerView } from "@/types";
import { MCP_APPS, mcpServersJson } from "./mcp-templates";
import type { McpLibrary } from "./use-mcp-library";

/** A switch the cell cannot take: the agent is missing, cannot hold it, or cannot be read. */
const locked = (cell: McpCell) => cell.state === "notInstalled" || cell.state === "unsupported" || cell.state === "unreadable";

function CellMark({ cell }: { cell: McpCell }) {
  if (locked(cell)) return <Minus className="h-3.5 w-3.5 text-muted-foreground/50" />;
  if (cell.state === "synced") return <Check className="h-3.5 w-3.5" strokeWidth={3} />;
  if (cell.state === "missing" || cell.state === "differs") return <AlertTriangle className="h-3.5 w-3.5" />;
  if (cell.state === "outside") return <CircleDot className="h-3.5 w-3.5 text-muted-foreground" />;
  return <span className="h-2.5 w-2.5 rounded-full border-[1.5px] border-muted-foreground/45" />;
}

export const copyText = async (text: string, done: string) => {
  try {
    await navigator.clipboard.writeText(text);
    toast({ title: done, variant: "success" });
  } catch {
    // Clipboard unavailable: nothing was copied.
  }
};

/** Rows are servers, columns are agents; each cell turns the server on or off for that agent. */
export function McpMatrix({ view, servers, library, onEdit, onDelete }: {
  view: McpLibraryView;
  /** The servers shown, after search. */
  servers: McpServerView[];
  library: McpLibrary;
  onEdit: (server: McpServerView) => void;
  onDelete: (server: McpServerView) => void;
}) {
  const { t } = useTranslation();
  const names = servers.map(server => server.name);
  const set = (list: string[], app: McpApp | null, on: boolean) => void library.run(() => api.setMcpServers(list, app, on));
  const appView = (app: McpApp) => view.apps.find(item => item.app === app);

  return (
    <div className="overflow-x-auto" data-mcp-matrix>
      <table className="w-full min-w-[760px] border-separate border-spacing-0 text-sm">
        <thead>
          <tr>
            <th scope="col" className="sticky left-0 z-10 bg-card px-3 py-3 text-left text-xs font-medium text-muted-foreground sm:px-4">
              {t("mcp.server")}
            </th>
            {MCP_APPS.map(app => {
              const target = appView(app);
              return (
                <th key={app} scope="col" className="px-1 py-2 text-center align-bottom">
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <button type="button" data-mcp-column={app} disabled={library.busy || !target?.installed}
                        title={target?.installed ? target.path : t("mcp.column.notInstalled")}
                        className={cn(
                          "inline-flex max-w-[6.5rem] flex-col items-center rounded-md px-1.5 py-1 text-xs font-medium leading-tight transition-colors hover:bg-muted disabled:hover:bg-transparent",
                          target?.installed ? "text-foreground" : "text-muted-foreground/60",
                          target?.error && "text-destructive",
                        )}>
                        {t(`mcp.apps.${app}`)}
                        {!target?.installed && <span className="text-[10px] font-normal">{t("mcp.column.notInstalled")}</span>}
                      </button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="center" className="w-56">
                      <DropdownMenuLabel className="break-all text-[11px] font-normal text-muted-foreground">{target?.path}</DropdownMenuLabel>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem onSelect={() => set(names, app, true)}><Power className="h-3.5 w-3.5" />{t("mcp.column.enableAll")}</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => set(names, app, false)}><PowerOff className="h-3.5 w-3.5" />{t("mcp.column.disableAll")}</DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </th>
              );
            })}
            <th scope="col" className="w-20 px-2" aria-label={t("mcp.more")} />
          </tr>
        </thead>
        <tbody>
          {servers.map(server => (
            <tr key={server.name} data-mcp-server={server.name} className="group">
              <th scope="row" className="sticky left-0 z-10 w-36 max-w-36 border-t border-border/50 bg-card px-3 py-2.5 text-left font-normal shadow-[1px_0_0_hsl(var(--border)/0.5)] sm:w-auto sm:max-w-[18rem] sm:px-4">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate font-mono text-[13px] font-semibold text-foreground">{server.name}</span>
                  <span className="shrink-0 rounded bg-muted px-1.5 py-px text-[10px] uppercase text-muted-foreground">{server.spec.transport}</span>
                </div>
                <p className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground" title={server.spec.transport === "stdio"
                  ? [server.spec.command, ...(server.spec.args ?? [])].join(" ") : server.spec.url}>
                  {server.spec.transport === "stdio" ? [server.spec.command, ...(server.spec.args ?? [])].join(" ") : server.spec.url}
                </p>
                {server.note && <p className="mt-0.5 truncate text-[11px] text-muted-foreground/80">{server.note}</p>}
              </th>
              {server.cells.map(cell => {
                const app = t(`mcp.apps.${cell.app}`);
                const state = cell.reason ? t(cell.reason) : t(`mcp.state.${cell.state}`);
                return (
                  <td key={cell.app} className="border-t border-border/50 px-1 py-2 text-center">
                    <button type="button" role="switch" aria-checked={cell.on} data-mcp-cell={cell.app} data-state={cell.state}
                      aria-label={t("mcp.toggle", { server: server.name, app })} title={`${app} · ${state}`}
                      disabled={library.busy || locked(cell)}
                      onClick={() => set([server.name], cell.app, !cell.on)}
                      className={cn(
                        "inline-flex h-8 w-8 items-center justify-center rounded-lg ring-1 ring-inset transition-colors disabled:cursor-not-allowed",
                        locked(cell) ? "bg-transparent ring-transparent"
                          : cell.on && cell.state === "synced" ? "bg-primary text-primary-foreground ring-primary/30 hover:bg-primary/90"
                            : cell.on ? "bg-amber-500/15 text-amber-700 ring-amber-500/40 hover:bg-amber-500/25 dark:text-amber-300"
                              : "bg-background ring-border hover:bg-muted",
                      )}>
                      <CellMark cell={cell} />
                    </button>
                  </td>
                );
              })}
              <td className="border-t border-border/50 px-2 py-2 text-right">
                <div className="flex items-center justify-end gap-0.5">
                  <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={t("mcp.row.edit")} title={t("mcp.row.edit")}
                    disabled={library.busy} onClick={() => onEdit(server)}>
                    <Pencil className="h-3.5 w-3.5" />
                  </Button>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="icon" className="h-8 w-8" aria-label={t("mcp.more")} disabled={library.busy}>
                        <MoreHorizontal className="h-4 w-4" />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end">
                      <DropdownMenuItem onSelect={() => void copyText(mcpServersJson([server]), t("mcp.copied"))}>
                        <Copy className="h-3.5 w-3.5" />{t("mcp.row.copyJson")}
                      </DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => set([server.name], null, true)}><Power className="h-3.5 w-3.5" />{t("mcp.row.enableAll")}</DropdownMenuItem>
                      <DropdownMenuItem onSelect={() => set([server.name], null, false)}><PowerOff className="h-3.5 w-3.5" />{t("mcp.row.disableAll")}</DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem className="text-destructive focus:text-destructive" onSelect={() => onDelete(server)}>
                        <Trash2 className="h-3.5 w-3.5" />{t("mcp.row.delete")}
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </div>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
