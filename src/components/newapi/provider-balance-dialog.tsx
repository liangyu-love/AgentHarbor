import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, RefreshCw, Wallet, Code2, KeyRound, AlertTriangle, ShieldCheck } from "lucide-react";
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { isOfficialBalanceSite, isMimoBalanceSite, balanceSourceName, BALANCE_SCRIPT_TEMPLATE } from "@/lib/provider-balance";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import type { NewApiProvider, NewApiBalance } from "@/types/newapi";
import type { ProviderState } from "./use-provider-state";
import { relayGroupKey } from "./relay-groups";
import { CommandCodeBalanceSummary } from "./command-code-balance-summary";

export function ProviderBalanceDialog({ provider, store, onClose }: {
  provider: NewApiProvider; store: ProviderState; onClose: () => void;
}) {
  const { t, i18n } = useTranslation();
  const [userId, setUserId] = useState(provider.userId?.toString() ?? "");
  const [token, setToken] = useState("");
  const [script, setScript] = useState(provider.balanceScript ?? "");
  const [custom, setCustom] = useState(!!provider.balanceScript);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const started = useRef(false);
  const saveInFlight = useRef(false);
  const initialUrl = useRef(provider.baseUrl);
  const official = isOfficialBalanceSite(provider.baseUrl);
  const mimo = isMimoBalanceSite(provider.baseUrl);
  const firstSiteCard = store.state.data?.providers.find(item => relayGroupKey(item.baseUrl) === relayGroupKey(provider.baseUrl));
  const canShare = !!firstSiteCard && firstSiteCard.id !== provider.id;
  const shared = !!provider.balanceSharedFrom;
  const effectiveScript = shared ? firstSiteCard?.balanceScript : provider.balanceScript;
  const balanceCredentialReady = shared
    ? !!firstSiteCard && (firstSiteCard.hasApiKey || firstSiteCard.hasAccessToken || !!firstSiteCard.balanceScript)
    : provider.hasApiKey || provider.hasAccessToken || !!provider.balanceScript;
  const busy = saving || store.balanceBusy.has(provider.id);
  const error = saveError ?? (store.balanceErrors[provider.id] ? t(store.balanceErrors[provider.id]!) : null);
  const balance = provider.balance;
  const commandCode = provider.balanceSource === "commandCode" && !effectiveScript && (() => {
    try { return new URL(provider.baseUrl).hostname.toLowerCase() === "api.commandcode.ai"; }
    catch { return false; }
  })();
  const dirty = (custom ? script : "") !== (provider.balanceScript ?? "");
  const entries: NonNullable<NewApiBalance["entries"]> = balance?.entries?.length ? balance.entries : balance ? [{
    remaining: balance.conversionAvailable ? balance.remaining : balance.rawQuota,
    used: balance.used, total: null, unit: balance.unit, valid: balance.remaining > 0,
  }] : [];
  const number = (value: number) => new Intl.NumberFormat(i18n.language, { maximumFractionDigits: 4 }).format(value);

  useEffect(() => {
    if (started.current) return;
    started.current = true;
    if (provider.balanceScript || (!mimo && (provider.hasAccessToken || (official && balanceCredentialReady)))) void store.refreshBalance(provider.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [provider.id, provider.hasAccessToken, balanceCredentialReady, official, mimo, effectiveScript]);

  const saveAndQuery = async () => {
    if (saveInFlight.current || busy) return;
    saveInFlight.current = true;
    setSaving(true);
    setSaveError(null);
    try {
      const text = userId.trim();
      const id = text ? Number(text) : null;
      if (id !== null && (!/^\d+$/.test(text) || !Number.isSafeInteger(id) || id <= 0)) throw new Error("newapi.errors.userId");
      const next = await api.providers.saveBalanceCredentials(provider.id, initialUrl.current, id, token);
      store.patchBalanceCredentials(next);
      setToken("");
      await store.refreshBalance(provider.id);
    } catch (cause) {
      setSaveError(formatInvokeError(cause));
    } finally {
      saveInFlight.current = false;
      setSaving(false);
    }
  };

  return (
    <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
      <DialogContent className={custom || commandCode ? "sm:max-w-2xl max-h-[90vh] overflow-y-auto" : "sm:max-w-lg max-h-[90vh] overflow-y-auto"}>
        <div aria-hidden className="pointer-events-none absolute inset-x-0 top-0 h-24 rounded-t-2xl bg-gradient-to-b from-amber-500/[0.08] via-amber-500/[0.02] to-transparent" />
        
        {/* Header */}
        <div className="relative flex min-w-0 items-center justify-between gap-3 border-b border-border/40 pb-3 pr-5">
          <div className="flex min-w-0 items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-amber-500/20 to-amber-500/5 text-amber-600 dark:text-amber-400 ring-1 ring-inset ring-amber-500/20 shadow-xs">
              <Wallet className="h-5 w-5" />
            </div>
            <DialogHeader className="min-w-0 space-y-0.5 text-left">
              <DialogTitle className="break-words text-base font-semibold tracking-tight">
                {t("newapi.queryBalance")} · {provider.name}
              </DialogTitle>
              <DialogDescription className="break-all font-mono text-[11px] text-muted-foreground/80">
                {provider.baseUrl}
              </DialogDescription>
            </DialogHeader>
          </div>
          {official && (
            <span className="hidden sm:inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2.5 py-0.5 text-[11px] font-medium text-emerald-600 dark:text-emerald-400 ring-1 ring-inset ring-emerald-500/20">
              <ShieldCheck className="h-3 w-3" />
              {t("newapi.officialEndpoint")}
            </span>
          )}
        </div>

        {canShare && <div className="flex items-center justify-between gap-3 rounded-lg border border-border/60 bg-muted/20 px-3 py-2 text-xs">
          <span>{shared
            ? t("newapi.balanceSharedHint", { name: firstSiteCard?.name })
            : t("newapi.balanceIndependentHint", { name: firstSiteCard?.name })}</span>
          <Button size="sm" variant="outline" disabled={busy} onClick={async () => {
            setSaving(true);
            setSaveError(null);
            try {
              store.patchBalanceCredentials(await api.providers.setBalanceSharing(provider.id, initialUrl.current, !shared));
              if (!shared) onClose();
            } catch (cause) { setSaveError(formatInvokeError(cause)); }
            finally { setSaving(false); }
          }}>{t(shared ? "newapi.useOwnBalance" : "newapi.useSharedBalance")}</Button>
        </div>}

        {/* Balance Status Cards */}
        <div className="space-y-2.5 pt-1" aria-busy={busy}>
          {commandCode && balance ? <CommandCodeBalanceSummary balance={balance} providerName={provider.name} /> : entries.map((entry, index) => (
            <div
              key={`${entry.unit}-${index}`}
              className="relative overflow-hidden rounded-xl border border-border/60 bg-gradient-to-br from-card via-card to-muted/30 p-4 shadow-xs"
            >
              <div className="flex items-baseline justify-between gap-3">
                <span className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
                  {entry.label ? t(`newapi.balanceWindow.${entry.label}`, { defaultValue: entry.label }) : t("newapi.balanceRemaining")}
                </span>
                <strong className="text-2xl font-bold tabular-nums tracking-tight text-foreground">
                  {number(entry.remaining)}{" "}
                  <span className="text-sm font-medium text-muted-foreground">
                    {entry.unit === "quota" ? t("newapi.balanceQuota") : entry.unit}
                  </span>
                </strong>
              </div>

              {/* Progress bar if total quota exists */}
              {entry.total != null && entry.total > 0 && (
                <div className="mt-3.5 space-y-1.5">
                  <div className="h-2 overflow-hidden rounded-full bg-muted/60">
                    <div
                      className="h-full rounded-full bg-gradient-to-r from-amber-500 to-emerald-500 transition-[width] duration-300 motion-reduce:transition-none"
                      style={{ width: `${Math.max(0, Math.min(100, (entry.remaining / entry.total) * 100))}%` }}
                    />
                  </div>
                  <div className="flex justify-between text-[11px] text-muted-foreground">
                    {entry.used !== null && (
                      <span>{t("newapi.balanceUsed")}: {number(entry.used)} {entry.unit}</span>
                    )}
                    <span>{t("newapi.balanceTotal")}: {number(entry.total)} {entry.unit}</span>
                  </div>
                </div>
              )}

              {/* Fallback metrics when no total is set */}
              {!(entry.total != null && entry.total > 0) && (
                <div className="mt-2 flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
                  {entry.used !== null && (
                    <p>{t("newapi.balanceUsed")}: {number(entry.used)} {entry.unit}</p>
                  )}
                  {entry.total !== null && (
                    <p>{t("newapi.balanceTotal")}: {number(entry.total)} {entry.unit}</p>
                  )}
                </div>
              )}

              {entry.resetsAt && Number.isFinite(Date.parse(entry.resetsAt)) && (
                <p className="mt-2 text-[11px] text-muted-foreground/90">
                  {t("newapi.balanceResets", { time: new Date(entry.resetsAt).toLocaleString(i18n.language) })}
                </p>
              )}

              {!entry.valid && (
                <p className="mt-2 flex items-center gap-1.5 text-xs font-medium text-destructive">
                  <AlertTriangle className="h-3.5 w-3.5 shrink-0" />
                  {t("newapi.balanceUnavailable")}
                </p>
              )}
            </div>
          ))}

          {!balance && (
            <div className="rounded-xl border border-dashed border-border/80 bg-muted/10 py-6 text-center text-sm text-muted-foreground">
              {busy ? (
                <span className="inline-flex items-center gap-2">
                  <Loader2 className="h-4 w-4 animate-spin text-primary" />
                  {t("newapi.balanceQuerying")}
                </span>
              ) : (
                t(mimo && !effectiveScript ? "newapi.balanceMimoShort" : "newapi.balanceNotQueried")
              )}
            </div>
          )}

          {balance && !commandCode && (
            <div className="flex items-center justify-between text-[11px] text-muted-foreground px-1">
              <span>{balanceSourceName(provider.balanceSource)} · {new Date(balance.checkedAt * 1000).toLocaleString(i18n.language)}</span>
            </div>
          )}

          {error && (
            <div role="alert" className="rounded-xl border border-destructive/30 bg-destructive/5 p-3 text-xs text-destructive">
              <div className="flex items-start gap-2">
                <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                <div>
                  <p className="font-medium">{error}</p>
                  {balance && <span className="mt-1 block text-[11px] text-destructive/80">{t("newapi.balanceLastGood")}</span>}
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Custom Script Section */}
        <div className="space-y-3 rounded-xl border border-border/60 bg-muted/15 p-3.5">
          <div className="flex items-center justify-between">
            <label className="flex items-center gap-2 text-xs font-semibold cursor-pointer select-none text-foreground/90">
              <input
                type="checkbox"
                checked={custom}
                disabled={busy}
                onChange={event => {
                  setCustom(event.target.checked);
                  if (!script) setScript(BALANCE_SCRIPT_TEMPLATE);
                }}
                className="h-4 w-4 rounded border-border text-primary focus:ring-ring"
              />
              <Code2 className="h-3.5 w-3.5 text-primary" />
              {t("newapi.customBalanceScript")}
            </label>
          </div>

          {custom && (
            <div className="space-y-2">
              <p className="text-[11px] leading-relaxed text-muted-foreground">
                {t("newapi.balanceScriptHint")}
              </p>
              <textarea
                aria-label={t("newapi.customBalanceScript")}
                spellCheck={false}
                value={script}
                disabled={busy}
                onChange={event => setScript(event.target.value)}
                className="h-48 w-full resize-y rounded-lg border border-border/70 bg-background/90 p-3 font-mono text-xs leading-5 shadow-inner focus:border-primary focus:outline-none focus:ring-2 focus:ring-ring"
              />
            </div>
          )}

          {(custom || provider.balanceScript) && (
            <Button
              size="sm"
              disabled={busy || !dirty || (custom && !script.trim())}
              onClick={async () => {
                setSaving(true);
                setSaveError(null);
                try {
                  const next = await api.providers.saveBalanceScript(provider.id, initialUrl.current, custom ? script : "");
                  store.patchBalanceCredentials(next);
                  await store.refreshBalance(provider.id);
                } catch (cause) {
                  setSaveError(formatInvokeError(cause));
                } finally {
                  setSaving(false);
                }
              }}
            >
              {saving && <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />}
              {t("newapi.balanceSaveAndQuery")}
            </Button>
          )}
        </div>

        {/* Credentials / API Key hint */}
        {mimo ? (
          <p className="rounded-lg bg-sky-500/[0.05] px-3 py-2 text-xs leading-relaxed text-muted-foreground ring-1 ring-sky-500/15">
            {t("newapi.balanceMimoHint")}
          </p>
        ) : official && commandCode ? (
          <details className="text-xs text-muted-foreground">
            <summary className="cursor-pointer select-none">{t("newapi.balanceCredentials")}</summary>
            <p className="mt-2 leading-relaxed">{t("newapi.balanceApiKeyHint")}</p>
          </details>
        ) : official ? (
          <p className="rounded-lg bg-emerald-500/[0.05] px-3 py-2 text-xs leading-relaxed text-muted-foreground ring-1 ring-emerald-500/15">
            {t("newapi.balanceApiKeyHint")}
          </p>
        ) : (
          <details
            open={!custom && !provider.hasAccessToken}
            className="group rounded-xl border border-border/60 bg-card p-3.5 shadow-xs transition-colors"
          >
            <summary className="flex cursor-pointer items-center justify-between text-xs font-semibold text-foreground/90 select-none">
              <span className="flex items-center gap-2">
                <KeyRound className="h-3.5 w-3.5 text-primary" />
                {t("newapi.balanceCredentials")}
              </span>
              <span className="text-[11px] text-muted-foreground font-normal group-open:hidden">
                {t("newapi.clickToExpand")}
              </span>
            </summary>
            <div className="mt-3 space-y-3 pt-2 border-t border-border/40">
              <p className="text-[11px] text-muted-foreground leading-relaxed">
                {t("newapi.balanceCredentialsHint")}
              </p>
              <div className="space-y-1.5">
                <Label htmlFor="balance-user-id" className="text-xs">{t("newapi.userId")}</Label>
                <Input
                  id="balance-user-id"
                  inputMode="numeric"
                  value={userId}
                  onChange={event => setUserId(event.target.value)}
                  disabled={busy}
                  placeholder="例如: 1"
                  className="h-8 text-xs"
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="balance-access-token" className="text-xs">{t("newapi.accessToken")}</Label>
                <Input
                  id="balance-access-token"
                  type="password"
                  autoComplete="off"
                  value={token}
                  onChange={event => setToken(event.target.value)}
                  placeholder={provider.hasAccessToken ? t("newapi.keepSecret") : ""}
                  disabled={busy}
                  className="h-8 text-xs"
                />
              </div>
              <Button size="sm" onClick={() => void saveAndQuery()} disabled={busy}>
                {saving && <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />}
                {t("newapi.balanceSaveAndQuery")}
              </Button>
            </div>
          </details>
        )}

        {/* Footer */}
        <div className="flex items-center justify-between border-t border-border/40 pt-3">
          <Button variant="ghost" size="sm" onClick={onClose} disabled={busy}>
            {t("common.close")}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={busy || dirty || !balanceCredentialReady || (mimo && !effectiveScript)}
            onClick={() => {
              setSaveError(null);
              void store.refreshBalance(provider.id);
            }}
          >
            {busy ? <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" /> : <RefreshCw className="mr-2 h-3.5 w-3.5" />}
            {t("newapi.refreshBalance")}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
