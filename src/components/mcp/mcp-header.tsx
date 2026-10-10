import { useTranslation } from "react-i18next";
import { Plus, RotateCw, Server } from "lucide-react";
import { PageHero } from "@/components/layout/page-hero";
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
    <PageHero kicker="MCP Servers" watermark="MCP" icon={<Server />} title={t("mcp.title")} description={t("mcp.description")} actions={<>
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
    </>} />
  );
}
