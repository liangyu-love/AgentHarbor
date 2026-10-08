import { useTranslation } from "react-i18next";
import { Check, Copy, Globe, Key, Layers, Pencil, Radio, Terminal, Trash2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { BentoCard } from "@/components/ui/bento-card";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import type { McpServerSummary } from "@/types";
import type { McpServers } from "@/components/mcp/use-mcp-servers";

const transportStyles: Record<string, { dot: string; text: string }> = {
  stdio: { dot: "bg-blue-500 shadow-[0_0_0_2px_rgba(59,130,246,0.2)]", text: "text-blue-500" },
  http: { dot: "bg-violet-500 shadow-[0_0_0_2px_rgba(139,92,246,0.2)]", text: "text-violet-500" },
  sse: { dot: "bg-amber-500 shadow-[0_0_0_2px_rgba(245,158,11,0.2)]", text: "text-amber-500" },
};

const transportTile: Record<string, string> = {
  stdio: "bg-blue-500/10 ring-blue-500/15 text-blue-500 dark:text-blue-400",
  http: "bg-violet-500/10 ring-violet-500/15 text-violet-500 dark:text-violet-400",
  sse: "bg-amber-500/10 ring-amber-500/15 text-amber-500 dark:text-amber-400",
};

function DotBadge({ dotClass, textClass, children }: { dotClass: string; textClass: string; children: React.ReactNode }) {
  return (
    <Badge variant="outline" className={cn("gap-1.5 pl-2 pr-2.5 py-0.5 text-[11px] font-medium", textClass)}>
      <span className={cn("inline-block h-1.5 w-1.5 rounded-full shrink-0", dotClass)} />
      {children}
    </Badge>
  );
}

export function McpServerCard({ server, mcp }: { server: McpServerSummary; mcp: McpServers }) {
  const { t } = useTranslation();
  const { copiedCommand, toggleMutation, handleCopyCommand, setEditing, setRemoving, mutationBusy } = mcp;
  const isStdio = server.transport === "stdio";
  const cmdString = isStdio
    ? [server.command, ...(server.args || [])].filter(Boolean).join(" ")
    : server.url ?? "";
  const envKeys = Object.keys(server.environment || {});
  const headerKeys = Object.keys(server.headers || {});
  const hasEnv = envKeys.length > 0;
  const hasHeaders = headerKeys.length > 0;
  const isCopied = copiedCommand === server.name;

  const transportIcon = isStdio ? (
    <Terminal className="h-4 w-4" />
  ) : server.transport === "http" ? (
    <Globe className="h-4 w-4" />
  ) : (
    <Radio className="h-4 w-4" />
  );


  return (
    <BentoCard
      compact
      className={cn(
        "group flex flex-col justify-between transition-all duration-200 hover:border-primary/40 hover:shadow-md",
        !server.enabled && "opacity-80 dark:opacity-75 bg-card/60"
      )}
    >
      <div>
        {/* Card Header: Icon + Name + Switch */}
        <div className="flex items-center justify-between gap-2.5">
          <div className="flex items-center gap-2.5 min-w-0 flex-1">
            <div
              className={cn(
                "flex h-9 w-9 shrink-0 items-center justify-center rounded-xl ring-1 ring-inset transition-transform group-hover:scale-105",
                transportTile[server.transport] ?? transportTile.stdio
              )}
            >
              {transportIcon}
            </div>
            <div className="min-w-0 flex-1">
              <h2
                className="text-[14px] font-bold tracking-tight text-foreground truncate"
                title={server.name}
              >
                {server.name}
              </h2>
              <div className="mt-0.5 flex items-center gap-1.5">
                <DotBadge
                  dotClass={(transportStyles[server.transport] ?? transportStyles.stdio).dot}
                  textClass={(transportStyles[server.transport] ?? transportStyles.stdio).text}
                >
                  {server.transport.toUpperCase()}
                </DotBadge>
                <DotBadge
                  dotClass={server.enabled
                    ? "bg-emerald-500 shadow-[0_0_0_2px_rgba(16,185,129,0.2)]"
                    : "bg-zinc-400 dark:bg-zinc-600"}
                  textClass={server.enabled ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground"}
                >
                  {server.enabled ? t("mcp.enabled") : t("mcp.disabled")}
                </DotBadge>
              </div>
            </div>
          </div>

          {/* Always visible Switch */}
          <Switch
            checked={server.enabled}
            disabled={mutationBusy}
            onCheckedChange={(v) => toggleMutation.mutate({ name: server.name, enabled: v })}
            aria-label={`${server.name} status`}
          />
        </div>

        {/* Monospace Command / URL Box */}
        <div className="relative mt-3 rounded-lg border border-border/70 bg-muted/50 dark:bg-black/40 px-2.5 py-2 font-mono text-[11px] leading-relaxed">
          <div className="flex items-center justify-between gap-2">
            <span
              className="min-w-0 flex-1 truncate text-foreground/85 select-all"
              title={cmdString}
            >
              {isStdio ? `> ${cmdString}` : cmdString}
            </span>
            <button
              type="button"
              onClick={() => handleCopyCommand(server)}
              className="shrink-0 p-1 rounded hover:bg-background/80 text-muted-foreground hover:text-foreground transition-colors"
              title={t("mcp.copyCommand")}
            >
              {isCopied ? (
                <Check className="h-3 w-3 text-emerald-500" />
              ) : (
                <Copy className="h-3 w-3" />
              )}
            </button>
          </div>
        </div>

        {/* Metadata Badges: Env & Headers */}
        <div className="mt-2.5 flex flex-wrap items-center gap-1.5 min-h-[22px]">
          {hasEnv ? (
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="inline-flex items-center gap-1 rounded-md border border-amber-500/25 bg-amber-500/10 px-2 py-0.5 text-[11px] font-medium text-amber-600 dark:text-amber-400 cursor-default">
                  <Key className="h-3 w-3" />
                  <span>{envKeys.length} ENV</span>
                </div>
              </TooltipTrigger>
              <TooltipContent className="text-left font-mono text-[11px] max-w-xs">
                <p className="font-sans font-semibold mb-1 text-xs">{t("mcp.envTooltip")}:</p>
                <div className="flex flex-col gap-0.5">
                  {envKeys.map((k) => (
                    <span key={k} className="truncate">
                      • {k}
                    </span>
                  ))}
                </div>
              </TooltipContent>
            </Tooltip>
          ) : (
            <span className="inline-flex items-center gap-1 text-[11px] text-muted-foreground/50 px-1">
              <Key className="h-3 w-3 opacity-40" />
              <span>{t("mcp.noEnv")}</span>
            </span>
          )}

          {hasHeaders && (
            <Tooltip>
              <TooltipTrigger asChild>
                <div className="inline-flex items-center gap-1 rounded-md border border-violet-500/25 bg-violet-500/10 px-2 py-0.5 text-[11px] font-medium text-violet-600 dark:text-violet-400 cursor-default">
                  <Layers className="h-3 w-3" />
                  <span>{headerKeys.length} Headers</span>
                </div>
              </TooltipTrigger>
              <TooltipContent className="text-left font-mono text-[11px] max-w-xs">
                <p className="font-sans font-semibold mb-1 text-xs">{t("mcp.headersTooltip")}:</p>
                <div className="flex flex-col gap-0.5">
                  {headerKeys.map((h) => (
                    <span key={h} className="truncate">
                      • {h}
                    </span>
                  ))}
                </div>
              </TooltipContent>
            </Tooltip>
          )}
        </div>
      </div>

      {/* Card Footer: Protocol info + Edit/Delete actions */}
      <div className="mt-3.5 pt-2.5 border-t border-border/50 flex items-center justify-between">
        <span className="text-[11px] text-muted-foreground font-mono">
          {isStdio ? "stdio process" : server.transport.toUpperCase()}
        </span>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => setEditing(server)}
            disabled={mutationBusy}
            className="h-7 w-7 text-muted-foreground hover:text-foreground"
            title={t("mcp.edit")}
          >
            <Pencil className="h-3.5 w-3.5" />
          </Button>
          <Button
            variant="ghost"
            size="icon-sm"
            onClick={() => setRemoving(server.name)}
            disabled={mutationBusy}
            className="h-7 w-7 text-muted-foreground hover:text-destructive hover:bg-destructive/10"
            title={t("mcp.delete")}
          >
            <Trash2 className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
    </BentoCard>
  );
}
