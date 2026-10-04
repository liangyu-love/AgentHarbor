import { useEffect, useMemo, useRef, useState } from "react";
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
import { mergeModelCatalog } from "@/lib/provider-models";
import type { CompressionPolicy, ModelProtocol, NewApiProvider, NewApiStateChange, NewApiModel } from "@/types/newapi";
import { ProviderModelEditor } from "@/components/newapi/provider-model-editor";
import { useModelResolutions } from "@/components/newapi/use-model-resolutions";
import { PROVIDER_PRESETS } from "@/lib/provider-presets";

// A relay usually speaks OpenAI-compatible Chat Completions unless it says
// otherwise, and the Codex-specific Responses protocol is the less common case.
// Fetching models refines this from the capabilities the relay advertises.
const DEFAULT_PROTOCOL: ModelProtocol = "chatCompletions";
interface ProviderDraft {
  name: string;
  baseUrl: string;
  apiKey: string;
  protocol: ModelProtocol;
  model: string;
  contextWindow: string;
  compressionMode: "inherit" | "percent" | "tokens" | "clientDefault";
  compressionValue: string;
  catalogProviders: string;
  priceCatalogProvider: string;
}
const emptyDraft: ProviderDraft = {
  name: "",
  baseUrl: "",
  apiKey: "",
  protocol: DEFAULT_PROTOCOL,
  model: "",
  contextWindow: "",
  compressionMode: "inherit",
  compressionValue: "",
  catalogProviders: "",
  priceCatalogProvider: "",
};
const fromProvider = (provider: NewApiProvider): ProviderDraft => {
  const compression = provider.compression ?? { mode: "inherit" as const };
  return {
    name: provider.name,
    baseUrl: provider.baseUrl,
    apiKey: "",
    protocol: provider.protocol,
    model: provider.model,
    contextWindow: provider.contextWindow?.toString() ?? "",
    compressionMode: compression.mode,
    compressionValue:
      compression.mode === "percent"
        ? String(compression.percent)
        : compression.mode === "tokens"
          ? String(compression.tokens)
          : "",
    catalogProviders: (provider.catalogProviders ?? []).join(", "),
    priceCatalogProvider: provider.priceCatalogProvider ?? "",
  };
};
function optionalNumber(text: string, errorKey: string) {
  if (!text.trim()) return null;
  const value = Number(text);
  if (!/^\d+$/.test(text) || !Number.isSafeInteger(value) || value <= 0) throw new Error(errorKey);
  return value;
}
/** Provider-level compression policy; throws a locale key when the input is invalid. */
function compressionPolicy(draft: ProviderDraft): CompressionPolicy {
  switch (draft.compressionMode) {
    case "inherit":
      return { mode: "inherit" };
    case "clientDefault":
      return { mode: "clientDefault" };
    case "percent": {
      const value = Number(draft.compressionValue || "90");
      if (!Number.isSafeInteger(value) || value < 1 || value > 99) {
        throw new Error("newapi.compression.percentRange");
      }
      return { mode: "percent", percent: value };
    }
    default: {
      const tokens = optionalNumber(draft.compressionValue, "newapi.compression.tokensRange");
      if (tokens === null) throw new Error("newapi.compression.tokensRange");
      return { mode: "tokens", tokens };
    }
  }
}
const protocolLabelKey: Record<ModelProtocol, string> = {
  responses: "newapi.protocolResponses",
  chatCompletions: "newapi.protocolChatCompletions",
  anthropicMessages: "newapi.protocolAnthropicMessages",
};

function SecretInput({ id, value, placeholder, disabled, hasStoredSecret, onReveal, onChange }: {
  id: string;
  value: string;
  placeholder?: string;
  disabled?: boolean;
  hasStoredSecret?: boolean;
  onReveal?: () => Promise<boolean>;
  onChange: (value: string) => void;
}) {
  const [visible, setVisible] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const canToggle = Boolean(value) || Boolean(hasStoredSecret && onReveal);
  const toggle = async () => {
    if (!value && hasStoredSecret && onReveal) {
      setRevealing(true);
      try {
        if (await onReveal()) setVisible(true);
      } finally {
        setRevealing(false);
      }
      return;
    }
    setVisible(current => !current);
  };
  return (
    <div className="relative">
      <Input
        id={id}
        type={visible ? "text" : "password"}
        autoComplete="off"
        spellCheck={false}
        className="pr-9"
        value={value}
        disabled={disabled || revealing}
        placeholder={placeholder}
        onChange={e => onChange(e.target.value)}
      />
      <button
        type="button"
        onClick={() => void toggle()}
        disabled={disabled || !canToggle || revealing}
        aria-label={visible ? "hide" : "show"}
        className="absolute right-2.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
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
  const [draft, setDraft] = useState<ProviderDraft>(emptyDraft);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [models, setModels] = useState<NewApiModel[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const modelRequest = useRef(0);
  const fetchScope = JSON.stringify([open, provider?.id, draft.baseUrl.trim(), draft.apiKey, draft.protocol]);
  const currentFetchScope = useRef(fetchScope);
  currentFetchScope.current = fetchScope;

  useEffect(() => {
    modelRequest.current += 1;
    setFetchingModels(false);
    return () => { modelRequest.current += 1; };
  }, [fetchScope]);

  useEffect(() => {
    if (open) {
      setDraft(provider ? fromProvider(provider) : emptyDraft);
      setError(null);
      setShowAdvanced(!!provider && (!!provider.userId || provider.hasAccessToken || !!provider.contextWindow || (provider.compression?.mode ?? "inherit") !== "inherit" || !!provider.catalogProviders?.length));
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
  const resolutionDraft = useMemo(() => {
    try {
      return {
        providerId: provider?.id ?? null,
        baseUrl: draft.baseUrl.trim(),
        hasDraftApiKey: Boolean(draft.apiKey.trim()),
        protocol: draft.protocol,
        contextWindow: optionalNumber(draft.contextWindow, "newapi.errors.context"),
        catalogProviders: draft.catalogProviders.split(/[\s,]+/).filter(Boolean),
        compression: compressionPolicy(draft),
        models: models.filter(model => model.included),
      };
    } catch {
      return null;
    }
  }, [provider?.id, draft.baseUrl, Boolean(draft.apiKey.trim()), draft.protocol, draft.contextWindow, draft.catalogProviders, draft.compressionMode, draft.compressionValue, models]);
  const resolutions = useModelResolutions(open, resolutionDraft);
  const update = (key: keyof typeof draft, value: string) => setDraft(d => ({ ...d, [key]: value }));
  const secretScope = JSON.stringify([open, provider?.id, draft.baseUrl, draft.apiKey]);
  const currentSecretScope = useRef(secretScope);
  currentSecretScope.current = secretScope;
  const secretGeneration = useRef(0);
  useEffect(() => {
    secretGeneration.current += 1;
    return () => { secretGeneration.current += 1; };
  }, [open, provider]);
  const revealSavedApiKey = async () => {
    if (!provider?.id) return false;
    const scope = secretScope;
    const generation = secretGeneration.current;
    const isCurrent = () => currentSecretScope.current === scope && secretGeneration.current === generation;
    setError(null);
    try {
      const key = await api.providers.revealApiKey(provider.id);
      if (!isCurrent()) return false;
      update("apiKey", key);
      return true;
    } catch (cause) {
      if (isCurrent()) setError(typeof cause === "string" ? cause : cause instanceof Error ? cause.message : String(cause));
      return false;
    }
  };
  const field = (key: keyof typeof draft, label: string, options: { password?: boolean; hint?: string; placeholder?: string; numeric?: boolean; list?: string } = {}) => (
    <div className="space-y-1.5">
      <Label htmlFor={`newapi-provider-${key}`} className="text-[13px]">{t(label)}</Label>
      {options.password ? (
        <SecretInput
          key={`${open}:${provider?.id ?? "new"}:${key}`}
          id={`newapi-provider-${key}`}
          value={draft[key]}
          placeholder={options.placeholder}
          disabled={saving}
          hasStoredSecret={key === "apiKey" && !!provider?.hasApiKey}
          onReveal={key === "apiKey" ? revealSavedApiKey : undefined}
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
      const compression = compressionPolicy(draft);
      const change = await api.providers.save({
        id: provider?.id ?? null,
        name: draft.name,
        baseUrl: draft.baseUrl,
        userId: provider?.userId ?? null,
        accessToken: "",
        apiKey: draft.apiKey,
        protocol: draft.protocol,
        model: draft.model,
        models,
        contextWindow: optionalNumber(draft.contextWindow, "newapi.errors.context"),
        compression,
        catalogProviders: draft.catalogProviders
          .split(/[,\s]+/)
          .map(value => value.trim().toLowerCase())
          .filter(Boolean),
        priceCatalogProvider: draft.priceCatalogProvider.trim().toLowerCase() || null,
      });
      onSaved(change);
      onOpenChange(false);
      toast({ title: t("newapi.saved"), variant: "success" });
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setError(key);
      if (key === "newapi.errors.context") setShowAdvanced(true);
    } finally {
      setSaving(false);
    }
  };
  const hasAdvancedConfigured =
    !!draft.contextWindow
    || draft.compressionMode !== "inherit"
    || !!draft.catalogProviders.trim();

  const fetchModels = async () => {
    if (fetchingModels || saving) return;
    if (!draft.baseUrl.trim()) {
      setFetchError("newapi.errors.url");
      return;
    }
    const request = ++modelRequest.current;
    const scope = currentFetchScope.current;
    const isCurrent = () => request === modelRequest.current && scope === currentFetchScope.current;
    setFetchingModels(true);
    setFetchError(null);
    try {
      const fetched = await api.providers.fetchModels(
        provider?.id ?? null,
        draft.baseUrl.trim(),
        draft.apiKey,
        draft.protocol,
      );
      if (!isCurrent()) return;
      setModels(current => {
        if (!isCurrent()) return current;
        const merged = mergeModelCatalog(current, fetched);
        setDraft(value => {
          const defaultAvailable = Boolean(value.model)
            && merged.some(model => model.id === value.model && model.included);
          return {
            ...value,
            // Keep an explicit default only while it remains selected. A
            // refresh must not silently choose the first fetched model.
            model: defaultAvailable ? value.model : "",
          };
        });
        return merged;
      });
    } catch (e) {
      if (!isCurrent()) return;
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setFetchError(key.startsWith("newapi.") ? key : "newapi.errors.unexpected");
    } finally {
      if (isCurrent()) setFetchingModels(false);
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
          {!provider && (
            <div className="rounded-xl border border-primary/15 bg-gradient-to-r from-primary/[0.04] via-primary/[0.01] to-transparent p-3 space-y-2">
              <div className="flex items-center justify-between">
                <Label htmlFor="provider-preset" className="text-xs font-semibold text-foreground/90">
                  {t("newapi.officialPreset")}
                </Label>
                <span className="text-[11px] text-muted-foreground">
                  一键填入常用官方与专线配置
                </span>
              </div>
              <select id="provider-preset" defaultValue="" disabled={saving} className="h-9 w-full rounded-lg border border-border/70 bg-background/90 px-3 text-xs font-medium shadow-xs transition-colors hover:border-primary/40 focus:border-primary focus:outline-none focus:ring-2 focus:ring-ring" onChange={event => {
              const preset = PROVIDER_PRESETS.find(item => item.name === event.target.value);
              if (!preset) return;
              setDraft({ ...emptyDraft, name: preset.name, baseUrl: preset.baseUrl, protocol: preset.protocol, model: preset.models?.[0]?.id ?? "" });
              setModels(preset.models ?? []); setFetchError(null);
            }}>
              <option value="">{t("newapi.customProvider")} (手动填写)</option>
              <optgroup label="代码与直连专线 (Coding / Direct)">
                {PROVIDER_PRESETS.filter(p => p.category === "coding" || p.category === "direct").map(preset => (
                  <option key={preset.name} value={preset.name}>
                    {preset.name} — {preset.baseUrl}
                  </option>
                ))}
              </optgroup>
              <optgroup label="模型聚合平台 (Aggregators)">
                {PROVIDER_PRESETS.filter(p => p.category === "aggregator").map(preset => (
                  <option key={preset.name} value={preset.name}>
                    {preset.name} — {preset.baseUrl}
                  </option>
                ))}
              </optgroup>
              <optgroup label="官方原厂标准协议 (Official)">
                {PROVIDER_PRESETS.filter(p => p.category === "official").map(preset => (
                  <option key={preset.name} value={preset.name}>
                    {preset.name} — {preset.baseUrl}
                  </option>
                ))}
              </optgroup>
</select>
            </div>
          )}
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
              <SectionLabel>{t("newapi.defaultProtocol")}</SectionLabel>
              <div className="space-y-1.5">
                <Label htmlFor="newapi-provider-protocol" className="text-[13px]">{t("newapi.defaultProtocol")}</Label>
                <Select value={draft.protocol} onValueChange={value => {
                  const protocol = value as ModelProtocol;
                  setDraft(current => ({
                    ...current,
                    protocol,
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
              key={`${open}:${provider?.id ?? "new"}`}
              models={models}
              providerName={draft.name}
              protocol={draft.protocol}
              defaultModel={draft.model}
              disabled={saving || fetchingModels}
              fetching={fetchingModels}
              fetchError={fetchError}
              resolutions={resolutions}
              onFetch={() => void fetchModels()}
              onModelsChange={setModels}
              onDefaultModelChange={model => update("model", model)}
            />
          </div>

          {/* Advanced model settings */}
          <div className="space-y-2.5 pt-1 border-t border-border/50">
            <button
              type="button"
              onClick={() => setShowAdvanced(v => !v)}
              aria-expanded={showAdvanced}
              className="flex w-full items-center justify-between py-1 text-left text-muted-foreground transition-colors hover:text-foreground"
            >
              <div className="flex items-center gap-2">
                <span className="text-[11px] font-semibold uppercase tracking-wider text-primary/80">
                  {t("newapi.sectionAdvanced")}
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
                <div className="space-y-1.5">
                  <Label htmlFor="newapi-provider-compression" className="text-[13px]">{t("newapi.compression.providerLevel")}</Label>
                  <div className="flex gap-2">
                    <Select
                      value={draft.compressionMode}
                      disabled={saving}
                      onValueChange={value => setDraft(current => ({ ...current, compressionMode: value as ProviderDraft["compressionMode"] }))}
                    >
                      <SelectTrigger id="newapi-provider-compression" className="min-w-0 flex-1">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="inherit">{t("newapi.compression.inherit")}</SelectItem>
                        <SelectItem value="percent">{t("newapi.compression.percent")}</SelectItem>
                        <SelectItem value="tokens">{t("newapi.compression.tokens")}</SelectItem>
                        <SelectItem value="clientDefault">{t("newapi.compression.clientDefault")}</SelectItem>
                      </SelectContent>
                    </Select>
                    {(draft.compressionMode === "percent" || draft.compressionMode === "tokens") && (
                      <>
                        <Input
                          className="w-24"
                          inputMode="numeric"
                          autoComplete="off"
                          spellCheck={false}
                          disabled={saving}
                          value={draft.compressionValue}
                          placeholder={draft.compressionMode === "percent" ? "90" : "180000"}
                          aria-label={t("newapi.compression.providerLevel")}
                          onChange={event => update("compressionValue", event.target.value)}
                        />
                        <span className="shrink-0 text-[10px] text-muted-foreground">
                          {t(draft.compressionMode === "percent" ? "newapi.percentUnit" : "newapi.tokenUnit")}
                        </span>
                      </>
                    )}
                  </div>
                  <p className="text-[11px] leading-relaxed text-muted-foreground">{t("newapi.compression.providerHint")}</p>
                </div>
                <div className="space-y-1.5 sm:col-span-2">
                  <Label htmlFor="newapi-provider-catalog" className="text-[13px]">{t("newapi.catalogMapping.title")}</Label>
                  <Input
                    id="newapi-provider-catalog"
                    value={draft.catalogProviders}
                    disabled={saving}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder={t("newapi.catalogMapping.placeholder")}
                    onChange={event => update("catalogProviders", event.target.value)}
                  />
                  <p className="text-[11px] leading-relaxed text-muted-foreground">{t("newapi.catalogMapping.description")}</p>
                </div>
                <div className="space-y-1.5">
                  <Label htmlFor="newapi-provider-price-catalog" className="text-[13px]">{t("newapi.catalogMapping.priceTitle")}</Label>
                  <Input
                    id="newapi-provider-price-catalog"
                    value={draft.priceCatalogProvider}
                    disabled={saving}
                    autoComplete="off"
                    spellCheck={false}
                    placeholder={t("newapi.catalogMapping.placeholder")}
                    onChange={event => update("priceCatalogProvider", event.target.value)}
                  />
                  <p className="text-[11px] leading-relaxed text-muted-foreground">{t("newapi.catalogMapping.priceHint")}</p>
                </div>
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
              <Button type="submit" size="sm" disabled={saving || !draft.name.trim() || !draft.baseUrl.trim() || (!!draft.model && !models.some(model => model.id === draft.model && model.included))}>
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
