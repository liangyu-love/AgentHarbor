import { useEffect, useRef, useState } from "react";
import { Check, Loader2, RadioTower, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { toast } from "@/hooks/use-toast";
import { effectiveModelProtocol, includedModels, mergeModelCatalog } from "@/lib/provider-models";
import type { NewApiModel, NewApiProvider } from "@/types/newapi";

const errorKey = (error: unknown) => {
  const value = typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
  return value.startsWith("newapi.") ? value : "newapi.errors.unexpected";
};

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
      setError(errorKey(cause));
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
    if (cached.length === 0) void loadModels(provider);
    // Opening the dialog initializes a fresh test session for the selected card.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, provider?.id]);

  const runTest = async () => {
    if (!provider || !selectedModel || testing) return;
    const request = ++testRequest.current;
    setTesting(true);
    setError(null);
    try {
      const tested = await api.providers.test(provider.id, selectedModel);
      onTested(tested);
      if (request !== testRequest.current) return;
      onOpenChange(false);
      toast({
        title: t("newapi.testSucceeded"),
        description: t("newapi.testSucceededBody", { model: selectedModel }),
        variant: "success",
      });
    } catch (cause) {
      onFailed();
      if (request !== testRequest.current) return;
      setError(errorKey(cause));
    } finally {
      if (request === testRequest.current) setTesting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <RadioTower className="h-4 w-4 text-primary" />
            {t("newapi.testDialogTitle", { name: provider?.name ?? "" })}
          </DialogTitle>
          <DialogDescription>{t("newapi.testDialogDescription")}</DialogDescription>
        </DialogHeader>

        {error && (
          <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
            {t(error)}
          </div>
        )}

        <div className="space-y-2">
          <div className="flex items-center justify-between gap-3">
            <span className="text-sm font-medium">{t("newapi.chooseTestModel")}</span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="h-7 gap-1.5 px-2 text-xs"
              disabled={!provider || loadingModels || testing}
              onClick={() => provider && void loadModels(provider)}
            >
              {loadingModels ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
              {t("newapi.refreshModels")}
            </Button>
          </div>

          <div className="max-h-64 overflow-y-auto rounded-xl border bg-muted/20 p-1">
            {loadingModels && models.length === 0 ? (
              <div className="flex items-center justify-center gap-2 py-10 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                {t("newapi.loadingModels")}
              </div>
            ) : models.length === 0 ? (
              <p className="px-3 py-8 text-center text-sm text-muted-foreground">{t("newapi.noModelsForTest")}</p>
            ) : (
              models.map(model => {
                const selected = selectedModel === model.id;
                const protocol = effectiveModelProtocol(model, provider?.protocol ?? "responses");
                return (
                  <button
                    key={model.id}
                    type="button"
                    className={cn(
                      "flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left transition-colors hover:bg-muted",
                      selected && "bg-primary/10 text-primary hover:bg-primary/10",
                    )}
                    onClick={() => setSelectedModel(model.id)}
                  >
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">{model.id}</span>
                    <span className="text-[10px] text-muted-foreground">{t(`newapi.protocolShort${protocol === "responses" ? "Responses" : protocol === "chatCompletions" ? "Chat" : "Anthropic"}`)}</span>
                    {model.contextWindow && (
                      <span className="shrink-0 text-[10px] text-muted-foreground">
                        {Math.round(model.contextWindow / 1000)}K
                      </span>
                    )}
                    <span className={cn("flex h-4 w-4 shrink-0 items-center justify-center rounded-full border", selected && "border-primary bg-primary text-primary-foreground")}>
                      {selected && <Check className="h-3 w-3" />}
                    </span>
                  </button>
                );
              })
            )}
          </div>
          <p className="text-xs leading-relaxed text-muted-foreground">
            {t("newapi.testRequestHint", { mode: t(provider?.networkMode === "direct" ? "newapi.networkDirect" : "newapi.networkProxy") })}
          </p>
        </div>

        <DialogFooter className="gap-2 sm:space-x-0">
          <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={testing}>
            {t("common.cancel")}
          </Button>
          <Button
            type="button"
            onClick={() => void runTest()}
            disabled={!models.some(model => model.id === selectedModel) || testing || loadingModels}
          >
            {testing ? <Loader2 className="h-4 w-4 animate-spin" /> : <RadioTower className="h-4 w-4" />}
            {t("newapi.sendTest")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
