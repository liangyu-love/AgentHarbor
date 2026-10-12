import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { ArrowRight } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { openTerminalProxySettings } from "@/lib/terminal-proxy-link";
import type { NewApiProvider } from "@/types/newapi";

const PREFIX = "clients.directProxy.";

/**
 * A direct client reaches its relay itself, outside AgentHarbor's proxy. A
 * terminal proxy reaches it through new terminals; otherwise only Claude Code
 * gets the app's manual HTTP proxy written into its settings.
 */
export function DirectProxyNote({ name, isClaude, provider }: {
  name: string; isClaude: boolean; provider?: NewApiProvider;
}) {
  const { t } = useTranslation();
  const query = useQuery({ queryKey: queryKeys.terminalProxy, queryFn: api.terminalProxy.load, retry: false });
  const status = query.data;
  const supported = status?.supported ?? false;
  const key = status?.applied ? "terminalOn"
    : isClaude ? (provider?.networkMode === "direct" ? "claudeCardDirect" : supported ? "claudeOff" : "claudeUnsupported")
    : supported ? "otherOff" : "otherUnsupported";
  return (
    <section data-direct-proxy={key} aria-label={t(PREFIX + "title")}
      className="space-y-1 rounded-lg border border-border/60 bg-muted/20 px-3 py-2 text-xs leading-relaxed text-muted-foreground">
      <p className="font-medium text-foreground">{t(PREFIX + "title")}</p>
      {!query.isPending && <p>{t(PREFIX + key, { name, url: status?.url ?? "" })}</p>}
      {supported && <button type="button" data-direct-proxy-open onClick={openTerminalProxySettings}
        className="inline-flex items-center gap-1 rounded-sm font-medium text-primary underline-offset-4 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
        {t(PREFIX + "open")}<ArrowRight className="h-3 w-3" aria-hidden />
      </button>}
    </section>
  );
}
