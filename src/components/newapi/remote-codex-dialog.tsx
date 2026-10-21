import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Eye, Loader2 } from "lucide-react";
import { api } from "@/lib/api";
import { clientModels, exclusionOf } from "@/lib/client-config";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { NewApiProvider } from "@/types/newapi";
import type { RemoteCodexPreview, RemoteCodexStatus, RemoteHost } from "@/types/remote";
import { remoteErrorKey } from "./remote-host-dialog";

const RESPONSES = ["responses" as const];

/** Points Codex on a remote host straight at one relay's Responses models. */
export function RemoteCodexDialog({ host, status, providers, onClose, onApplied }: {
  host: RemoteHost;
  status: RemoteCodexStatus;
  providers: NewApiProvider[];
  onClose: () => void;
  onApplied: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const eligible = providers.filter(provider => !exclusionOf(provider, RESPONSES, "native"));
  const [providerId, setProviderId] = useState(() =>
    eligible.find(provider => provider.id === status.request?.providerId)?.id ?? eligible[0]?.id ?? "");
  const provider = eligible.find(entry => entry.id === providerId);
  const models = provider ? clientModels(provider, RESPONSES) : [];
  const [model, setModel] = useState(() =>
    status.request && models.includes(status.request.model) ? status.request.model : models[0] ?? "");
  const [preview, setPreview] = useState<RemoteCodexPreview | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const label = (id: string) => provider?.models.find(entry => entry.id === id)?.displayName || id;
  // The preview shows every field it would replace; applying it replaces them.
  const replacing = status.conflict || !!preview?.conflictFields.length;

  const chooseProvider = (id: string) => {
    const next = eligible.find(entry => entry.id === id);
    const nextModels = next ? clientModels(next, RESPONSES) : [];
    setProviderId(id);
    setModel(next?.model && nextModels.includes(next.model) ? next.model : nextModels[0] ?? "");
    setPreview(null);
  };

  const run = async () => {
    if (busy || !provider || !model) return;
    setBusy(true);
    setError(null);
    const request = { providerId, model };
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
          <DialogDescription>{t("remote.codexDialog.description")}</DialogDescription>
        </DialogHeader>
        {eligible.length === 0 ? <p className="rounded-xl bg-muted/50 px-3 py-2.5 text-sm text-muted-foreground">{t("remote.codexDialog.noProviders")}</p>
          : <fieldset disabled={busy} className="grid gap-3.5 sm:grid-cols-2">
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="remote-codex-provider">{t("remote.codexDialog.provider")}</Label>
              <Select value={providerId} onValueChange={chooseProvider}>
                <SelectTrigger id="remote-codex-provider"><SelectValue /></SelectTrigger>
                <SelectContent>{eligible.map(entry => <SelectItem key={entry.id} value={entry.id}>{entry.name}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="min-w-0 space-y-1.5">
              <Label htmlFor="remote-codex-model">{t("remote.codexDialog.model")}</Label>
              <Select value={model} onValueChange={value => { setModel(value); setPreview(null); }}>
                <SelectTrigger id="remote-codex-model"><SelectValue placeholder={t("clients.selectModel")} /></SelectTrigger>
                <SelectContent>{models.map(id => <SelectItem key={id} value={id}>{label(id)}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <p className="text-[11px] leading-relaxed text-muted-foreground sm:col-span-2">
              {t("remote.codexDialog.models", { count: models.length })}
            </p>
          </fieldset>}
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
          <Button disabled={busy || !provider || !model} onClick={() => void run()}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <Eye className="h-4 w-4" />}
            {t(preview ? replacing ? "clients.applyConflict" : "clients.apply" : "clients.preview")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
