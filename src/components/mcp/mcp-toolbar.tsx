import { useTranslation } from "react-i18next";
import { Search, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { Input } from "@/components/ui/input";
import type { McpFilter } from "@/components/mcp/use-mcp-servers";
import type { McpServers } from "@/components/mcp/use-mcp-servers";

export function McpToolbar({ mcp }: { mcp: McpServers }) {
  const { t } = useTranslation();
  const {
    searchQuery, setSearchQuery, setCurrentPage, servers, enabledCount, disabledCount, stdioCount, remoteCount,
    transportFilter, setTransportFilter,
  } = mcp;

  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="relative min-w-[240px] max-w-sm flex-1">
        <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground pointer-events-none" />
        <Input
          value={searchQuery}
          onChange={(e) => {
            setSearchQuery(e.target.value);
            setCurrentPage(1);
          }}
          placeholder={t("mcp.searchPlaceholder")}
          className="pl-9 pr-8 h-9 text-sm rounded-lg"
        />
        {searchQuery && (
          <button
            type="button"
            onClick={() => {
              setSearchQuery("");
              setCurrentPage(1);
            }}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground p-0.5 rounded"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        )}
      </div>

      <div className="flex items-center gap-1.5 overflow-x-auto pb-0.5">
        {[
          { key: "all", label: `${t("mcp.filterAll")} (${servers.length})` },
          { key: "enabled", label: `${t("mcp.filterEnabled")} (${enabledCount})` },
          { key: "disabled", label: `${t("mcp.filterDisabled")} (${disabledCount})` },
          { key: "stdio", label: `STDIO (${stdioCount})` },
          { key: "http", label: `HTTP/SSE (${remoteCount})` },
        ].map((f) => {
          const active = transportFilter === f.key;
          return (
            <button
              key={f.key}
              type="button"
              onClick={() => {
                setTransportFilter(f.key as McpFilter);
                setCurrentPage(1);
              }}
              className={cn(
                "flex shrink-0 items-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs font-medium transition-all shadow-xs",
                active
                  ? "border-primary/50 bg-primary/10 text-primary font-semibold ring-1 ring-primary/20"
                  : "border-border/60 bg-card text-muted-foreground hover:bg-muted/70 hover:text-foreground"
              )}
            >
              {f.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}
