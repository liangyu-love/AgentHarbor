import { useTranslation } from "react-i18next";
import { Copy, FolderOpen, Server, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AuroraCard } from "@/components/ui/aurora-card";
import type { McpServers } from "@/components/mcp/use-mcp-servers";

// Server counts and the config file path banner.
export function McpStats({ mcp }: { mcp: McpServers }) {
  const { t } = useTranslation();
  const {
    servers, stdioCount, remoteCount, enabledCount, handleOpenConfigFolder, copyPath, sourcePath,
  } = mcp;

  return (
    <div className="app-metric-grid grid grid-cols-1 gap-3 lg:grid-cols-3">
      <AuroraCard color="blue">
        <div className="p-4 flex items-center justify-between">
          <div>
            <span className="text-xs font-medium text-muted-foreground">{t("mcp.serverCount")}</span>
            <div className="mt-1 flex items-baseline gap-2">
              <span className="text-2xl font-bold tracking-tight tabular-nums">{servers.length}</span>
              <span className="text-xs text-muted-foreground font-mono">
                ({stdioCount} stdio · {remoteCount} http)
              </span>
            </div>
          </div>
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-blue-500/10 text-blue-500 ring-1 ring-inset ring-blue-500/20">
            <Server className="h-5 w-5" />
          </div>
        </div>
      </AuroraCard>

      <AuroraCard color="green">
        <div className="p-4 flex items-center justify-between">
          <div>
            <span className="text-xs font-medium text-muted-foreground">{t("mcp.enabledCount")}</span>
            <div className="mt-1 flex items-baseline gap-2">
              <span className="text-2xl font-bold tracking-tight text-emerald-600 dark:text-emerald-400 tabular-nums">
                {enabledCount}
              </span>
              <span className="text-xs text-muted-foreground font-mono">
                / {servers.length} 运行中
              </span>
            </div>
          </div>
          <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-emerald-500/10 text-emerald-500 ring-1 ring-inset ring-emerald-500/20">
            <Terminal className="h-5 w-5" />
          </div>
        </div>
      </AuroraCard>

      <AuroraCard color="blue">
        <div className="p-4 flex flex-col justify-between h-full">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium text-muted-foreground">{t("mcp.configFile")}</span>
            <div className="flex items-center gap-1">
              <Button
                variant="ghost"
                size="icon-sm"
                className="h-6 w-6 text-muted-foreground hover:text-foreground"
                title={t("mcp.openFolder")}
                onClick={handleOpenConfigFolder}
              >
                <FolderOpen className="h-3.5 w-3.5" />
              </Button>
              <Button
                variant="ghost"
                size="icon-sm"
                className="h-6 w-6 text-muted-foreground hover:text-foreground"
                title={t("mcp.copyCommand")}
                onClick={copyPath}
              >
                <Copy className="h-3.5 w-3.5" />
              </Button>
            </div>
          </div>
          <div
            className="mt-1 flex items-center gap-1.5 cursor-pointer rounded px-1.5 py-1 -mx-1.5 transition-colors hover:bg-black/[0.04] dark:hover:bg-white/[0.04]"
            onClick={copyPath}
            title={sourcePath}
          >
            <span className="truncate font-mono text-xs font-medium text-foreground/80">{sourcePath}</span>
          </div>
        </div>
      </AuroraCard>
    </div>
  );
}
