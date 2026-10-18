import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Check, ChevronDown, Eye, EyeOff, Loader2, Plug } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { toast } from "@/hooks/use-toast";
import { mergeModelCatalog } from "@/lib/provider-models";
import type { CompressionPolicy, ModelProtocol, NewApiProvider, NewApiStateChange, NewApiModel, SearchProbeReport } from "@/types/newapi";
import { ProviderModelEditor } from "@/components/newapi/provider-model-editor";
import { useModelResolutions } from "@/components/newapi/use-model-resolutions";
import { ProviderPresetGallery } from "@/components/newapi/provider-preset-gallery";

// A relay usually speaks OpenAI-compatible Chat Completions unless it says
// otherwise, and the Codex-specific Responses protocol is the less common case.
// Fetching models refines this from the capabilities the relay advertises.
const DEFAULT_PROTOCOL: ModelProtocol = "chatCompletions";
interface ProviderDraft {
  name: string;
  baseUrl: string;
  apiKey: string;
  protocol: ModelProtocol;
  chatSearch: NonNullable<NewApiProvider["chatSearch"]>;
  model: string;
  contextWindow: string;
  compressionMode: "inherit" | "percent" | "tokens" | "clientDefault";
  compressionValue: string;
  catalogProviders: string;
  // Preserve legacy bindings on unrelated edits; new prices are automatic.
  priceCatalogProvider: string;
  priceMultiplier: string;
  /** Gateway short name; empty keeps the stored one or derives one from the name. */
  gatewaySlug: string;
}
const emptyDraft: ProviderDraft = {
  name: "",
  baseUrl: "",
  apiKey: "",
  protocol: DEFAULT_PROTOCOL,
  chatSearch: "auto",
  model: "",
  contextWindow: "",
  compressionMode: "inherit",
  compressionValue: "",
  catalogProviders: "",
  priceCatalogProvider: "",
  priceMultiplier: "1",
  gatewaySlug: "",
};
const fromProvider = (provider: NewApiProvider): ProviderDraft => {
  const compression = provider.compression ?? { mode: "inherit" as const };
  return {
    name: provider.name,
    baseUrl: provider.baseUrl,
    apiKey: "",
    protocol: provider.protocol,
    chatSearch: provider.chatSearch ?? "auto",
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
    priceMultiplier: String(provider.priceMultiplier ?? 1),
    gatewaySlug: provider.gatewaySlug ?? "",
  };
};
function optionalNumber(text: string, errorKey: string) {
  if (!text.trim()) return null;
  const value = Number(text);
  if (!/^\d+$/.test(text) || !Number.isSafeInteger(value) || value <= 0) throw new Error(errorKey);
  return value;
}

function priceMultiplier(text: string) {
  const value = text.trim() ? Number(text) : 1;
  if (!Number.isFinite(value) || value < 0 || value > 1000) throw new Error("newapi.errors.priceMultiplier");
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
        // WebView2 adds its own reveal eye to password fields; this one has a toggle already.
        className="pr-9 [&::-ms-reveal]:hidden"
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
    <div className="flex items-center gap-2.5">
      <span className="font-display text-sm font-semibold tracking-tight text-foreground">{children}</span>
      <span aria-hidden className="h-px flex-1 bg-gradient-to-r from-border/80 to-transparent" />
    </div>
  );
}

const PROTOCOLS: ModelProtocol[] = ["responses", "chatCompletions", "anthropicMessages"];
const protocolCode: Record<ModelProtocol, string> = { responses: "RSP", chatCompletions: "CHT", anthropicMessages: "ANT" };

/** The relay's default protocol as three described choices, arrow-key navigable. */
function ProtocolPicker({ value, disabled, onChange }: {
  value: ModelProtocol;
  disabled?: boolean;
  onChange: (protocol: ModelProtocol) => void;
}) {
  const { t } = useTranslation();
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);
  const move = (index: number) => {
    const next = (index + PROTOCOLS.length) % PROTOCOLS.length;
    onChange(PROTOCOLS[next]);
    buttons.current[next]?.focus();
  };
  return (
    <div role="radiogroup" id="newapi-provider-protocol" aria-label={t("newapi.defaultProtocol")} className="grid gap-2">
      {PROTOCOLS.map((option, index) => {
        const checked = value === option;
        return (
          <button key={option} ref={node => { buttons.current[index] = node; }} type="button" role="radio"
            aria-checked={checked} aria-label={t(protocolLabelKey[option])} tabIndex={checked ? 0 : -1} disabled={disabled}
            onClick={() => onChange(option)}
            onKeyDown={event => {
              if (event.key === "ArrowDown" || event.key === "ArrowRight") { event.preventDefault(); move(index + 1); }
              if (event.key === "ArrowUp" || event.key === "ArrowLeft") { event.preventDefault(); move(index - 1); }
            }}
            className={cn("protocol-card", checked && "protocol-card-on")}>
            <span aria-hidden className="protocol-code">{protocolCode[option]}</span>
            <span className="min-w-0 flex-1 text-left">
              <span className="block truncate text-[13px] font-semibold">{t(protocolLabelKey[option])}</span>
              <span className="block text-[11px] leading-snug text-muted-foreground">{t(`newapi.protocolHint.${option}`)}</span>
            </span>
            <span aria-hidden className="protocol-check">{checked && <Check className="h-3 w-3" strokeWidth={3} />}</span>
          </button>
        );
      })}
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
  const [preset, setPreset] = useState<string | null>(null);
  const [models, setModels] = useState<NewApiModel[]>([]);
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  const [searchProbe, setSearchProbe] = useState<{ running: boolean; report?: SearchProbeReport; error?: string }>({ running: false });
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
      setPreset(null);
      setError(null);
      setSearchProbe({ running: false });
      setShowAdvanced(!!provider && (!!provider.userId || provider.hasAccessToken || !!provider.contextWindow || (provider.compression?.mode ?? "inherit") !== "inherit" || !!provider.catalogProviders?.length || (provider.priceMultiplier ?? 1) !== 1));
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
        chatSearch: draft.chatSearch,
        models,
        contextWindow: optionalNumber(draft.contextWindow, "newapi.errors.context"),
        compression,
        catalogProviders: draft.catalogProviders
          .split(/[,\s]+/)
          .map(value => value.trim().toLowerCase())
          .filter(Boolean),
        priceCatalogProvider: draft.priceCatalogProvider.trim().toLowerCase() || null,
        priceMultiplier: priceMultiplier(draft.priceMultiplier),
        gatewaySlug: draft.gatewaySlug.trim() || null,
      });
      onSaved(change);
      onOpenChange(false);
      toast({ title: t("newapi.saved"), variant: "success" });
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setError(key);
      if (key === "newapi.errors.context" || key === "newapi.errors.priceMultiplier") setShowAdvanced(true);
    } finally {
      setSaving(false);
    }
  };
  const hasAdvancedConfigured =
    draft.chatSearch !== "auto"
    || !!draft.contextWindow
    || draft.compressionMode !== "inherit"
    || Number(draft.priceMultiplier || 1) !== 1
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
  const host = (() => { try { return new URL(draft.baseUrl).host; } catch { return ""; } })();
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(92vh,940px)] flex-col gap-0 overflow-hidden p-0 sm:max-w-5xl">
        <div className="route-surface relative shrink-0 overflow-hidden border-b border-border/60 px-6 pb-5 pt-6">
          <div className="relative flex flex-wrap items-end justify-between gap-3 pr-8">
            <div className="flex min-w-0 items-center gap-3.5">
              <div className="flex h-12 w-12 shrink-0 items-center justify-center rounded-2xl bg-foreground text-background">
                <Plug className="h-5 w-5" />
              </div>
              <DialogHeader className="min-w-0 space-y-0.5 text-left">
                <span className="font-display text-[11px] font-semibold uppercase tracking-[0.22em] text-primary">
                  {t(provider ? "newapi.dialogEyebrowEdit" : "newapi.dialogEyebrowAdd")}
                </span>
                <DialogTitle className="poster-title text-2xl">{t(provider ? "newapi.dialogTitleEdit" : "newapi.dialogTitleAdd")}</DialogTitle>
                <DialogDescription className="sr-only">{t(provider ? "newapi.dialogTitleEdit" : "newapi.dialogTitleAdd")}</DialogDescription>
                <p className="truncate font-mono text-xs text-muted-foreground">{host || t("newapi.dialogHostPending")}</p>
              </DialogHeader>
            </div>
            <span className="inline-flex items-center gap-1.5 rounded-full bg-background/80 px-3 py-1 text-xs font-medium text-primary ring-1 ring-inset ring-primary/25 backdrop-blur">
              <span aria-hidden className="relay-live-dot" />
              {t(protocolLabelKey[draft.protocol])}
            </span>
          </div>
        </div>

        <form className="flex min-h-0 flex-1 flex-col" onSubmit={e => { e.preventDefault(); void submit(); }}>
          <div className="min-h-0 flex-1 space-y-6 overflow-y-auto px-6 py-5">
            {!provider && (
              <ProviderPresetGallery selected={preset} disabled={saving}
                onCustom={() => { setPreset(null); setDraft(emptyDraft); setModels([]); setFetchError(null); }}
                onPick={item => {
                  setPreset(item.name);
                  setDraft({ ...emptyDraft, name: item.name, baseUrl: item.baseUrl, protocol: item.protocol, model: item.models?.[0]?.id ?? "" });
                  setModels(item.models ?? []);
                  setFetchError(null);
                }} />
            )}
            {error && (
              <div role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-2.5 text-[13px] text-destructive">
                {t(error)}
              </div>
            )}

            <div className="grid gap-5 lg:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)]">
              <section className="space-y-3 rounded-2xl border border-border/70 bg-muted/20 p-4">
                <SectionLabel>{t("newapi.sectionConnection")}</SectionLabel>
                {field("name", "newapi.name", { placeholder: t("newapi.namePlaceholder") })}
                {field("gatewaySlug", "newapi.gatewaySlug", { placeholder: "deepseek", hint: t("newapi.gatewaySlugHint") })}
                {field("baseUrl", "newapi.url", { placeholder: "https://example.com" })}
                {field("apiKey", "newapi.apiKey", {
                  password: true,
                  placeholder: provider?.hasApiKey ? t("newapi.keepSecret") : "sk-…",
                })}
              </section>
              <section className="space-y-3 rounded-2xl border border-border/70 bg-muted/20 p-4">
                <SectionLabel>{t("newapi.defaultProtocol")}</SectionLabel>
                <ProtocolPicker value={draft.protocol} disabled={saving}
                  onChange={protocol => setDraft(current => ({ ...current, protocol }))} />
              </section>
            </div>

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

            {/* Advanced model settings */}
            <div className="space-y-2.5 rounded-2xl border border-border/70 p-3">
              <button
                type="button"
                onClick={() => setShowAdvanced(v => !v)}
                aria-expanded={showAdvanced}
                className="flex w-full items-center justify-between px-1 py-0.5 text-left text-muted-foreground transition-colors hover:text-foreground"
              >
                <div className="flex items-center gap-2">
                  <span className="font-display text-sm font-semibold tracking-tight text-foreground">
                    {t("newapi.sectionAdvanced")}
                  </span>
                  {!showAdvanced && hasAdvancedConfigured && (
                    <span className="rounded-full bg-emerald-500/15 px-2 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-400">
                      {t("newapi.optionalConfigured")}
                    </span>
                  )}
                </div>
                <ChevronDown className={cn("h-4 w-4 shrink-0 text-muted-foreground transition-transform duration-200", showAdvanced && "rotate-180")} />
              </button>

              {showAdvanced && (
                <div className="grid gap-3 rounded-xl bg-muted/25 p-3 sm:grid-cols-2">
                  <div className="space-y-1.5 sm:col-span-2">
                    <Label htmlFor="newapi-provider-chat-search" className="text-[13px]">{t("newapi.chatSearch.title")}</Label>
                    <Select value={draft.chatSearch} disabled={saving}
                      onValueChange={value => setDraft(current => ({ ...current, chatSearch: value as ProviderDraft["chatSearch"] }))}>
                      <SelectTrigger id="newapi-provider-chat-search"><SelectValue /></SelectTrigger>
                      <SelectContent>
                        {(["auto", "openAi", "mimo", "selected", "command", "disabled"] as const).map(mode =>
                          <SelectItem key={mode} value={mode}>{t(`newapi.chatSearch.${mode}`)}</SelectItem>)}
                      </SelectContent>
                    </Select>
                    <p className="text-xs text-muted-foreground">{t("newapi.chatSearch.hint")}</p>
                    <div className="flex flex-wrap items-center gap-2 pt-1">
                      <Button type="button" size="sm" variant="outline"
                        disabled={saving || searchProbe.running || !provider?.id || !!provider?.accountSource || !draft.model.trim()}
                        onClick={async () => {
                          if (!provider?.id) return;
                          setSearchProbe({ running: true });
                          try {
                            setSearchProbe({ running: false, report: await api.providers.probeSearch(provider.id, draft.model.trim()) });
                          } catch (cause) {
                            setSearchProbe({ running: false, error: t(String(cause), { defaultValue: String(cause) }) });
                          }
                        }}>
                        {searchProbe.running && <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden />}
                        {t(searchProbe.running ? "newapi.searchProbe.running" : "newapi.searchProbe.action")}
                      </Button>
                      <span className="text-xs text-muted-foreground">{t(provider?.id ? "newapi.searchProbe.hint" : "newapi.searchProbe.saveFirst")}</span>
                    </div>
                    {searchProbe.error && <p className="text-xs text-destructive" role="alert">{searchProbe.error}</p>}
                    {searchProbe.report && (
                      <div className="space-y-1 rounded-lg border border-border/60 bg-card/60 p-2 text-xs" aria-live="polite">
                        {searchProbe.report.attempts.map(attempt => (
                          <p key={attempt.format} className="min-w-0 break-words">
                            <span className="font-medium">{t("newapi.searchProbe.formats." + attempt.format)}</span>
                            {" · "}
                            <span className={attempt.outcome === "executed" ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground"}>
                              {t("newapi.searchProbe.outcomes." + attempt.outcome)}
                            </span>
                            {attempt.status !== null && " · HTTP " + attempt.status}
                            {attempt.sources > 0 && " · " + t("newapi.searchProbe.sources", { count: attempt.sources })}
                            {attempt.detail && <span className="block text-muted-foreground">{attempt.detail}</span>}
                          </p>
                        ))}
                        {searchProbe.report.suggestion && searchProbe.report.suggestion !== draft.chatSearch ? (
                          <Button type="button" size="sm" variant="secondary" className="mt-1"
                            onClick={() => setDraft(current => ({ ...current, chatSearch: searchProbe.report!.suggestion! }))}>
                            {t("newapi.searchProbe.apply", { mode: t("newapi.chatSearch." + searchProbe.report.suggestion) })}
                          </Button>
                        ) : !searchProbe.report.attempts.some(attempt => attempt.outcome === "executed") && (
                          <p className="text-muted-foreground">{t("newapi.searchProbe.none")}</p>
                        )}
                      </div>
                    )}
                  </div>
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
                          <span className="self-center shrink-0 text-[10px] text-muted-foreground">
                            {t(draft.compressionMode === "percent" ? "newapi.percentUnit" : "newapi.tokenUnit")}
                          </span>
                        </>
                      )}
                    </div>
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
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="newapi-provider-price-multiplier" className="text-[13px]">{t("newapi.priceMultiplier")}</Label>
                    <Input id="newapi-provider-price-multiplier" type="number" min={0} max={1000} step="any"
                      disabled={saving} value={draft.priceMultiplier} placeholder="1"
                      onChange={event => update("priceMultiplier", event.target.value)} />
                  </div>
                </div>
              )}
            </div>
          </div>

          <DialogFooter className="shrink-0 gap-2 border-t border-border/60 bg-card/95 px-6 py-3 backdrop-blur sm:space-x-0">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={saving}>
              {t("common.cancel")}
            </Button>
            <Button type="submit" className="min-w-24 rounded-xl" disabled={saving || !draft.name.trim() || !draft.baseUrl.trim() || (!!draft.model && !models.some(model => model.id === draft.model && model.included))}>
              {saving && <Loader2 className="mr-1.5 h-3.5 w-3.5 animate-spin" />}
              {t("newapi.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
