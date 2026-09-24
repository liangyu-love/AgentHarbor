import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Eye, EyeOff, Loader2, Plug } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { toast } from "@/hooks/use-toast";
import { supportsProtocol } from "@/lib/provider-models";
import type { ModelProtocol, NewApiProvider, NewApiStateChange, NewApiModel } from "@/types/newapi";
import { ProviderModelEditor } from "@/components/newapi/provider-model-editor";

const emptyDraft = { name: "", baseUrl: "", userId: "", accessToken: "", apiKey: "", protocol: "responses" as ModelProtocol, model: "", contextWindow: "", autoCompactLimit: "" };
const fromProvider = (provider: NewApiProvider) => ({
  name: provider.name,
  baseUrl: provider.baseUrl,
  userId: provider.userId?.toString() ?? "",
  accessToken: "",
  apiKey: "",
  protocol: provider.protocol,
  model: provider.model,
  contextWindow: provider.contextWindow?.toString() ?? "",
  autoCompactLimit: provider.autoCompactLimit?.toString() ?? "",
});
function optionalNumber(text: string, errorKey: string) {
  if (!text.trim()) return null;
  const value = Number(text);
  if (!/^\d+$/.test(text) || !Number.isSafeInteger(value) || value <= 0) throw new Error(errorKey);
  return value;
}
const protocolLabelKey: Record<ModelProtocol, string> = {
  responses: "newapi.protocolResponses",
  chatCompletions: "newapi.protocolChatCompletions",
  anthropicMessages: "newapi.protocolAnthropicMessages",
};

function SecretInput({ id, value, placeholder, disabled, onChange }: {
  id: string;
  value: string;
  placeholder?: string;
  disabled?: boolean;
  onChange: (value: string) => void;
}) {
  const [visible, setVisible] = useState(false);
  return (
    <div className="relative">
      <Input
        id={id}
        type={visible ? "text" : "password"}
        autoComplete="off"
        spellCheck={false}
        className="pr-9"
        value={value}
        disabled={disabled}
        placeholder={placeholder}
        onChange={e => onChange(e.target.value)}
      />
      <button
        type="button"
        onClick={() => setVisible(v => !v)}
        aria-label={visible ? "hide" : "show"}
        className="absolute right-2.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground"
      >
        {visible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
      </button>
    </div>
  );
}

function SectionLabel({ children }: { children: string }) {
  return (
    <div className="flex items-center gap-2.5 pt-1">
      <span className="text-[11px] font-semibold uppercase tracking-wider text-primary/80">{children}</span>
      <span aria-hidden className="h-px flex-1 bg-gradient-to-r from-border/80 to-transparent" />
    </div>
  );
}

interface ProviderDialogProps {
  open: boolean;
  provider: NewApiProvider | null;
  onOpenChange: (open: boolean) => void;
  onSaved: (change: NewApiStateChange) => void;
}

export function ProviderDialog({ open, provider, onOpenChange, onSaved }: ProviderDialogProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(emptyDraft);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [models, setModels] = useState<NewApiModel[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  useEffect(() => {
    if (open) {
      setDraft(provider ? fromProvider(provider) : emptyDraft);
      setError(null);
      setShowAdvanced(!!provider && (!!provider.userId || provider.hasAccessToken || !!provider.contextWindow || !!provider.autoCompactLimit));
      setModels(provider?.models.length ? provider.models : provider?.model ? [{
        id: provider.model,
        displayName: provider.model,
        contextWindow: provider.contextWindow,
        maxOutputTokens: null,
        endpointTypes: [],
        included: true,
        metadataSource: "local",
      }] : []);
      setFetchError(null);
    }
  }, [open, provider]);
  const update = (key: keyof typeof draft, value: string) => setDraft(d => ({ ...d, [key]: value }));
  const field = (key: keyof typeof draft, label: string, options: { password?: boolean; hint?: string; placeholder?: string; numeric?: boolean; list?: string } = {}) => (
    <div className="space-y-1.5">
      <Label htmlFor={`newapi-provider-${key}`} className="text-[13px]">{t(label)}</Label>
      {options.password ? (
        <SecretInput
          id={`newapi-provider-${key}`}
          value={draft[key]}
          placeholder={options.placeholder}
          disabled={saving}
          onChange={value => update(key, value)}
        />
      ) : (
        <Input
          id={`newapi-provider-${key}`}
          type="text"
          inputMode={options.numeric ? "numeric" : undefined}
          autoComplete="off"
          spellCheck={false}
          list={options.list}
          value={draft[key]}
          disabled={saving}
          placeholder={options.placeholder}
          onChange={e => update(key, e.target.value)}
          aria-describedby={options.hint ? `newapi-provider-${key}-hint` : undefined}
        />
      )}
      {options.hint && <p id={`newapi-provider-${key}-hint`} className="text-[11px] leading-relaxed text-muted-foreground">{options.hint}</p>}
    </div>
  );
  const submit = async () => {
    if (saving) return;
    setSaving(true);
    setError(null);
    try {
      const change = await api.providers.save({
        id: provider?.id ?? null,
        name: draft.name,
        baseUrl: draft.baseUrl,
        userId: optionalNumber(draft.userId, "newapi.errors.userId"),
        accessToken: draft.accessToken,
        apiKey: draft.apiKey,
        protocol: draft.protocol,
        model: draft.model,
        models,
        contextWindow: optionalNumber(draft.contextWindow, "newapi.errors.context"),
        autoCompactLimit: optionalNumber(draft.autoCompactLimit, "newapi.errors.context"),
      });
      onSaved(change);
      onOpenChange(false);
      toast({ title: t("newapi.saved"), variant: "success" });
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setError(key);
      if (key === "newapi.errors.userId" || key === "newapi.errors.context") setShowAdvanced(true);
    } finally {
      setSaving(false);
    }
  };
  const hasAdvancedConfigured =
    (!!provider && (!!provider.userId || provider.hasAccessToken || !!provider.contextWindow || !!provider.autoCompactLimit)) ||
    !!draft.contextWindow || !!draft.autoCompactLimit || !!draft.userId || !!draft.accessToken;

  const fetchModels = async () => {
    if (fetchingModels || saving) return;
    if (!draft.baseUrl.trim()) {
      setFetchError("newapi.errors.url");
      return;
    }
    setFetchingModels(true);
    setFetchError(null);
    try {
      const fetched = await api.providers.fetchModels(
        provider?.id ?? null,
        draft.baseUrl.trim(),
        draft.apiKey,
        draft.protocol,
      );
      setModels(current => {
        const fetchedIds = new Set(fetched.map(model => model.id));
        const merged = fetched.map(model => {
          const existing = current.find(item => item.id === model.id);
          if (!existing) return model;
          const manualMetadata = existing.metadataSource === "manual";
          return {
            ...model,
            included: existing.included,
            displayName: manualMetadata ? existing.displayName : model.displayName,
            contextWindow: manualMetadata ? existing.contextWindow : model.contextWindow,
            maxOutputTokens: manualMetadata ? existing.maxOutputTokens : model.maxOutputTokens,
            metadataSource: manualMetadata ? "manual" as const : model.metadataSource,
          };
        });
        merged.push(...current.filter(model => !fetchedIds.has(model.id)));
        setDraft(value => {
          const defaultAvailable = merged.some(model => model.id === value.model && model.included && supportsProtocol(model, value.protocol));
          return defaultAvailable ? value : { ...value, model: merged.find(model => model.included && supportsProtocol(model, value.protocol))?.id ?? "" };
        });
        return merged;
      });
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setFetchError(key.startsWith("newapi.") ? key : "newapi.errors.unexpected");
    } finally {
      setFetchingModels(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[92vh] overflow-y-auto sm:max-w-4xl">
        <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 -m-6 mb-0 h-28 bg-gradient-to-b from-primary/[0.07] via-primary/[0.02] to-transparent" />
        <div className="relative flex items-center justify-between gap-3 border-b border-border/40 pb-3">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-primary/20 to-primary/5 ring-1 ring-inset ring-primary/15">
              <Plug className="h-5 w-5 text-primary" />
            </div>
            <DialogHeader className="min-w-0 space-y-0.5 text-left">
              <DialogTitle className="text-base font-semibold">{t(provider ? "newapi.dialogTitleEdit" : "newapi.dialogTitleAdd")}</DialogTitle>
              <DialogDescription className="text-xs text-muted-foreground">{t("newapi.dialogSubtitle")}</DialogDescription>
            </DialogHeader>
          </div>
          <span className="hidden sm:inline-flex items-center rounded-full bg-primary/10 px-2.5 py-0.5 text-[11px] font-medium text-primary ring-1 ring-inset ring-primary/20">
            {t(protocolLabelKey[draft.protocol])}
          </span>
        </div>

        <form className="relative space-y-4 pt-1" onSubmit={e => { e.preventDefault(); void submit(); }}>
          {error && (
            <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 p-2.5 text-[13px] text-destructive">
              {t(error)}
            </div>
          )}

          {/* 2-Column Main Section */}
          <div className="grid gap-4 sm:grid-cols-2">
            {/* Left Column: Basic Connection */}
            <div className="space-y-3">
              <SectionLabel>{t("newapi.sectionConnection")}</SectionLabel>
              {field("name", "newapi.name", { placeholder: t("newapi.namePlaceholder") })}
              {field("baseUrl", "newapi.url", { placeholder: "https://example.com" })}
              {field("apiKey", "newapi.apiKey", {
                password: true,
                placeholder: provider?.hasApiKey ? t("newapi.keepSecret") : "sk-…",
                hint: t("newapi.apiKeyHint"),
              })}
            </div>

            {/* Right Column: API Compatibility */}
            <div className="space-y-3">
              <SectionLabel>{t("newapi.protocol")}</SectionLabel>
              <div className="space-y-1.5">
                <Label htmlFor="newapi-provider-protocol" className="text-[13px]">{t("newapi.protocol")}</Label>
                <Select value={draft.protocol} onValueChange={value => {
                  const protocol = value as ModelProtocol;
                  setDraft(current => ({
                    ...current,
                    protocol,
                    model: models.some(model => model.id === current.model && model.included && supportsProtocol(model, protocol))
                      ? current.model
                      : models.find(model => model.included && supportsProtocol(model, protocol))?.id ?? "",
                  }));
                }} disabled={saving}>
                  <SelectTrigger id="newapi-provider-protocol">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="responses">{t("newapi.protocolResponses")}</SelectItem>
                    <SelectItem value="chatCompletions">{t("newapi.protocolChatCompletions")}</SelectItem>
                    <SelectItem value="anthropicMessages">{t("newapi.protocolAnthropicMessages")}</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              <p className="rounded-lg bg-primary/[0.05] px-2.5 py-2 text-[11px] leading-relaxed text-muted-foreground ring-1 ring-primary/15">
                {t("newapi.protocolHint")}
              </p>
            </div>
          </div>

          <div className="border-t border-border/50 pt-4">
            <ProviderModelEditor
              models={models}
              protocol={draft.protocol}
              defaultModel={draft.model}
              disabled={saving || fetchingModels}
              fetching={fetchingModels}
              fetchError={fetchError}
              onFetch={() => void fetchModels()}
              onModelsChange={setModels}
              onDefaultModelChange={model => update("model", model)}
            />
          </div>

          {/* Collapsible Advanced & Balance Settings */}
          <div className="space-y-2.5 pt-1 border-t border-border/50">
            <button
              type="button"
              onClick={() => setShowAdvanced(v => !v)}
              aria-expanded={showAdvanced}
              className="flex w-full items-center justify-between py-1 text-left text-muted-foreground transition-colors hover:text-foreground"
            >
              <div className="flex items-center gap-2">
                <span className="text-[11px] font-semibold uppercase tracking-wider text-primary/80">
                  {t("newapi.sectionAdvanced")} & {t("newapi.sectionBalance")}
                </span>
                {!showAdvanced && hasAdvancedConfigured && (
                  <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-400">
                    {t("newapi.optionalConfigured")}
                  </span>
                )}
              </div>
              <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", showAdvanced && "rotate-180")} />
            </button>

            {showAdvanced && (
              <div className="grid gap-3 sm:grid-cols-2 rounded-xl bg-black/[0.02] p-3 ring-1 ring-border/50 dark:bg-white/[0.02]">
                {field("contextWindow", "newapi.contextWindow", { numeric: true, placeholder: t("newapi.useDefault") })}
                {field("autoCompactLimit", "newapi.autoCompact", { numeric: true, placeholder: t("newapi.useDefault") })}
                {field("userId", "newapi.userId", { numeric: true, hint: t("newapi.userIdHint") })}
                {field("accessToken", "newapi.accessToken", {
                  password: true,
                  placeholder: provider?.hasAccessToken ? t("newapi.keepSecret") : "",
                  hint: t("newapi.accessTokenHint"),
                })}
              </div>
            )}
          </div>

          {/* Footer: Dialog Tips + Actions */}
          <div className="flex flex-col-reverse sm:flex-row sm:items-center justify-between gap-3 border-t border-border/40 pt-3">
            <p className="max-w-xs text-[11px] leading-relaxed text-muted-foreground/80">
              {t("newapi.dialogTips")}
            </p>
            <DialogFooter className="gap-2 sm:space-x-0 shrink-0">
              <Button type="button" variant="ghost" size="sm" onClick={() => onOpenChange(false)} disabled={saving}>
                {t("common.cancel")}
              </Button>
              <Button type="submit" size="sm" disabled={saving || !draft.name.trim() || !draft.baseUrl.trim() || !draft.model || !models.some(model => model.id === draft.model && model.included)}>
                {saving && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
                {t("newapi.save")}
              </Button>
            </DialogFooter>
          </div>
        </form>
      </DialogContent>
    </Dialog>
  );
}
