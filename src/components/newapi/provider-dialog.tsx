import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, Eye, EyeOff, Loader2, Plug, RefreshCw } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { toast } from "@/hooks/use-toast";
import type { NewApiProvider, NewApiStateChange, NewApiModel } from "@/types/newapi";

const emptyDraft = { name: "", baseUrl: "", userId: "", accessToken: "", apiKey: "", model: "", contextWindow: "", autoCompactLimit: "", wireApi: "responses" };
const fromProvider = (provider: NewApiProvider) => ({
  name: provider.name,
  baseUrl: provider.baseUrl,
  userId: provider.userId?.toString() ?? "",
  accessToken: "",
  apiKey: "",
  model: provider.model,
  contextWindow: provider.contextWindow?.toString() ?? "",
  autoCompactLimit: provider.autoCompactLimit?.toString() ?? "",
  wireApi: provider.wireApi || "responses",
});
const wireOf = (model: NewApiModel) =>
  model.endpointTypes.some(type => type.includes("response")) ? "responses" : "chat";
function optionalNumber(text: string, errorKey: string) {
  if (!text.trim()) return null;
  const value = Number(text);
  if (!/^\d+$/.test(text) || !Number.isSafeInteger(value) || value <= 0) throw new Error(errorKey);
  return value;
}

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

const OFFICIAL_PRESETS = [
  { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com", wire: "responses" },
  { id: "xiaomi-mimo", name: "小米 MiMo", baseUrl: "https://token-plan-cn.xiaomimimo.com/v1", wire: "chat" },
  { id: "opencode-go", name: "OpenCode Go", baseUrl: "https://opencode.ai/zen/go/v1", wire: "chat" },
  { id: "commandcode-goat", name: "Command Code GOAT", baseUrl: "https://api.commandcode.ai/provider/v1", wire: "chat" },
];

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
  const [showBalance, setShowBalance] = useState(false);
  const [models, setModels] = useState<NewApiModel[]>([]);
  const [modelWire, setModelWire] = useState<Record<string, string>>({});
  const [fetchingModels, setFetchingModels] = useState(false);
  const [fetchError, setFetchError] = useState<string | null>(null);
  useEffect(() => {
    if (open) {
      setDraft(provider ? fromProvider(provider) : emptyDraft);
      setError(null);
      setShowBalance(!!provider && (!!provider.userId || provider.hasAccessToken));
      setModels([]);
      setModelWire({});
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
      const change = await api.saveNewApiProvider({
        id: provider?.id ?? null,
        name: draft.name,
        baseUrl: draft.baseUrl,
        userId: optionalNumber(draft.userId, "newapi.errors.userId"),
        accessToken: draft.accessToken,
        apiKey: draft.apiKey,
        model: draft.model,
        contextWindow: optionalNumber(draft.contextWindow, "newapi.errors.context"),
        autoCompactLimit: optionalNumber(draft.autoCompactLimit, "newapi.errors.context"),
        wireApi: draft.wireApi,
      });
      onSaved(change);
      onOpenChange(false);
      toast({ title: t("newapi.saved"), variant: "success" });
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setError(key);
      if (key === "newapi.errors.userId") setShowBalance(true);
    } finally {
      setSaving(false);
    }
  };
  const balanceConfigured = !!provider && (!!provider.userId || provider.hasAccessToken);
  const fetchModels = async () => {
    if (fetchingModels || saving) return;
    if (!draft.baseUrl.trim()) {
      setFetchError("newapi.errors.url");
      return;
    }
    setFetchingModels(true);
    setFetchError(null);
    try {
      setModels(await api.fetchNewApiModels(provider?.id ?? null, draft.baseUrl.trim(), draft.apiKey));
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setFetchError(key.startsWith("newapi.") ? key : "newapi.errors.unexpected");
    } finally {
      setFetchingModels(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-md">
        <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 -m-6 mb-0 h-28 bg-gradient-to-b from-primary/[0.07] via-primary/[0.02] to-transparent" />
        <div className="relative flex items-start gap-3">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-primary/20 to-primary/5 ring-1 ring-inset ring-primary/15">
            <Plug className="h-5 w-5 text-primary" />
          </div>
          <DialogHeader className="min-w-0 flex-1 space-y-1.5 pt-0.5 text-left">
            <DialogTitle className="text-base">{t(provider ? "newapi.dialogTitleEdit" : "newapi.dialogTitleAdd")}</DialogTitle>
            <DialogDescription className="text-xs">{t("newapi.dialogSubtitle")}</DialogDescription>
          </DialogHeader>
        </div>
        <form className="relative space-y-3.5" onSubmit={e => { e.preventDefault(); void submit(); }}>
          {error && <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 p-2.5 text-[13px] text-destructive">{t(error)}</div>}
          <SectionLabel>{t("newapi.sectionThirdParty")}</SectionLabel>
          <div className="grid gap-3 sm:grid-cols-2">
            {field("name", "newapi.name", { placeholder: t("newapi.namePlaceholder") })}
            {field("baseUrl", "newapi.url", { placeholder: "https://example.com" })}
          </div>
          {field("apiKey", "newapi.apiKey", { password: true, placeholder: provider?.hasApiKey ? t("newapi.keepSecret") : "sk-…", hint: t("newapi.apiKeyHint") })}
          <div className="space-y-1.5">
            <div className="flex items-center justify-between gap-2">
              <Label htmlFor="newapi-provider-model" className="text-[13px]">{t("newapi.model")}</Label>
              <button
                type="button"
                onClick={() => void fetchModels()}
                disabled={fetchingModels || saving || !draft.baseUrl.trim()}
                className="flex items-center gap-1 rounded-md px-1.5 py-0.5 text-[11px] font-medium text-primary transition-colors hover:bg-primary/10 disabled:pointer-events-none disabled:opacity-40"
              >
                {fetchingModels ? <Loader2 className="h-3 w-3 animate-spin" /> : <RefreshCw className="h-3 w-3" />}
                {t("newapi.fetchModels")}
              </button>
            </div>
            <Input
              id="newapi-provider-model"
              type="text"
              autoComplete="off"
              spellCheck={false}
              value={draft.model}
              disabled={saving}
              onChange={e => update("model", e.target.value)}
            />
            {fetchError && <p className="text-[11px] text-destructive">{t(fetchError)}</p>}
            {models.length > 0 && (
              <div className="max-h-40 overflow-y-auto rounded-lg bg-black/[0.02] ring-1 ring-border/60 dark:bg-white/[0.02]">
                {models.map(model => {
                  const wire = modelWire[model.id] ?? wireOf(model);
                  return (
                    <div key={model.id} className={cn("flex items-center gap-2 px-2.5 py-1.5", draft.model === model.id && "bg-primary/10")}>
                      <button
                        type="button"
                        onClick={() => {
                          update("model", model.id);
                          update("wireApi", wire);
                        }}
                        className="flex min-w-0 flex-1 items-center justify-between gap-2 text-left font-mono text-[11px] transition-colors hover:text-primary"
                      >
                        <span className={cn("truncate", draft.model === model.id && "font-medium text-primary")}>{model.id}</span>
                        {typeof model.contextWindow === "number" && model.contextWindow > 0 && (
                          <span className="shrink-0 font-sans text-[10px] text-muted-foreground">{Math.round(model.contextWindow / 1000)}K</span>
                        )}
                      </button>
                      <Select
                        value={wire}
                        onValueChange={value => {
                          setModelWire(current => ({ ...current, [model.id]: value }));
                          if (draft.model === model.id) update("wireApi", value);
                        }}
                      >
                        <SelectTrigger className="h-6 w-[100px] shrink-0 rounded-md px-2 text-[10px]">
                          <SelectValue />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="responses">Responses</SelectItem>
                          <SelectItem value="chat">Chat</SelectItem>
                        </SelectContent>
                      </Select>
                    </div>
                  );
                })}
              </div>
            )}
            <p className="text-[11px] leading-relaxed text-muted-foreground">{t("newapi.modelHint")}</p>
          </div>
          <SectionLabel>{t("newapi.sectionOfficial")}</SectionLabel>
          <div className="flex flex-wrap gap-2">
            {OFFICIAL_PRESETS.map(preset => {
              const picked = draft.baseUrl.trim().replace(/\/+$/, "") === preset.baseUrl.replace(/\/+$/, "");
              return (
                <button
                  key={preset.id}
                  type="button"
                  disabled={saving}
                  onClick={() => {
                    update("baseUrl", preset.baseUrl);
                    update("wireApi", preset.wire);
                    if (!draft.name.trim()) update("name", preset.name);
                  }}
                  className={cn(
                    "rounded-lg border bg-card px-2.5 py-1 text-[12px] font-medium shadow-sm transition-all",
                    picked
                      ? "border-primary/50 bg-primary/[0.06] text-primary ring-1 ring-primary/15"
                      : "text-foreground/80 hover:-translate-y-px hover:bg-muted/60",
                  )}
                >
                  {preset.name}
                </button>
              );
            })}
          </div>
          <p className="text-[11px] leading-relaxed text-muted-foreground">{t("newapi.officialHint")}</p>
          <SectionLabel>{t("newapi.sectionAdvanced")}</SectionLabel>
          <div className="grid gap-3 sm:grid-cols-2">
            {field("contextWindow", "newapi.contextWindow", { numeric: true, placeholder: t("newapi.useDefault") })}
            {field("autoCompactLimit", "newapi.autoCompact", { numeric: true, placeholder: t("newapi.useDefault") })}
          </div>
          <button
            type="button"
            onClick={() => setShowBalance(v => !v)}
            aria-expanded={showBalance}
            className="flex w-full items-center gap-2.5 pt-1 text-left"
          >
            <span className="text-[11px] font-semibold uppercase tracking-wider text-primary/80">{t("newapi.sectionBalance")}</span>
            {!showBalance && balanceConfigured && (
              <span className="rounded-full bg-emerald-500/15 px-2 py-px text-[10px] font-medium text-emerald-700 dark:text-emerald-400">
                {t("newapi.optionalConfigured")}
              </span>
            )}
            <span aria-hidden className="h-px flex-1 bg-gradient-to-r from-border/80 to-transparent" />
            <ChevronDown className={cn("h-3.5 w-3.5 shrink-0 text-muted-foreground transition-transform", showBalance && "rotate-180")} />
          </button>
          {showBalance && (
            <div className="grid gap-3 sm:grid-cols-2">
              {field("userId", "newapi.userId", { numeric: true, hint: t("newapi.userIdHint") })}
              {field("accessToken", "newapi.accessToken", { password: true, placeholder: provider?.hasAccessToken ? t("newapi.keepSecret") : "", hint: t("newapi.accessTokenHint") })}
            </div>
          )}
          <p className="rounded-lg bg-black/[0.03] px-3 py-2 text-[11px] leading-relaxed text-muted-foreground ring-1 ring-border/60 dark:bg-white/[0.03]">{t("newapi.dialogTips")}</p>
          <DialogFooter className="gap-2 pt-1">
            <Button type="button" variant="ghost" onClick={() => onOpenChange(false)} disabled={saving}>{t("common.cancel")}</Button>
            <Button type="submit" disabled={saving || !draft.name.trim() || !draft.baseUrl.trim()}>
              {saving && <Loader2 className="h-4 w-4 animate-spin" />}
              {t("newapi.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
