import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Loader2, RefreshCw, Rocket, RotateCcw, Settings2, ShieldCheck, Unlink } from "lucide-react";
import { api } from "@/lib/api";
import { desktopErrorKey } from "@/lib/claude-desktop";
import { ClaudeDesktopDialog } from "./claude-desktop-config-dialog";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { queryKeys } from "@/lib/query-keys";
import type { ClaudeDesktopActionRequest, ClaudeDesktopStatus, DesktopAction } from "@/types/claude-desktop";
import type { NewApiProvider } from "@/types/newapi";

const STATUS_KEY = queryKeys.claudeDesktopStatus;
const CANDIDATES_KEY = queryKeys.claudeDesktopCandidates;

/** 需要确认的管理操作与它们的文案 key。 */
const CONFIRMABLE: DesktopAction[] = [
  "reapply",
  "restorePrevious",
  "switchOfficial",
  "release",
  "recoverRollback",
];

const confirmKey = (action: DesktopAction) =>
  action === "restorePrevious"
    ? { title: "restoreTitle", description: "restoreDescription" }
    : action === "switchOfficial"
      ? { title: "switchOfficialTitle", description: "switchOfficialDescription" }
      : action === "release"
        ? { title: "releaseTitle", description: "releaseDescription" }
        : action === "recoverRollback"
          ? { title: "recoverTitle", description: "recoverDescription" }
          : { title: "restoreTitle", description: "restoreDescription" };

const stateKey = (status: ClaudeDesktopStatus) => `claudeDesktop.state.${status.managementState}`;

/** 后端返回 `kind:detail` 形式的标记；`:detail` 可能是文件角色或模型 ID。 */
const describeFlag = (flag: string) => {
  const [kind, ...rest] = flag.split(":");
  const detail = rest.join(":");
  return detail ? `${kind}(${detail})` : kind;
};

const modeKey = (status: ClaudeDesktopStatus) => {
  switch (status.mode) {
    case "official":
      return "claudeDesktop.modeOfficial";
    case "foreignThirdParty":
      return "claudeDesktop.modeForeignThirdParty";
    case "direct":
      return "claudeDesktop.modeDirect";
    default:
      return "claudeDesktop.modeUnknown";
  }
}

export function ClaudeDesktopConfigCard({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const status = useQuery({
    queryKey: STATUS_KEY,
    queryFn: api.claudeDesktop.status,
    refetchOnWindowFocus: false,
  });
  // 对话框打开时才拉取候选：预览/提交都会在后台重新校验，不信任这份结果。
  const candidates = useQuery({
    queryKey: CANDIDATES_KEY,
    queryFn: () => api.claudeDesktop.candidates(),
    enabled: false,
    refetchOnWindowFocus: false,
  });
  const [dialog, setDialog] = useState(false);
  const [pending, setPending] = useState<DesktopAction | null>(null);
  const [busy, setBusy] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restartError, setRestartError] = useState<string | null>(null);

  const data = status.data;
  const refresh = async () => {
    await Promise.all([
      queryClient.invalidateQueries({ queryKey: STATUS_KEY }),
      queryClient.invalidateQueries({ queryKey: CANDIDATES_KEY }),
    ]);
  };

  const openDialog = () => {
    setError(null);
    setDialog(true);
    void candidates.refetch();
  };

  const actionOf = (action: DesktopAction) => data?.actions.find(entry => entry.action === action);
  // 没有已保存卡片时给出可解释的空状态，而不是一个点不动的按钮。
  const hasSavedCards = providers.length > 0;
  // 恢复未完成事务之前，其余写操作都会被后端拒绝。
  const restartHint = data?.restartRequired ? t("claudeDesktop.card.restartHint") : null;
  const blockedReason = (() => {
    if (!data) return null;
    if (data.pendingRecovery) return t("claudeDesktop.actionReasons.pendingRecovery");
    if (!hasSavedCards) return t("claudeDesktop.card.emptyProviders");
    const first = data.actions.find(entry => !entry.enabled && entry.disabledReason);
    return first?.disabledReason ? t(first.disabledReason) : null;
  })();

  const sessionRequest = (): ClaudeDesktopActionRequest | null => {
    if (!data?.providerId) return null;
    return {
      providerId: data.providerId,
      models: data.models.map(model => ({
        modelId: model.modelId,
        labelOverride: model.label || null,
        oneMMode: model.oneM ? "auto" : "off",
      })),
      paths: data.paths
        ? { normalRoot: data.paths.normalRoot, thirdPartyRoot: data.paths.thirdPartyRoot }
        : {},
      rebaseline: false,
    };
  };

  /** 重启应用：后端会先完全退出再启动，使刚写入的配置立即生效。 */
  const restartApp = async () => {
    setRestarting(true);
    setRestartError(null);
    try {
      await api.claudeDesktop.restart();
      toast({
        title: t("claudeDesktop.card.restartDone"),
        description: t("claudeDesktop.card.restartDoneHint"),
        variant: "success",
      });
    } catch (cause) {
      setRestartError(desktopErrorKey(cause, "claudeDesktop.errors.process"));
    } finally {
      setRestarting(false);
      await refresh();
    }
  };

  const run = async (action: DesktopAction) => {
    const request = sessionRequest();
    if (action !== "recoverRollback" && !request) {
      setError("claudeDesktop.errors.providerMissing");
      setPending(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const payload = request ?? { paths: data?.paths ? { normalRoot: data.paths.normalRoot, thirdPartyRoot: data.paths.thirdPartyRoot } : {} };
      // 管理操作同样先预览再提交，不另走无 revision 的捷径。
      const preview = await api.claudeDesktop.preview(action, payload);
      const applied = await api.claudeDesktop.commit(action, preview.revision, payload);
      toast({
        title: t(`claudeDesktop.actions.${action}`),
        description: applied.requiresRestart
          ? t("claudeDesktop.card.restartHint")
          : applied.warnings.length
            ? applied.warnings.map(key => t(key)).join(" ")
            : undefined,
        variant: "success",
      });
      setPending(null);
    } catch (cause) {
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
    } finally {
      setBusy(false);
      await refresh();
    }
  };

  const confirm = pending ? confirmKey(pending) : null;

  return (
    <section className="space-y-3" aria-labelledby="claude-desktop-title">
      <div>
        <h2 id="claude-desktop-title" className="text-lg font-semibold">
          {t("claudeDesktop.title")}
        </h2>
        <p className="text-sm text-muted-foreground">{t("claudeDesktop.description")}</p>
      </div>

      {(error || status.error) && (
        <p role="alert" className="text-sm text-destructive">
          {t(error ?? desktopErrorKey(status.error, "claudeDesktop.errors.storage"))}
        </p>
      )}

      <div className="rounded-xl border bg-card/70 p-4">
        {status.isPending && <p className="text-sm text-muted-foreground">{t("common.loading")}</p>}
        {data && (
          <div className="space-y-3">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={data.managementState === "managed" ? "secondary" : "outline"}>
                {t(stateKey(data))}
              </Badge>
              <Badge variant="outline" className="text-xs">
                {t(modeKey(data))}
              </Badge>
              {data.detection.version !== "unknown" && (
                <span className="text-xs tabular-nums text-muted-foreground">
                  v{data.detection.version}
                </span>
              )}
              {data.restartRequired && (
                <Badge variant="destructive" className="ml-auto text-xs">
                  {t("claudeDesktop.card.requiresRestart")}
                </Badge>
              )}
            </div>

            {restartHint && (
              <p className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground">
                {restartHint}
              </p>
            )}

            {/* 供应商与模型是这张卡片的主信息。 */}
            <p className="break-words text-sm">
              {data.isManaged
                ? t("claudeDesktop.card.hintManaged", { name: data.providerName ?? "—" })
                : t("claudeDesktop.card.hintUnmanaged")}
            </p>

            {data.models.length > 0 && (
              <p className="break-words font-mono text-xs text-muted-foreground">
                {data.models.map(model => model.modelId).join(" · ")}
              </p>
            )}

            {data.paths && (
              <p className="break-all font-mono text-xs text-muted-foreground">{data.paths.normalConfig}</p>
            )}

            {/* 未完成事务优先于一切：它阻止其它写操作。 */}
            {data.pendingRecovery && (
              <div className="space-y-2 rounded-lg border border-destructive/40 bg-destructive/5 px-3 py-2">
                <p role="alert" className="text-xs text-destructive">
                  {t("claudeDesktop.card.recoveryHint")}
                </p>
                <Button
                  size="sm"
                  variant="destructive"
                  disabled={busy || !actionOf("recoverRollback")?.enabled}
                  onClick={() => setPending("recoverRollback")}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  {t("claudeDesktop.actions.recoverRollback")}
                </Button>
              </div>
            )}

            {data.providerMissing && (
              <p role="alert" className="text-xs text-destructive">{t("claudeDesktop.card.providerMissing")}</p>
            )}

            {data.conflicts.length > 0 && (
              <p role="alert" className="break-words text-xs text-destructive">
                {t("claudeDesktop.card.conflicts")}: {data.conflicts.map(describeFlag).join(" · ")}
              </p>
            )}

            {data.staleReasons.length > 0 && (
              <p role="status" className="break-words text-xs text-amber-600 dark:text-amber-500">
                {t("claudeDesktop.card.staleReasons")}: {data.staleReasons.map(describeFlag).join(" · ")}
              </p>
            )}

            {(data.lastAppliedAt !== null || data.backupCount > 0) && (
              <p className="text-xs text-muted-foreground">
                {data.lastAppliedAt
                  ? `${t("claudeDesktop.card.lastApplied")}: ${new Date(data.lastAppliedAt * 1000).toLocaleString()}`
                  : t("claudeDesktop.card.neverApplied")}
                {data.backupCount > 0 && ` · ${t("claudeDesktop.card.backupCount")}: ${data.backupCount}`}
              </p>
            )}

            {data.warnings.map(key => (
              <p
                key={key}
                role="status"
                className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs text-muted-foreground"
              >
                {t(key)}
              </p>
            ))}

            {/* 空状态给出下一步，而不是一个点不动的按钮。 */}
            {!hasSavedCards && (
              <p role="status" className="text-xs text-muted-foreground">
                {t("claudeDesktop.card.emptyProviders")}
              </p>
            )}

            <div className="space-y-2">
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  variant={data.isManaged ? "outline" : "default"}
                  disabled={busy || !hasSavedCards || !actionOf("applyDirect")?.enabled}
                  onClick={openDialog}
                >
                  <Settings2 className="h-3.5 w-3.5" />
                  {t(data.isManaged ? "claudeDesktop.card.change" : "claudeDesktop.card.configure")}
                </Button>
                {/* 只有确认过启动方式、且配置确实待生效时才出现。 */}
                {data.restartCapable && (data.restartRequired || data.appRunning) && (
                  <Button
                    size="sm"
                    variant={data.restartRequired ? "default" : "ghost"}
                    disabled={busy || restarting}
                    title={t("claudeDesktop.card.restartHint")}
                    onClick={() => void restartApp()}
                  >
                    {restarting ? (
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    ) : (
                      <Rocket className="h-3.5 w-3.5" />
                    )}
                    {t(restarting ? "claudeDesktop.card.restarting" : "claudeDesktop.card.restart")}
                  </Button>
                )}
                {CONFIRMABLE.filter(action => action !== "recoverRollback").map(action => {
                  const view = actionOf(action);
                  if (!view) return null;
                  const Icon =
                    action === "reapply"
                      ? RefreshCw
                      : action === "restorePrevious"
                        ? RotateCcw
                        : action === "switchOfficial"
                          ? ShieldCheck
                          : Unlink;
                  return (
                    <Button
                      key={action}
                      size="sm"
                      variant="ghost"
                      disabled={busy || !view.enabled}
                      title={view.disabledReason ? t(view.disabledReason) : undefined}
                      onClick={() => setPending(action)}
                    >
                      <Icon className="h-3.5 w-3.5" />
                      {t(`claudeDesktop.actions.${action}`)}
                    </Button>
                  );
                })}
              </div>
              {/* 被禁用的原因写在按钮下方：title 在触屏与键盘下都不可靠。 */}
              {restartError && (
                <p role="alert" className="text-[11px] text-destructive">{t(restartError)}</p>
              )}
              {blockedReason && (
                <p role="status" className="text-[11px] text-muted-foreground">{blockedReason}</p>
              )}
            </div>
          </div>
        )}
      </div>

      {dialog && (
        <ClaudeDesktopDialog
          candidates={candidates.data}
          candidatesLoading={candidates.isPending}
          initialProviderId={data?.providerId ?? null}
          availableActions={(data?.actions ?? []).filter(view => view.enabled).map(view => view.action)}
          onClose={() => setDialog(false)}
          onApplied={refresh}
        />
      )}

      <AlertDialog open={pending !== null} onOpenChange={open => { if (!open && !busy) setPending(null); }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{confirm ? t(`claudeDesktop.dialog.${confirm.title}`) : ""}</AlertDialogTitle>
            <AlertDialogDescription>
              {confirm ? t(`claudeDesktop.dialog.${confirm.description}`) : ""}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {error && <p role="alert" className="text-sm text-destructive">{t(error)}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={busy}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction
              disabled={busy}
              onClick={event => {
                event.preventDefault();
                if (pending) void run(pending);
              }}
            >
              {t("common.confirm")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </section>
  );
}
