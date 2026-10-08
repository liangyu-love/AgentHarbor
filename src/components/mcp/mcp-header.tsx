import { useTranslation } from "react-i18next";
import { Plus, RotateCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ButtonBusyContent } from "@/components/ui/button-busy-content";
import { RainbowButton } from "@/registry/magicui/rainbow-button";
import type { McpServers } from "@/components/mcp/use-mcp-servers";

export function McpHeader({ mcp }: { mcp: McpServers }) {
  const { t } = useTranslation();
  const {
    setEditing, handleRefresh, refreshBusy,
  } = mcp;

  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div>
        <h1 className="text-xl font-bold tracking-tight text-foreground">{t("mcp.title")}</h1>
        <p className="mt-1 text-sm text-muted-foreground">{t("mcp.description")}</p>
      </div>
      <div className="flex items-center gap-2">
        <RainbowButton size="sm" className="h-9 rounded-lg font-medium" disabled={mcp.mutationBusy} onClick={() => setEditing("new")}>
          <Plus className="h-4 w-4" />
          {t("mcp.addServer")}
        </RainbowButton>
        <Button
          variant="outline"
          size="icon"
          className="h-9 w-9"
          onClick={handleRefresh}
          disabled={refreshBusy}
          aria-busy={refreshBusy}
          title={refreshBusy ? t("common.refreshing") : t("common.refresh")}
          aria-label={refreshBusy ? t("common.refreshing") : t("common.refresh")}
        >
          <ButtonBusyContent
            busy={refreshBusy}
            idleIcon={<RotateCw className="h-4 w-4" />}
          />
        </Button>
      </div>
    </div>
  );
}
