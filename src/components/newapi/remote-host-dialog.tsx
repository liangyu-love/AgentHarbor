import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { KeyRound, Loader2, LockKeyhole, ShieldCheck } from "lucide-react";
import { api } from "@/lib/api";
import { errorKey } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { SecretInput } from "@/components/ui/secret-input";
import type { RemoteAuth, RemoteHost } from "@/types/remote";

export const remoteErrorKey = (cause: unknown) =>
  errorKey(cause, ["remote", "clients", "newapi", "accountSources"], "remote.errors.storage");

/** Adds or edits a remote host. Secrets are write-only: a blank field keeps the stored one. */
export function RemoteHostDialog({ host, onClose, onSaved }: {
  host: RemoteHost | null;
  onClose: () => void;
  onSaved: (host: RemoteHost) => void;
}) {
  const { t } = useTranslation();
  const aliases = useQuery({ queryKey: queryKeys.sshAliases, queryFn: api.remote.sshAliases, staleTime: 30_000 });
  const [name, setName] = useState(host?.name ?? "");
  const [address, setAddress] = useState(host?.host ?? "");
  const [port, setPort] = useState(host?.port ? String(host.port) : "");
  const [user, setUser] = useState(host?.user ?? "");
  const [auth, setAuth] = useState<RemoteAuth>(host?.auth ?? "key");
  const [identityFile, setIdentityFile] = useState(host?.identityFile ?? "");
  const [passphrase, setPassphrase] = useState("");
  const [clearPassphrase, setClearPassphrase] = useState(false);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const portNumber = port.trim() === "" ? null : Number(port);

  const save = async () => {
    if (busy) return;
    if (portNumber !== null && (!Number.isInteger(portNumber) || portNumber < 1 || portNumber > 65535)) {
      setError("remote.errors.port"); return;
    }
    setBusy(true);
    setError(null);
    try {
      onSaved(await api.remote.save({
        id: host?.id ?? null, name: name.trim() || address.trim(), host: address.trim(), port: portNumber,
        user: user.trim() || null, auth, identityFile: auth === "key" ? identityFile.trim() || null : null,
        password: auth === "password" ? password : null, passphrase: auth === "key" ? passphrase : null,
        clearPassphrase: auth === "key" && clearPassphrase,
      }));
    } catch (cause) {
      setError(remoteErrorKey(cause));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
      <DialogContent className="max-w-lg" data-remote-host-dialog>
        <DialogHeader>
          <DialogTitle className="font-display text-xl tracking-tight">{t(host ? "remote.editTitle" : "remote.addTitle")}</DialogTitle>
          <DialogDescription>{t("remote.hostDialogHint")}</DialogDescription>
        </DialogHeader>
        <form id="remote-host-form" className="grid grid-cols-6 gap-x-3 gap-y-3.5" onSubmit={event => { event.preventDefault(); void save(); }}>
          <fieldset disabled={busy} className="contents">
            <div className="col-span-6 space-y-1.5">
              <Label htmlFor="remote-host-name">{t("remote.fields.name")}</Label>
              <Input id="remote-host-name" value={name} maxLength={64} placeholder={address || t("remote.fields.namePlaceholder")}
                onChange={event => setName(event.target.value)} />
            </div>
            <div className="col-span-4 space-y-1.5">
              <Label htmlFor="remote-host-address">{t("remote.fields.host")}</Label>
              <Input id="remote-host-address" value={address} list="remote-ssh-aliases" spellCheck={false} autoComplete="off"
                placeholder="dev · 10.0.0.5 · example.com" onChange={event => setAddress(event.target.value)} />
              <datalist id="remote-ssh-aliases">{aliases.data?.map(alias => <option key={alias} value={alias} />)}</datalist>
            </div>
            <div className="col-span-2 space-y-1.5">
              <Label htmlFor="remote-host-port">{t("remote.fields.port")}</Label>
              <Input id="remote-host-port" value={port} inputMode="numeric" placeholder={String(host?.resolved.port ?? 22)}
                onChange={event => setPort(event.target.value.replace(/[^0-9]/g, ""))} />
            </div>
            <p className="col-span-6 -mt-2 text-[11px] text-muted-foreground">
              {t("remote.fields.hostHint")}{aliases.data?.length ? " " + t("remote.fields.aliases", { count: aliases.data.length }) : ""}
            </p>
            <div className="col-span-6 space-y-1.5">
              <Label htmlFor="remote-host-user">{t("remote.fields.user")}</Label>
              <Input id="remote-host-user" value={user} spellCheck={false} autoComplete="off"
                placeholder={host?.resolved.user ?? t("remote.fields.userPlaceholder")} onChange={event => setUser(event.target.value)} />
            </div>
            <div className="col-span-6 space-y-2">
              <Label>{t("remote.fields.auth")}</Label>
              <div className="flex w-fit gap-1 rounded-lg border bg-muted/40 p-1" role="group" aria-label={t("remote.fields.auth")}>
                {(["key", "password"] as const).map(mode => (
                  <Button key={mode} type="button" size="sm" className="h-7 gap-1.5" variant={auth === mode ? "secondary" : "ghost"}
                    aria-pressed={auth === mode} onClick={() => setAuth(mode)}>
                    {mode === "key" ? <KeyRound className="h-3.5 w-3.5" /> : <LockKeyhole className="h-3.5 w-3.5" />}
                    {t(mode === "key" ? "remote.authKey" : "remote.authPassword")}
                  </Button>
                ))}
              </div>
            </div>
            {auth === "key" ? <>
              <div className="col-span-6 space-y-1.5">
                <Label htmlFor="remote-host-identity">{t("remote.fields.identityFile")}</Label>
                <Input id="remote-host-identity" value={identityFile} spellCheck={false} autoComplete="off" className="font-mono text-xs"
                  placeholder={host?.resolved.identityFiles[0] ?? "~/.ssh/id_ed25519"} onChange={event => setIdentityFile(event.target.value)} />
                <p className="text-[11px] text-muted-foreground">{t("remote.fields.identityFileHint")}</p>
              </div>
              <div className="col-span-6 space-y-1.5">
                <Label htmlFor="remote-host-passphrase">{t("remote.fields.passphrase")}</Label>
                <SecretInput id="remote-host-passphrase" value={passphrase} onChange={setPassphrase} disabled={busy || clearPassphrase}
                  placeholder={host?.hasPassphrase ? t("remote.fields.storedSecret") : t("remote.fields.passphraseHint")} />
                {host?.hasPassphrase && <label className="flex items-center gap-2 text-xs text-muted-foreground">
                  <Checkbox checked={clearPassphrase} onCheckedChange={value => setClearPassphrase(value === true)} />
                  {t("remote.fields.clearPassphrase")}
                </label>}
              </div>
            </> : <div className="col-span-6 space-y-1.5">
              <Label htmlFor="remote-host-password">{t("remote.fields.password")}</Label>
              <SecretInput id="remote-host-password" value={password} onChange={setPassword}
                placeholder={host?.hasPassword ? t("remote.fields.storedSecret") : ""} />
            </div>}
          </fieldset>
        </form>
        <p className="flex items-start gap-2 rounded-xl bg-muted/50 px-3 py-2 text-[11px] leading-relaxed text-muted-foreground">
          <ShieldCheck aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0 text-emerald-600 dark:text-emerald-400" />
          {t("remote.fields.secretsNote")}
        </p>
        {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
        <DialogFooter>
          <Button variant="outline" onClick={onClose} disabled={busy}>{t("common.cancel")}</Button>
          <Button type="submit" form="remote-host-form" disabled={busy || !address.trim()}>
            {busy && <Loader2 className="h-4 w-4 animate-spin" />}{t("remote.save")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
