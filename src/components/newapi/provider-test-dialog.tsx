import { useEffect, useRef, useState } from "react";
import { AlertCircle, Info, Loader2, Network, PlugZap, RadioTower, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { ProbeTimeline } from "./probe-timeline";
import { effectiveModelProtocol, includedModels, mergeModelCatalog } from "@/lib/provider-models";
import type { NewApiModel, NewApiProvider, ProviderProbeEvent } from "@/types/newapi";
import { errorKey } from "@/lib/invoke-error";

const providerErrorKey = (error: unknown) => errorKey(error, ["newapi"], "newapi.errors.unexpected");

interface ProviderTestDialogProps {
  open: boolean;
  provider: NewApiProvider | null;
  onOpenChange: (open: boolean) => void;
  onTested: (provider: NewApiProvider) => void;
  onFailed: () => void;
}

export function ProviderTestDialog({ open, provider, onOpenChange, onTested, onFailed }: ProviderTestDialogProps) {
  const { t } = useTranslation();
  const [models, setModels] = useState<NewApiModel[]>([]);
  const [selectedModel, setSelectedModel] = useState("");
  const [loadingModels, setLoadingModels] = useState(false);
  const [events, setEvents] = useState<ProviderProbeEvent[]>([]);
  const running = useRef(false);
  const [testing, setTesting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const modelRequest = useRef(0);
  const testRequest = useRef(0);

  const loadModels = async (target: NewApiProvider) => {
    const request = ++modelRequest.current;
    setLoadingModels(true);
    setError(null);
    try {
      const loaded = await api.providers.fetchModels(
        target.id,
        target.baseUrl,
        "",
        target.protocol,
      );
      if (request !== modelRequest.current) return;
      const compatible = includedModels(mergeModelCatalog(target.models, loaded));
      setModels(compatible);
      setSelectedModel(current => {
        if (current && compatible.some(model => model.id === current)) return current;
        if (target.model && compatible.some(model => model.id === target.model)) return target.model;
        return compatible[0]?.id ?? "";
      });
    } catch (cause) {
      if (request !== modelRequest.current) return;
      setError(providerErrorKey(cause));
    } finally {
      if (request === modelRequest.current) setLoadingModels(false);
    }
  };

  useEffect(() => {
    modelRequest.current += 1;
    testRequest.current += 1;
    if (!open || !provider) {
      setLoadingModels(false);
      return;
    }
    const cached = includedModels(provider.models);
    setModels(cached);
    setSelectedModel(
      provider.model && cached.some(model => model.id === provider.model)
        ? provider.model
        : (cached[0]?.id ?? ""),
    );
    setError(null);
    setTesting(false);
    setEvents([]);
    if (cached.length === 0) void loadModels(provider);
    // Opening the dialog initializes a fresh test session for the selected card.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, provider?.id]);

  useEffect(() => () => { testRequest.current += 1; modelRequest.current += 1; }, []);

  const runTest = async () => {
    if (!provider || !selectedModel || running.current) return;
    running.current = true;
    setEvents([]);
    const request = ++testRequest.current;
    setTesting(true);
    setError(null);
    try {
      const tested = await api.providers.test(provider.id, selectedModel, event => {
        if (request !== testRequest.current) return;
        setEvents(current => {
          // Bound logs without discarding the final stage.
          if (event.stage === "content" && current.filter(item => item.stage === "content").reduce((n, item) => n + item.text.length, 0) > 16000) return current;
          return [...current, event];
        });
      });
      if (request !== testRequest.current) return;
      onTested(tested);
    } catch (cause) {
      if (request !== testRequest.current) return;
      onFailed();
      setError(providerErrorKey(cause));
    } finally {
      running.current = false;
      if (request === testRequest.current) setTesting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={value => { if (!testing) onOpenChange(value); }}>
      <DialogContent className="sm:max-w-3xl overflow-hidden">
        <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 -m-6 mb-0 h-24 bg-gradient-to-b from-primary/[0.08] via-primary/[0.02] to-transparent" />

        {/* Header */}
        <div className="relative flex items-center justify-between gap-3 border-b border-border/40 pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-primary/20 to-primary/5 text-primary ring-1 ring-inset ring-primary/20 shadow-xs">
              <RadioTower className="h-5 w-5" />
            </div>
            <DialogHeader className="min-w-0 space-y-0.5 text-left">
              <DialogTitle className="text-base font-semibold tracking-tight">
                {t("newapi.testDialogTitle", { name: provider?.name ?? "" })}
              </DialogTitle>
              <DialogDescription className="break-all font-mono text-[11px] text-muted-foreground/80">
                {provider?.baseUrl}
              </DialogDescription>
            </DialogHeader>
          </div>
          {provider && (
            <div className="hidden sm:flex items-center gap-1.5 rounded-full border border-border/60 bg-muted/30 px-2.5 py-1 text-[11px] text-muted-foreground">
              {provider.networkMode === "direct" ? (
                <Network className="h-3 w-3 text-emerald-500" />
              ) : (
                <PlugZap className="h-3 w-3 text-blue-500" />
              )}
              <span>{t(provider.networkMode === "direct" ? "newapi.networkDirect" : "newapi.networkProxy")}</span>
            </div>
          )}
        </div>

        {error && (
          <div role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
            <div className="flex items-start gap-2">
              <AlertCircle className="h-4 w-4 shrink-0 mt-0.5" />
              <span className="font-medium">{t(error)}</span>
            </div>
          </div>
        )}

        {/* Model Selector Card */}
        <div className="space-y-2.5 rounded-xl border border-border/60 bg-muted/15 p-3.5">
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <span className="text-xs font-semibold text-foreground/90">{t("newapi.chooseTestModel")}</span>
              {models.length > 0 && (
                <span className="rounded-full bg-primary/10 px-2 py-0.2 font-mono text-[10px] font-medium text-primary">
                  {models.length}
                </span>
              )}
            </div>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 px-2 text-xs text-muted-foreground hover:text-foreground"
              disabled={!provider || loadingModels || testing}
              onClick={() => provider && void loadModels(provider)}
            >
              {loadingModels ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
              {t("newapi.refreshModels")}
            </Button>
          </div>

          <select
            aria-label={t("newapi.chooseTestModel")}
            value={selectedModel}
            disabled={testing || loadingModels || !models.length}
            onChange={event => { setSelectedModel(event.target.value); setEvents([]); setError(null); }}
            className="native-select h-10 w-full rounded-xl border border-border/70 bg-background/90 px-3 font-mono text-xs shadow-xs"
          >
            {!models.length && <option value="">{t(loadingModels ? "newapi.loadingModels" : "newapi.noModelsForTest")}</option>}
            {models.map(model => (
              <option key={model.id} value={model.id}>
                {model.id} · {effectiveModelProtocol(model, provider?.protocol ?? "responses")}
                {model.contextWindow ? ` · ${Math.round(model.contextWindow / 1000)}K` : ""}
              </option>
            ))}
          </select>

          <p className="flex items-center gap-1.5 text-[11px] leading-relaxed text-muted-foreground">
            <Info className="h-3 w-3 shrink-0" />
            <span>{t("newapi.testRequestHint", { mode: t(provider?.networkMode === "direct" ? "newapi.networkDirect" : "newapi.networkProxy") })}</span>
          </p>
        </div>

        <ProbeTimeline events={events} testing={testing} failed={!!error} />

        <DialogFooter className="gap-2 sm:space-x-0 pt-2 border-t border-border/40">
          <Button type="button" variant="ghost" size="sm" onClick={() => onOpenChange(false)} disabled={testing}>
            {t("common.close")}
          </Button>
          <Button
            type="button"
            size="sm"
            className="gap-1.5 shadow-xs"
            onClick={() => void runTest()}
            disabled={!models.some(model => model.id === selectedModel) || testing || loadingModels}
          >
            {testing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <RadioTower className="h-3.5 w-3.5" />}
            {t("newapi.sendTest")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
