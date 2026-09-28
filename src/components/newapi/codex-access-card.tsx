import { useTranslation } from "react-i18next";
import { CircleHelp, Loader2, RefreshCw, Settings2, Sparkles } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Switch } from "@/components/ui/switch";
import { AuroraCard } from "@/components/ui/aurora-card";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { toast } from "@/hooks/use-toast";
import { providerModelProtocol } from "@/lib/provider-models";
import { protocolBadgeKey, softBadge } from "@/components/newapi/relay-groups";
import type { ProviderState } from "@/components/newapi/use-provider-state";
import type { CodexAccessMode, NewApiProvider } from "@/types/newapi";

// Codex routing switch, access mode picker and accountless toggle.
export function CodexAccessCard({ store, providers, selectedProvider, nativeCompatible, onRestart }: {
  store: ProviderState;
  providers: NewApiProvider[];
  selectedProvider: NewApiProvider | undefined;
  nativeCompatible: boolean;
  onRestart: () => void;
}) {
  const { t } = useTranslation();
  const { busy, mutationLocked, run, applyChange, appliedNote } = store;
  const router = store.state.data?.router;
  const routingEnabled = !!router?.enabled;
  const accessMode = router?.accessMode ?? "router";
  const nativeAccess = accessMode === "native";
  const configDrifted = !!router?.configDrifted;

  return (
      <AuroraCard
        color="blue"
        shine={routingEnabled}
      >
        <div aria-labelledby="newapi-routing-title" className="p-5 sm:p-6">
          <div className="flex flex-col gap-4 sm:flex-row sm:items-start sm:justify-between">
            <div className="flex items-start gap-3.5">
              <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/10 text-primary">
                <Sparkles className="h-5 w-5" />
              </div>
              <div className="min-w-0 flex-1">
                <div className="flex flex-wrap items-center gap-2.5">
                  <h2 id="newapi-routing-title" className="text-base font-semibold tracking-tight">{t("newapi.codexAccess")}</h2>
                  {routingEnabled && !configDrifted && <Badge className={softBadge("green")}>{t("newapi.enabledBadge")}</Badge>}
                  {configDrifted && <Badge className={softBadge("amber")}>{t("newapi.driftedBadge")}</Badge>}
                </div>
                <p className="mt-1 text-sm leading-relaxed text-muted-foreground">{t(nativeAccess ? "newapi.nativeAccessDesc" : "newapi.smartRoutingDesc")}</p>
              </div>
            </div>

            <div className="flex items-center gap-3 self-end sm:self-auto shrink-0">
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-9 gap-2 rounded-xl border-border/80 bg-background/80 px-3.5 text-xs font-semibold shadow-xs backdrop-blur-md transition-all hover:border-primary/40 hover:bg-background hover:shadow-sm"
                disabled={!!busy}
                onClick={onRestart}
              >
                {busy === "restart-codex" ? (
                  <Loader2 className="h-4 w-4 animate-spin text-primary" />
                ) : (
                  <RefreshCw className="h-4 w-4 text-primary" />
                )}
                <span>{t(busy === "restart-codex" ? "newapi.restartingCodex" : "newapi.restartCodex")}</span>
              </Button>
              <Switch
                aria-label={t("newapi.codexAccess")}
                checked={routingEnabled}
                disabled={mutationLocked || (!routingEnabled && (providers.length === 0 || (nativeAccess && !nativeCompatible)))}
                onCheckedChange={value =>
                  void run("routing", async () => {
                    const change = await api.routing.setEnabled(value);
                    applyChange(change);
                    toast({ title: t(value ? "newapi.routingApplied" : "newapi.routingRemoved"), description: change.applied?.backupPath ?? undefined, variant: "success" });
                  }, t("newapi.routingFailed"))
                }
              />
            </div>
          </div>

          <div className="mt-5 space-y-2.5 rounded-xl border border-border/60 bg-muted/20 p-4 backdrop-blur-xs">
            <Label htmlFor="codex-access-mode" className="text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              {t("newapi.accessMode")}
            </Label>
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <Select value={accessMode} disabled={mutationLocked} onValueChange={value => void run("access-mode", async () => {
                applyChange(await api.routing.setAccessMode(value as CodexAccessMode));
                toast({ title: t("newapi.accessModeApplied"), variant: "success" });
              }, t("newapi.routingFailed"))}>
                <SelectTrigger id="codex-access-mode" className="w-full sm:max-w-xs bg-background/80"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="router">{t("newapi.accessRouter")}</SelectItem>
                  <SelectItem value="native" disabled={!nativeCompatible}>{t("newapi.accessNative")}</SelectItem>
                </SelectContent>
              </Select>
              {selectedProvider && (
                <div className="text-xs text-muted-foreground truncate">
                  {t("newapi.accessSelectedModel", { provider: selectedProvider.name, model: selectedProvider.model, protocol: t(protocolBadgeKey[providerModelProtocol(selectedProvider)]) })}
                </div>
              )}
            </div>
            {!nativeCompatible && <p className="text-xs text-amber-700 dark:text-amber-300">{t(selectedProvider ? "newapi.nativeProtocolHint" : "newapi.nativeProviderHint")}</p>}
            {nativeAccess && <p className="text-xs leading-relaxed text-muted-foreground">{t("newapi.nativeAuthHint")}</p>}
            <p className="text-xs leading-relaxed text-muted-foreground">{t(nativeAccess ? "newapi.nativeNetworkHint" : "newapi.routerNetworkHint")}</p>
          </div>
          <p className="mt-3 text-sm leading-relaxed text-muted-foreground">
            {t(routingEnabled ? (nativeAccess ? "newapi.nativeOnHint" : "newapi.routingOnHint") : "newapi.routingOffHint")}
          </p>
          {configDrifted && (
            <p role="alert" className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/[0.07] px-3 py-2 text-[13px] leading-relaxed text-amber-700 dark:text-amber-300">
              {t("newapi.configDriftedHint")}
            </p>
          )}
          {appliedNote && (
            <p className="mt-2 break-all font-mono text-[11px] text-muted-foreground/80">
              {appliedNote.path}
              {appliedNote.backupPath ? ` · ${t("newapi.backup")}: ${appliedNote.backupPath}` : ""}
            </p>
          )}
          {!nativeAccess && <div className="mt-4 flex items-center justify-between gap-4 rounded-xl bg-black/[0.03] p-3.5 ring-1 ring-border/60 dark:bg-white/[0.03]">
            <div className="min-w-0">
              <div className="flex items-center gap-1.5">
                <Settings2 className="h-3.5 w-3.5 text-muted-foreground" />
                <span className="text-sm font-semibold">{t("newapi.accountless")}</span>
                <span title={t("newapi.accountlessTip")}>
                  <CircleHelp className="h-4 w-4 text-muted-foreground" />
                </span>
              </div>
              <p className="mt-0.5 text-[13px] text-muted-foreground">{t("newapi.accountlessHint")}</p>
            </div>
            <Switch
              aria-label={t("newapi.accountless")}
              checked={!!router?.accountless}
              disabled={!routingEnabled || mutationLocked}
              onCheckedChange={value =>
                void run("accountless", async () => {
                  const change = await api.routing.setAccountless(value);
                  applyChange(change);
                  toast({ title: t("newapi.accountlessApplied"), variant: "success" });
                }, t("newapi.routingFailed"))
              }
            />
          </div>}
        </div>
      </AuroraCard>
  );
}
