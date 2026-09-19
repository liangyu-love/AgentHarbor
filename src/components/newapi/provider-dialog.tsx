import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Eye, EyeOff, Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { toast } from "@/hooks/use-toast";
import type { NewApiProvider, NewApiStateChange } from "@/types/newapi";

const emptyDraft = { name: "", baseUrl: "", userId: "", accessToken: "", apiKey: "", model: "", contextWindow: "", autoCompactLimit: "" };
const fromProvider = (provider: NewApiProvider) => ({
  name: provider.name,
  baseUrl: provider.baseUrl,
  userId: provider.userId?.toString() ?? "",
  accessToken: "",
  apiKey: "",
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
  useEffect(() => {
    if (open) {
      setDraft(provider ? fromProvider(provider) : emptyDraft);
      setError(null);
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
      });
      onSaved(change);
      onOpenChange(false);
      toast({ title: t("newapi.saved"), variant: "success" });
    } catch (e) {
      const key = typeof e === "string" ? e : e instanceof Error ? e.message : String(e);
      setError(key);
    } finally {
      setSaving(false);
    }
  };
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto sm:max-w-md">
        <DialogHeader className="space-y-1.5">
          <DialogTitle className="text-base">{t(provider ? "newapi.dialogTitleEdit" : "newapi.dialogTitleAdd")}</DialogTitle>
          <DialogDescription className="text-xs">{t("newapi.dialogSubtitle")}</DialogDescription>
        </DialogHeader>
        <form className="space-y-3.5" onSubmit={e => { e.preventDefault(); void submit(); }}>
          {error && <div role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 p-2.5 text-[13px] text-destructive">{t(error)}</div>}
          <div className="grid gap-3 sm:grid-cols-2">
            {field("name", "newapi.name", { placeholder: t("newapi.namePlaceholder") })}
            {field("baseUrl", "newapi.url", { placeholder: "https://example.com" })}
          </div>
          {field("apiKey", "newapi.apiKey", { password: true, placeholder: provider?.hasApiKey ? t("newapi.keepSecret") : "sk-…", hint: t("newapi.apiKeyHint") })}
          {field("model", "newapi.model", { list: "newapi-provider-models", hint: t("newapi.modelHint") })}
          <datalist id="newapi-provider-models">
            {provider?.models.map(model => <option key={model.id} value={model.id} />)}
          </datalist>
          <div className="grid gap-3 sm:grid-cols-2">
            {field("userId", "newapi.userId", { numeric: true, hint: t("newapi.userIdHint") })}
            {field("accessToken", "newapi.accessToken", { password: true, placeholder: provider?.hasAccessToken ? t("newapi.keepSecret") : "" })}
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            {field("contextWindow", "newapi.contextWindow", { numeric: true, placeholder: t("newapi.useDefault") })}
            {field("autoCompactLimit", "newapi.autoCompact", { numeric: true, placeholder: t("newapi.useDefault") })}
          </div>
          <p className="rounded-lg bg-muted/50 px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">{t("newapi.dialogTips")}</p>
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
