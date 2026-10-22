import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Check, Globe, Loader2, Search, Trash2, Upload, X } from "lucide-react";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { SearchTest, WebSearchStatus } from "@/types/remote";
import { remoteErrorKey } from "./remote-host-dialog";

const TEST_QUERY = "latest stable Linux kernel version";

/** Web search on a host: an MCP server the host's Codex and Claude Code call,
 *  backed by a relay's own search service (DeepSeek's API or MiMo's Chat
 *  search), with no router in between. */
export function RemoteWebSearch({ hostId, status, onChanged }: {
  hostId: string;
  status: WebSearchStatus;
  onChanged: () => Promise<void>;
}) {
  const { t } = useTranslation();
  const providers = useQuery({ queryKey: queryKeys.remoteSearchProviders, queryFn: api.remote.searchProviders });
  const cards = providers.data ?? [];
  const [providerId, setProviderId] = useState(status.request?.providerId ?? "");
  const card = cards.find(entry => entry.id === providerId) ?? cards[0];
  const [model, setModel] = useState(status.request?.model ?? "");
  const chosenModel = card?.backend === "mimo" ? (card.models.includes(model) ? model : card.models[0] ?? "") : null;
  const [busy, setBusy] = useState<"deploy" | "remove" | "test" | null>(null);
  const [test, setTest] = useState<SearchTest | { failed: string } | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const changed = !!status.request && (status.request.providerId !== card?.id || (status.request.model ?? null) !== chosenModel);

  const deploy = async () => {
    if (!card) return;
    setBusy("deploy");
    setTest(null);
    try {
      await api.remote.deploySearch(hostId, { providerId: card.id, model: chosenModel });
      toast({ title: t("remote.search.deployed"), description: t("remote.search.deployedHint"), variant: "success" });
      await onChanged();
    } catch (cause) {
      toast({ title: t("remote.actionFailed"), description: t(remoteErrorKey(cause)), variant: "destructive" });
    } finally {
      setBusy(null);
    }
  };
  const remove = () => setConfirm({
    title: t("remote.search.removeTitle"), description: t("remote.search.removeDescription"), confirmLabel: t("remote.search.remove"),
    destructive: true,
    onConfirm: () => void (async () => {
      setBusy("remove");
      setTest(null);
      try {
        await api.remote.removeSearch(hostId);
        toast({ title: t("remote.search.removed"), variant: "success" });
        await onChanged();
      } catch (cause) {
        toast({ title: t("remote.actionFailed"), description: t(remoteErrorKey(cause)), variant: "destructive" });
      } finally {
        setBusy(null);
      }
    })(),
  });
  const runTest = async () => {
    setBusy("test");
    setTest(null);
    try {
      setTest(await api.remote.testSearch(hostId, TEST_QUERY));
    } catch (cause) {
      setTest({ failed: remoteErrorKey(cause) });
    } finally {
      setBusy(null);
    }
  };

  const deployedCard = cards.find(entry => entry.id === status.request?.providerId);
  return (
    <div className={cn("harbor-well relative space-y-3 px-4 py-3.5",
      status.deployed && "before:absolute before:inset-y-3 before:left-0 before:w-[3px] before:rounded-full before:bg-emerald-500")}
      data-remote-search={status.deployed ? "deployed" : "absent"}>
      <div className="flex flex-wrap items-center gap-3">
        <span className="inject-tab-glyph"><Globe aria-hidden className="h-[18px] w-[18px]" /></span>
        <div className="min-w-0 flex-1 basis-56">
          <p className="font-display text-[15px] font-semibold tracking-tight">{t("remote.search.title")}</p>
          <p className={cn("text-xs", status.deployed ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground")}>
            {status.deployed
              ? t("remote.search.deployedVia", {
                card: deployedCard?.name ?? status.request?.providerId,
                backend: t(`remote.search.backend.${status.backend ?? "deepSeek"}`),
              })
              : t("remote.search.absent")}
          </p>
        </div>
        {status.deployed && <div className="flex shrink-0 flex-wrap items-center gap-1.5 text-[11px]" data-remote-search-clients>
          {([["Codex", status.codex], ["Claude Code", status.claudeCode]] as const).map(([name, registered]) => (
            <span key={name} className="kind-chip" aria-pressed={registered}>
              {registered ? <Check aria-hidden className="h-3 w-3" /> : <X aria-hidden className="h-3 w-3" />}{name}
            </span>
          ))}
        </div>}
      </div>
      <div className="space-y-2 sm:pl-[2.8rem]">
        <p className="text-[11px] leading-relaxed text-muted-foreground">{t("remote.search.hint")}</p>
        {cards.length === 0 && providers.isSuccess
          ? <p className="text-xs text-amber-700 dark:text-amber-300">{t("remote.search.noProviders")}</p>
          : <div className="flex flex-wrap items-center gap-2">
            <Select value={card?.id ?? ""} onValueChange={value => { setProviderId(value); setTest(null); }} disabled={!!busy}>
              <SelectTrigger className="h-8 w-auto min-w-56 max-w-full gap-2" aria-label={t("remote.search.provider")}><SelectValue placeholder={t("remote.search.provider")} /></SelectTrigger>
              <SelectContent>{cards.map(entry => <SelectItem key={entry.id} value={entry.id}>
                {entry.name} · {t(`remote.search.backend.${entry.backend}`)}
              </SelectItem>)}</SelectContent>
            </Select>
            {card?.backend === "mimo" && <Select value={chosenModel ?? ""} onValueChange={setModel} disabled={!!busy}>
              <SelectTrigger className="h-8 w-48" aria-label={t("remote.search.model")}><SelectValue /></SelectTrigger>
              <SelectContent>{card.models.map(id => <SelectItem key={id} value={id}>{id}</SelectItem>)}</SelectContent>
            </Select>}
            <Button size="sm" className="gap-1.5" disabled={!!busy || !card || (status.deployed && !changed && !status.stale)} onClick={() => void deploy()}>
              {busy === "deploy" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Upload className="h-3.5 w-3.5" />}
              {t(status.deployed ? "remote.search.update" : "remote.search.deploy")}
            </Button>
            {status.deployed && <>
              <Button size="sm" variant="outline" className="gap-1.5" disabled={!!busy} onClick={() => void runTest()} title={t("remote.search.testHint")}>
                {busy === "test" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Search className="h-3.5 w-3.5" />}{t("remote.search.test")}
              </Button>
              <Button size="sm" variant="ghost" className="gap-1.5 text-destructive hover:text-destructive" disabled={!!busy} onClick={remove}>
                {busy === "remove" ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Trash2 className="h-3.5 w-3.5" />}{t("remote.search.remove")}
              </Button>
            </>}
          </div>}
        {status.stale && <p role="status" className="text-xs text-amber-700 dark:text-amber-300">{t("remote.search.stale")}</p>}
        {status.error && <p role="alert" className="text-xs text-destructive">{t(status.error)}</p>}
        {test && ("failed" in test
          ? <p role="alert" className="flex items-center gap-1.5 text-xs text-destructive" data-remote-search-test="failed"><X className="h-3.5 w-3.5" />{t(test.failed)}</p>
          : <div role="status" className="space-y-1" data-remote-search-test={test.ok ? "ok" : "error"}>
            <p className={cn("flex items-center gap-1.5 text-xs font-medium", test.ok ? "text-emerald-600 dark:text-emerald-400" : "text-destructive")}>
              {test.ok ? <Check className="h-3.5 w-3.5" /> : <X className="h-3.5 w-3.5" />}{t(test.ok ? "remote.search.testOk" : "remote.search.testFailed", { query: TEST_QUERY })}
            </p>
            <pre className="max-h-48 overflow-auto whitespace-pre-wrap rounded-lg bg-background/70 p-2.5 font-mono text-[11px] leading-relaxed text-muted-foreground">{test.output}</pre>
          </div>)}
      </div>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}
