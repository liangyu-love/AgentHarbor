import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Eye, Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { clientModels, exclusionOf } from "@/lib/client-config";
import { queryKeys } from "@/lib/query-keys";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import type { NewApiProvider } from "@/types/newapi";
import type { RemoteCodexPreview, RemoteCodexStatus, RemoteHost } from "@/types/remote";
import { remoteErrorKey } from "./remote-host-dialog";

const RESPONSES = ["responses" as const];

/** Codex on a remote host: straight to one relay's Responses models, or
 *  through this machine's router over the host's tunnel. */
export function RemoteCodexDialog({ host, status, providers, onClose, onApplied }: {
  host: RemoteHost;
  status: RemoteCodexStatus;
  providers: NewApiProvider[];
  onClose: () => void;
  onApplied: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const saved = status.request;
  const [routed, setRouted] = useState(!!saved?.router);
  const routerModels = useQuery({ queryKey: queryKeys.remoteRouterModels, queryFn: api.remote.routerModels, enabled: routed });
  const eligible = providers.filter(provider => !exclusionOf(provider, RESPONSES, "native"));
  const [providerId, setProviderId] = useState(() =>
    eligible.find(provider => provider.id === saved?.providerId)?.id ?? eligible[0]?.id ?? "");
  const provider = eligible.find(entry => entry.id === providerId);
  const directModels = provider ? clientModels(provider, RESPONSES) : [];
  const [directModel, setDirectModel] = useState(() =>
    !saved?.router && saved && directModels.includes(saved.model) ? saved.model : directModels[0] ?? "");
  const routerMenu = routerModels.data ?? [];
  const [routerModel, setRouterModel] = useState(saved?.router ? saved.model : "");
  const chosenRouterModel = routerMenu.some(entry => entry.id === routerModel) ? routerModel : routerMenu[0]?.id ?? "";
  const model = routed ? chosenRouterModel : directModel;
  const [webSearch, setWebSearch] = useState(saved?.webSearch ?? false);
  // A profile by default, so `codex` keeps the official sign-in beside it.
  const [asProfile, setAsProfile] = useState(!saved || !!saved.profile);
  const [profile, setProfile] = useState(saved?.profile ?? "relay");
  const profileValid = /^[A-Za-z0-9_-]{1,32}$/.test(profile);
  const [preview, setPreview] = useState<RemoteCodexPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const label = (id: string) => provider?.models.find(entry => entry.id === id)?.displayName || id;
  // The preview shows every field it would replace; applying it replaces them.
  const replacing = status.conflict || !!preview?.conflictFields.length;
  const ready = !!model && (routed || !!provider) && (!asProfile || profileValid);

  const chooseProvider = (id: string) => {
    const next = eligible.find(entry => entry.id === id);
    const nextModels = next ? clientModels(next, RESPONSES) : [];
    setProviderId(id);
    setDirectModel(next?.model && nextModels.includes(next.model) ? next.model : nextModels[0] ?? "");
    setPreview(null);
  };

  const run = async () => {
    if (busy || !ready) return;
    setBusy(true);
    setError(null);
    const request = {
      providerId: routed ? "" : providerId, model, webSearch: routed || webSearch,
      profile: asProfile ? profile : null, router: routed,
    };
    try {
      if (preview) {
        await api.remote.applyCodex(host.id, request, preview.revision, true);
        await onApplied();
        toast({ title: t("remote.clients.applied"), description: t("remote.clients.appliedHint", { name: "Codex" }), variant: "success" });
        onClose();
      } else {
        setPreview(await api.remote.previewCodex(host.id, request, true));
      }
    } catch (cause) {
      setError(remoteErrorKey(cause));
      setPreview(null);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
      <DialogContent className="max-w-xl" data-remote-codex-dialog>
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-tight">{t("remote.configureTitle", { name: "Codex", host: host.name })}</DialogTitle>
          <DialogDescription>{t(routed ? "remote.codexDialog.routedDescription" : "remote.codexDialog.description")}</DialogDescription>
        </DialogHeader>
        <fieldset disabled={busy} className="grid gap-3.5 sm:grid-cols-2">
          <div className="flex w-fit gap-1 rounded-lg border bg-muted/40 p-1 sm:col-span-2" role="group" aria-label={t("clients.accessMode")} data-remote-codex-access>
            {([false, true] as const).map(through => (
              <Button key={String(through)} type="button" size="sm" className="h-7" variant={routed === through ? "secondary" : "ghost"}
                aria-pressed={routed === through} onClick={() => { setRouted(through); setPreview(null); }}>
                {t(through ? "remote.accessRouter" : "remote.accessDirect")}
              </Button>
            ))}
          </div>
          {routed ? <div className="min-w-0 space-y-1.5 sm:col-span-2">
            <Label htmlFor="remote-codex-router-model">{t("remote.codexDialog.model")}</Label>
            <Select value={chosenRouterModel} onValueChange={value => { setRouterModel(value); setPreview(null); }}>
              <SelectTrigger id="remote-codex-router-model">
                <SelectValue placeholder={t(routerModels.isLoading ? "remote.connecting" : "clients.selectModel")} />
              </SelectTrigger>
              <SelectContent>{routerMenu.map(entry => <SelectItem key={entry.id} value={entry.id}>{entry.label}</SelectItem>)}</SelectContent>
            </Select>
            <p className="text-[11px] leading-relaxed text-muted-foreground">{t("remote.codexDialog.routedModels", { count: routerMenu.length })}</p>
          </div> : eligible.length === 0
            ? <p className="rounded-xl bg-muted/50 px-3 py-2.5 text-sm text-muted-foreground sm:col-span-2">{t("remote.codexDialog.noProviders")}</p>
            : <>
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor="remote-codex-provider">{t("remote.codexDialog.provider")}</Label>
                <Select value={providerId} onValueChange={chooseProvider}>
                  <SelectTrigger id="remote-codex-provider"><SelectValue /></SelectTrigger>
                  <SelectContent>{eligible.map(entry => <SelectItem key={entry.id} value={entry.id}>{entry.name}</SelectItem>)}</SelectContent>
                </Select>
              </div>
              <div className="min-w-0 space-y-1.5">
                <Label htmlFor="remote-codex-model">{t("remote.codexDialog.model")}</Label>
                <Select value={directModel} onValueChange={value => { setDirectModel(value); setPreview(null); }}>
                  <SelectTrigger id="remote-codex-model"><SelectValue placeholder={t("clients.selectModel")} /></SelectTrigger>
                  <SelectContent>{directModels.map(id => <SelectItem key={id} value={id}>{label(id)}</SelectItem>)}</SelectContent>
                </Select>
              </div>
              <p className="text-[11px] leading-relaxed text-muted-foreground sm:col-span-2">
                {t("remote.codexDialog.models", { count: directModels.length })}
              </p>
              <div className="flex items-start justify-between gap-3 rounded-xl bg-muted/50 px-3 py-2.5 sm:col-span-2">
                <div className="min-w-0 space-y-0.5">
                  <Label htmlFor="remote-codex-web-search">{t("remote.codexDialog.webSearch")}</Label>
                  <p className="text-[11px] leading-relaxed text-muted-foreground">{t("remote.codexDialog.webSearchHint")}</p>
                </div>
                <Switch id="remote-codex-web-search" checked={webSearch} onCheckedChange={value => { setWebSearch(value); setPreview(null); }} />
              </div>
            </>}
          <div className="space-y-2 rounded-xl bg-muted/50 px-3 py-2.5 sm:col-span-2" data-remote-codex-mode>
            <div className="flex w-fit gap-1 rounded-lg border bg-background/60 p-1" role="group" aria-label={t("remote.codexDialog.mode")}>
              {([true, false] as const).map(profiled => (
                <Button key={String(profiled)} type="button" size="sm" className="h-7" variant={asProfile === profiled ? "secondary" : "ghost"}
                  aria-pressed={asProfile === profiled} onClick={() => { setAsProfile(profiled); setPreview(null); }}>
                  {t(profiled ? "remote.codexDialog.asProfile" : "remote.codexDialog.asDefault")}
                </Button>
              ))}
            </div>
            {asProfile ? <div className="flex flex-wrap items-center gap-2">
              <Label htmlFor="remote-codex-profile" className="text-xs">{t("remote.codexDialog.profileName")}</Label>
              <Input id="remote-codex-profile" value={profile} maxLength={32} spellCheck={false} aria-invalid={!profileValid}
                className="h-8 w-32 font-mono text-xs" onChange={event => { setProfile(event.target.value.trim()); setPreview(null); }} />
              <p className="w-full text-[11px] leading-relaxed text-muted-foreground">
                {t(profileValid ? "remote.codexDialog.profileHint" : "remote.errors.codexProfile", { name: profile })}
              </p>
            </div> : <p className="text-[11px] leading-relaxed text-muted-foreground">{t("remote.codexDialog.defaultHint")}</p>}
          </div>
          {routed && <p className="rounded-xl bg-primary/[0.06] px-3 py-2 text-[11px] leading-relaxed text-muted-foreground sm:col-span-2">
            {t("remote.routedNote", { port: host.tunnelPort })}
          </p>}
        </fieldset>
        {preview && <div className="min-w-0 space-y-2" data-remote-codex-preview>
          <p className="text-sm font-semibold">{t("remote.codexDialog.previewTitle")}</p>
          <pre className="max-h-64 overflow-auto rounded-md bg-muted p-3 text-xs">{preview.content}</pre>
          {!!preview.conflictFields.length && <p role="status" className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs">
            {t("remote.codexDialog.conflict", { fields: preview.conflictFields.join(", ") })}
          </p>}
          <p className="text-xs text-muted-foreground">{t("remote.backupHint")}</p>
        </div>}
        {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>{t("common.cancel")}</Button>
          <Button disabled={busy || !ready} onClick={() => void run()}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}
            {t(preview ? replacing ? "clients.applyConflict" : "clients.apply" : "clients.preview")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
