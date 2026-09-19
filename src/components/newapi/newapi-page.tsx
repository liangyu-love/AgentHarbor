import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Download, Eye, Loader2, RotateCw, Save, Trash2 } from "lucide-react";
import { api } from "@/lib/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { toast } from "@/hooks/use-toast";
import type { NewApiBalance, NewApiConnection, NewApiModel, NewApiAppliedConfig } from "@/types/newapi";
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from "@/components/ui/alert-dialog";

const emptyDraft = { baseUrl: "", userId: "", accessToken: "", apiKey: "", model: "", contextWindow: "", autoCompactLimit: "" };
const fromConnection = (c: NewApiConnection) => ({ ...emptyDraft, baseUrl: c.baseUrl, userId: c.userId?.toString() ?? "", model: c.model, contextWindow: c.contextWindow?.toString() ?? "", autoCompactLimit: c.autoCompactLimit?.toString() ?? "" });
function optionalNumber(text: string, errorKey: string) {
  if (!text.trim()) return null;
  const value = Number(text);
  if (!/^\d+$/.test(text) || !Number.isSafeInteger(value) || value <= 0) throw new Error(errorKey);
  return value;
}

export function NewApiPage() {
  const { t, i18n } = useTranslation();
  const [draft, setDraft] = useState(emptyDraft);
  const [saved, setSaved] = useState<NewApiConnection | null>(null);
  const [balance, setBalance] = useState<NewApiBalance | null>(null);
  const [models, setModels] = useState<NewApiModel[] | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [applied, setApplied] = useState<NewApiAppliedConfig | null>(null);
  const [busy, setBusy] = useState<string | null>("load");
  const [error, setError] = useState<string | null>(null);
  const [balanceError, setBalanceError] = useState(false);
  const [writeApiKey, setWriteApiKey] = useState(false);
  const [confirm, setConfirm] = useState<"apply" | "forget" | null>(null);
  const running = useRef(false);
  const errorRef = useRef<HTMLDivElement>(null);
  useEffect(() => { if (error) errorRef.current?.scrollIntoView({ block: "nearest" }); }, [error]);
  useEffect(() => {
    let active = true;
    api.loadNewApiConnection().then(c => { if (active) { setSaved(c); setDraft(fromConnection(c)); } })
      .catch(() => { if (active) setError("newapi.errors.storage"); })
      .finally(() => { if (active) setBusy(null); });
    return () => { active = false; };
  }, []);
  const connectionDirty = !saved || draft.baseUrl !== saved.baseUrl || draft.userId !== (saved.userId?.toString() ?? "") || !!draft.accessToken || !!draft.apiKey;
  const chosen = models?.find(m => m.id === draft.model);
  const modelDirty = !saved || draft.model !== saved.model || draft.contextWindow !== (saved.contextWindow?.toString() ?? "") || draft.autoCompactLimit !== (saved.autoCompactLimit?.toString() ?? "");
  const update = (key: keyof typeof draft, value: string) => { setDraft(d => ({ ...d, [key]: value })); setPreview(null); setApplied(null); };
  const run = async (action: string, fn: () => Promise<void>) => {
    if (running.current || busy === "load") return;
    running.current = true; setBusy(action); setError(null);
    try { await fn(); } catch (e) {
      const key = e instanceof Error ? e.message : String(e);
      setError(key.startsWith("newapi.errors.") ? key : "newapi.errors.unexpected");
      if (action === "balance") setBalanceError(true);
    } finally { running.current = false; setBusy(null); }
  };
  const persist = async () => {
    const result = await api.saveNewApiConnection({ ...draft, userId: optionalNumber(draft.userId, "newapi.errors.userId"), contextWindow: optionalNumber(draft.contextWindow, "newapi.errors.context"), autoCompactLimit: optionalNumber(draft.autoCompactLimit, "newapi.errors.context") });
    if (connectionDirty) { setBalance(null); setModels(null); setBalanceError(false); }
    setSaved(result); setDraft(fromConnection(result)); setPreview(null); setApplied(null);
    return result;
  };
  const number = (value: number | null | undefined) => value == null ? "—" : new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 4 }).format(value);
  const actionIcon = (action: string, icon: React.ReactNode) => busy === action ? <Loader2 className="h-4 w-4 animate-spin" /> : icon;
  const field = (key: keyof typeof draft, label: string, options: { password?: boolean; hint?: string; placeholder?: string; numeric?: boolean } = {}) => (
    <div className="space-y-2">
      <Label htmlFor={`newapi-${key}`}>{t(label)}</Label>
      <Input id={`newapi-${key}`} type={options.password ? "password" : "text"} inputMode={options.numeric ? "numeric" : undefined}
        autoComplete="off" spellCheck={false} value={draft[key]} disabled={!!busy} placeholder={options.placeholder}
        onChange={e => update(key, e.target.value)} aria-describedby={options.hint ? `newapi-${key}-hint` : undefined} />
      {options.hint && <p id={`newapi-${key}-hint`} className="text-xs leading-relaxed text-muted-foreground">{options.hint}</p>}
    </div>
  );
  return (
    <div className="space-y-6">
      <div><h1 className="text-xl font-semibold">{t("newapi.title")}</h1><p className="mt-1 text-sm text-muted-foreground">{t("newapi.description")}</p></div>
      {error && <div ref={errorRef} role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 text-sm text-destructive">{t(error)}</div>}
      <section className="rounded-xl border bg-card p-5" aria-labelledby="newapi-connection-title">
        <h2 id="newapi-connection-title" className="mb-4 font-semibold">{t("newapi.connection")}</h2>
        <form onSubmit={e => { e.preventDefault(); void run("save", async () => { await persist(); toast({ title: t("newapi.saved"), variant: "success" }); }); }}>
          <div className="grid gap-5 md:grid-cols-2">
            {field("baseUrl", "newapi.url", { placeholder: "https://example.com", hint: t("newapi.urlHint") })}
            {field("userId", "newapi.userId", { numeric: true, hint: t("newapi.userIdHint") })}
            {field("accessToken", "newapi.accessToken", { password: true, placeholder: saved?.hasAccessToken && draft.baseUrl === saved.baseUrl && draft.userId === saved.userId?.toString() ? t("newapi.keepSecret") : "", hint: t("newapi.accessTokenHint") })}
            {field("apiKey", "newapi.apiKey", { password: true, placeholder: saved?.hasApiKey && draft.baseUrl === saved.baseUrl ? t("newapi.keepSecret") : "sk-…", hint: t("newapi.apiKeyHint") })}
          </div>
          <div className="mt-5 flex flex-wrap items-center gap-3">
            <Button type="submit" disabled={!!busy || !draft.baseUrl.trim()}>{actionIcon("save", <Save className="h-4 w-4" />)}{t("newapi.save")}</Button>
            <Button type="button" variant="ghost" disabled={!!busy || !saved?.baseUrl} onClick={() => setConfirm("forget")}><Trash2 className="h-4 w-4" />{t("newapi.forget")}</Button>
            {(connectionDirty && !!saved?.baseUrl) && <span className="text-xs text-muted-foreground">{t("newapi.unsaved")}</span>}
          </div>
          <p className="mt-3 text-xs leading-relaxed text-muted-foreground">{t("newapi.storageHint")}</p>
        </form>
      </section>
      <section className="rounded-xl border bg-card p-5" aria-labelledby="newapi-balance-title">
        <div className="flex flex-wrap items-center justify-between gap-3"><h2 id="newapi-balance-title" className="font-semibold">{t("newapi.balance")}</h2>
          <Button size="sm" variant="outline" disabled={!!busy || connectionDirty || !saved?.hasAccessToken || !saved?.userId} onClick={() => void run("balance", async () => { setBalance(await api.queryNewApiBalance()); setBalanceError(false); })}>{actionIcon("balance", <RotateCw className="h-4 w-4" />)}{t("newapi.queryBalance")}</Button></div>
        {balance ? <>
          <dl className="mt-5 grid gap-5 sm:grid-cols-3">
            <div><dt className="text-sm text-muted-foreground">{t("newapi.remaining")}</dt><dd className="mt-2 text-2xl font-semibold tabular-nums">{number(balance.remaining)} <span className="text-sm font-normal">{balance.unit === "quota" ? t("newapi.quota") : balance.unit}</span></dd></div>
            <div><dt className="text-sm text-muted-foreground">{t("newapi.used")}</dt><dd className="mt-2 text-2xl font-semibold tabular-nums">{number(balance.used)}</dd></div>
            <div><dt className="text-sm text-muted-foreground">{t("newapi.requests")}</dt><dd className="mt-2 text-2xl font-semibold tabular-nums">{number(balance.requestCount)}</dd></div>
          </dl>
          <p className="mt-5 text-xs text-muted-foreground">{t(balanceError ? "newapi.previousResult" : "newapi.checkedAt")} {new Date(balance.checkedAt * 1000).toLocaleString(i18n.language)}</p>
          {!balance.conversionAvailable && <p className="mt-2 text-xs text-muted-foreground">{t("newapi.rawQuotaHint")}</p>}
        </> : <p className="mt-4 text-sm text-muted-foreground">{t("newapi.balanceEmpty")}</p>}
        <p className="mt-2 text-xs text-muted-foreground">{t("newapi.balanceHint")}</p>
      </section>
      <section className="rounded-xl border bg-card p-5" aria-labelledby="newapi-models-title">
        <div className="flex flex-wrap items-center justify-between gap-3"><h2 id="newapi-models-title" className="font-semibold">{t("newapi.models")}</h2>
          <Button size="sm" variant="outline" disabled={!!busy || connectionDirty || !saved?.hasApiKey} onClick={() => void run("models", async () => { setModels(await api.fetchNewApiModels()); })}>{actionIcon("models", <Download className="h-4 w-4" />)}{t("newapi.fetchModels")}</Button></div>
        {models !== null && <p className="mt-3 text-xs text-muted-foreground">{t("newapi.modelCount", { count: models.length })}</p>}
        <div className="mt-5 grid gap-5 md:grid-cols-2">
          <div className="space-y-2">
            <Label htmlFor="newapi-model">{t("newapi.model")}</Label>
            <select id="newapi-model" className="flex h-10 w-full min-w-0 rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50" value={draft.model} disabled={!!busy || !models?.length} onChange={e => { update("model", e.target.value); update("contextWindow", ""); update("autoCompactLimit", ""); }}>
              <option value="">{t("newapi.selectModel")}</option>
              {draft.model && !models?.some(m => m.id === draft.model) && <option value={draft.model}>{draft.model}</option>}
              {models?.map(m => <option key={m.id} value={m.id}>{m.id}</option>)}
            </select>
            <p className="text-xs leading-relaxed text-muted-foreground">{t("newapi.responsesHint")}</p>
          </div>
          <div className="self-center text-xs leading-relaxed text-muted-foreground">
            <p>{t("newapi.reportedContext")}: {chosen?.contextWindow ? number(chosen.contextWindow) : t("newapi.notReported")}</p>
            <p className="mt-1">{t("newapi.reportedOutput")}: {chosen?.maxOutputTokens ? number(chosen.maxOutputTokens) : t("newapi.notReported")}</p>
            {!!chosen?.endpointTypes.length && <p className="mt-1 break-words">{t("newapi.endpoints")}: {chosen.endpointTypes.join(", ")}</p>}
          </div>
          {field("contextWindow", "newapi.contextWindow", { numeric: true, placeholder: t("newapi.useDefault"), hint: t("newapi.contextHint") })}
          {field("autoCompactLimit", "newapi.autoCompact", { numeric: true, placeholder: t("newapi.useDefault"), hint: t("newapi.compactHint") })}
        </div>
        <div className="mt-5 flex flex-wrap items-center gap-3">
          <Button variant="outline" disabled={!!busy || !draft.model || connectionDirty} onClick={() => void run("preview", async () => { await persist(); setPreview(await api.previewNewApiConfig()); })}>{actionIcon("preview", <Eye className="h-4 w-4" />)}{t("newapi.preview")}</Button>
          <Button disabled={!!busy || !preview || connectionDirty || modelDirty} onClick={() => setConfirm("apply")}><Save className="h-4 w-4" />{t("newapi.apply")}</Button>
        </div>
        {preview && <div className="mt-4 space-y-3">
          <pre className="overflow-x-auto rounded-lg bg-muted p-4 text-xs leading-relaxed" aria-label={t("newapi.configPreview")}>{writeApiKey ? preview.replace(/^env_key = "AIMAMI_NEWAPI_KEY"$/m, `experimental_bearer_token = ${JSON.stringify(t("newapi.redactedKey"))}`) : preview}</pre>
          <p className="text-xs leading-relaxed text-muted-foreground">{t(writeApiKey ? "newapi.keyHint" : "newapi.envHint")}</p>
          <div className="flex items-center gap-2"><Checkbox id="newapi-write-key" checked={writeApiKey} disabled={!!busy || !saved?.hasApiKey} onCheckedChange={v => setWriteApiKey(v === true)} /><Label htmlFor="newapi-write-key" className="text-xs leading-relaxed">{t("newapi.writeKey")}</Label></div>
        </div>}
        {applied && <div role="status" className="mt-4 space-y-2 text-xs leading-relaxed"><p>{t("newapi.applied")}</p><p className="break-all">{applied.path}</p>{applied.backupPath && <p className="break-all">{t("newapi.backup")}: {applied.backupPath}</p>}</div>}
      </section>
      <AlertDialog open={confirm !== null} onOpenChange={open => { if (!open) setConfirm(null); }}>
        <AlertDialogContent><AlertDialogHeader><AlertDialogTitle>{t(confirm === "forget" ? "newapi.forget" : "newapi.apply")}</AlertDialogTitle><AlertDialogDescription>{t(confirm === "forget" ? "newapi.forgetConfirm" : writeApiKey ? "newapi.applyKeyConfirm" : "newapi.applyConfirm")}</AlertDialogDescription></AlertDialogHeader>
          <AlertDialogFooter><AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel><AlertDialogAction onClick={() => {
            const action = confirm; setConfirm(null);
            void run(action ?? "apply", async () => {
              if (action === "forget") { await api.forgetNewApiConnection(); setSaved(null); setDraft(emptyDraft); setBalance(null); setModels(null); setPreview(null); setApplied(null); setWriteApiKey(false); }
              else { setApplied(await api.applyNewApiConfig(writeApiKey)); }
            });
          }}>{t("common.confirm")}</AlertDialogAction></AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
