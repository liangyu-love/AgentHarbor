import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Copy, Share2 } from "lucide-react";
import { api } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { useFold } from "@/hooks/use-fold";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { AggregationState } from "@/types/aggregation";
import type { ModelProtocol } from "@/types/newapi";
import { cn } from "@/lib/utils";

type Profile = "generic" | "anthropicSdk" | "opencode";

// The local aggregation endpoint other tools can point at, and the connection
// settings to give them.
export function AggregateGateway({ state, ready, busy, run, onChanged }: {
  state: AggregationState | undefined;
  ready: boolean;
  busy: boolean;
  run: (action: () => Promise<unknown>) => void;
  onChanged: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const fold = useFold("aggregation-gateway");
  const [exportId, setExportId] = useState("");
  const [ingressProtocol, setIngressProtocol] = useState<ModelProtocol>("responses");
  const [consumerProfile, setConsumerProfile] = useState<Profile>("generic");
  const [configText, setConfigText] = useState<string | null>(null);
  // Any change to the aggregates withdraws a preview made before it.
  useEffect(() => setConfigText(null), [state]);

  const available = (state?.models ?? []).filter((model) => model.available);
  const selectedExport = exportId ? available.find((model) => model.id === exportId) : available[0];
  const profileName = consumerProfile === "generic" ? t("aggregation.generic") : consumerProfile === "anthropicSdk" ? "Anthropic SDK" : "OpenCode";

  const connection = (revealCredential: boolean) =>
    api.aggregation.exportConnection({
      ingressProtocol,
      aggregateIds: available.map((model) => model.id),
      selectedAggregateId: selectedExport?.id ?? null,
      consumerProfile,
      revealCredential,
    });

  return (
    <div className="agg-gateway" role="group" aria-labelledby="aggregate-export-title">
      <FoldHeader
        open={fold.open}
        onToggle={fold.toggle}
        titleId="aggregate-export-title"
        contentId="aggregate-gateway-body"
        title={t("aggregation.gateway")}
        titleClassName="text-sm"
        leading={<Share2 className="h-4 w-4 text-muted-foreground" />}
        meta={
          <span className={cn("inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] font-medium",
            state?.listenerReady ? "bg-emerald-500/10 text-emerald-700 dark:text-emerald-300" : "bg-muted text-muted-foreground")}>
            <span aria-hidden className={cn("h-1.5 w-1.5 rounded-full", state?.listenerReady ? "bg-emerald-500" : "bg-muted-foreground/40")} />
            {t(state?.listenerReady ? "aggregation.listenerReady" : "aggregation.listenerStopped")}
          </span>
        }
        actions={
          <div className="flex items-center gap-2">
            <Label htmlFor="aggregate-external-enabled" className="cursor-pointer text-xs font-medium">
              {t("aggregation.keepService")}
            </Label>
            <Switch
              id="aggregate-external-enabled"
              checked={state?.externalEnabled ?? false}
              disabled={busy || !ready}
              onCheckedChange={(value) =>
                run(async () => {
                  await api.aggregation.setExternalEnabled(value);
                  await onChanged();
                  setConfigText(null);
                })
              }
            />
          </div>
        }
      />

      <FoldBody open={fold.open} id="aggregate-gateway-body" className="mt-4 space-y-3">
        <div className="grid grid-cols-1 items-end gap-3 md:grid-cols-[repeat(3,minmax(0,1fr))_auto]">
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor="aggregate-export-model" className="text-xs font-medium text-muted-foreground">{t("aggregation.defaultModel")}</Label>
            <Select value={selectedExport?.id ?? ""} onValueChange={(value) => { setExportId(value); setConfigText(null); }}>
              <SelectTrigger id="aggregate-export-model" className="bg-card text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>
                {exportId && !selectedExport && <SelectItem value={exportId} disabled>{t("aggregation.unavailable")}</SelectItem>}
                {available.map((model) => <SelectItem key={model.id} value={model.id}>{model.displayName}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor="aggregate-export-protocol" className="text-xs font-medium text-muted-foreground">{t("aggregation.ingress")}</Label>
            <Select value={ingressProtocol} onValueChange={(value) => { setIngressProtocol(value as ModelProtocol); setConfigText(null); }}>
              <SelectTrigger id="aggregate-export-protocol" className="bg-card text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="responses" disabled={consumerProfile === "anthropicSdk"}>Responses</SelectItem>
                <SelectItem value="chatCompletions" disabled={consumerProfile === "anthropicSdk"}>Chat</SelectItem>
                <SelectItem value="anthropicMessages">Messages</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="min-w-0 space-y-1.5">
            <Label htmlFor="aggregate-export-profile" className="text-xs font-medium text-muted-foreground">{t("aggregation.template")}</Label>
            <Select
              value={consumerProfile}
              onValueChange={(value) => {
                setConsumerProfile(value as Profile);
                if (value === "anthropicSdk") setIngressProtocol("anthropicMessages");
                setConfigText(null);
              }}
            >
              <SelectTrigger id="aggregate-export-profile" className="bg-card text-xs"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="generic">{t("aggregation.generic")}</SelectItem>
                <SelectItem value="anthropicSdk">Anthropic SDK</SelectItem>
                <SelectItem value="opencode">OpenCode</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" className="h-9 text-xs" disabled={busy || !selectedExport}
              onClick={() => run(async () => setConfigText((await connection(false)).configText))}>
              {t("aggregation.previewConnection")}
            </Button>
            <Button size="sm" className="h-9 gap-1.5 text-xs shadow-sm" disabled={busy || !selectedExport}
              onClick={() => run(async () => {
                const result = await connection(true);
                await navigator.clipboard.writeText(result.configText);
                toast({ title: t("aggregation.copied"), variant: "success" });
              })}>
              <Copy className="h-3.5 w-3.5" />
              {t("aggregation.copyConnection")}
            </Button>
          </div>
        </div>

        {configText !== null && (
          <div className="relative rounded-xl border border-border/60 bg-zinc-950 p-4 font-mono text-xs text-zinc-100 shadow-inner">
            <div className="mb-2 flex items-center justify-between border-b border-zinc-800 pb-2 text-[11px] text-zinc-400">
              <span>{t("aggregation.template")} · {profileName}</span>
              <button
                type="button"
                disabled={busy}
                onClick={() => run(async () => {
                  await navigator.clipboard.writeText(configText);
                  toast({ title: t("aggregation.copied"), variant: "success" });
                })}
                className="flex shrink-0 items-center gap-1 rounded px-1 py-1 transition-colors hover:text-white focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50"
              >
                <Copy className="h-3 w-3" />
                <span>{t("aggregation.copyAll")}</span>
              </button>
            </div>
            <pre className="max-h-72 overflow-auto font-mono text-xs leading-relaxed" data-aggregate-export-preview>{configText}</pre>
          </div>
        )}
      </FoldBody>
    </div>
  );
}
