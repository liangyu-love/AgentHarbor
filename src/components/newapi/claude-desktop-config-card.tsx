import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertCircle,
  Loader2,
  Monitor,
  RefreshCw,
  Rocket,
  RotateCcw,
  Settings2,
  ShieldCheck,
  Unlink,
} from "lucide-react";
import { api } from "@/lib/api";
import { desktopErrorKey, invalidateClaudeDesktop } from "@/lib/claude-desktop";
import { ClaudeDesktopDialog } from "./claude-desktop-config-dialog";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { FoldBody, FoldHeader } from "@/components/ui/fold-header";
import { useFold } from "@/hooks/use-fold";
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
import { desktopProtocolLabel } from "./claude-desktop-route-mapping";
import { cn } from "@/lib/utils";

const STATUS_KEY = queryKeys.claudeDesktopStatus;
const CANDIDATES_KEY = queryKeys.claudeDesktopCandidates;
const DETAIL_WARNINGS = new Set([
  "claudeDesktop.warnings.openPreviews",
  "claudeDesktop.warnings.isolated",
  "claudeDesktop.warnings.released",
]);

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
    : action === "reapply"
      ? { title: "reapplyTitle", description: "reapplyDescription" }
      : action === "switchOfficial"
        ? { title: "switchOfficialTitle", description: "switchOfficialDescription" }
        : action === "release"
          ? { title: "releaseTitle", description: "releaseDescription" }
          : action === "recoverRollback"
            ? { title: "recoverTitle", description: "recoverDescription" }
            : { title: "restoreTitle", description: "restoreDescription" };

const stateKey = (status: ClaudeDesktopStatus) => `claudeDesktop.state.${status.managementState}`;

/** 后端返回 `kind:detail` 形式的标记；`:detail` 可能是文件角色或模型 ID。 */
const FLAG_KEYS: Record<string, string> = {
  name: "claudeDesktop.flags.name",
  credential: "claudeDesktop.flags.credential",
  webAccess: "claudeDesktop.flags.webAccess",
  endpoint: "claudeDesktop.flags.endpoint",
  model: "claudeDesktop.flags.model",
  label: "claudeDesktop.flags.label",
  context: "claudeDesktop.flags.context",
  drift: "claudeDesktop.flags.drift",
  paths: "claudeDesktop.flags.paths",
  pointerMovedExternally: "claudeDesktop.errors.pointerMovedExternally",
  routes: "claudeDesktop.flags.routes",
  route: "claudeDesktop.flags.routes",
  protocol: "claudeDesktop.flags.protocol",
  gateway: "claudeDesktop.flags.gateway",
};

const modeKey = (status: ClaudeDesktopStatus) => {
  switch (status.mode) {
    case "official":
      return "claudeDesktop.modeOfficial";
    case "foreignThirdParty":
      return "claudeDesktop.modeForeignThirdParty";
    case "direct":
      return "claudeDesktop.modeDirect";
    case "router":
      return "claudeDesktop.modeRouter";
    default:
      return "claudeDesktop.modeUnknown";
  }
};

export function ClaudeDesktopConfigCard({ providers }: { providers: NewApiProvider[] }) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [dialog, setDialog] = useState(false);
  const fold = useFold("claude-desktop");
  const status = useQuery({
    queryKey: STATUS_KEY,
    queryFn: api.claudeDesktop.status,
    refetchOnWindowFocus: false,
  });
  // 对话框打开时才拉取候选：预览/提交都会在后台重新校验，不信任这份结果。
  const candidates = useQuery({
    queryKey: CANDIDATES_KEY,
    queryFn: () => api.claudeDesktop.candidates(),
    enabled: dialog,
    refetchOnWindowFocus: false,
  });
  const [pending, setPending] = useState<DesktopAction | null>(null);
  const [busy, setBusy] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [restartError, setRestartError] = useState<string | null>(null);
  const operationBusy = busy || restarting;

  const data = status.data;
  const refresh = () => invalidateClaudeDesktop(queryClient);

  const openDialog = () => {
    setError(null);
    setDialog(true);
  };

  const actionOf = (action: DesktopAction) => data?.actions.find((entry) => entry.action === action);
  // 没有已保存卡片时给出可解释的空状态，而不是一个点不动的按钮。
  const hasSavedCards = providers.length > 0;
  const blockedReason = (() => {
    if (!data) return null;
    if (data.pendingRecovery) return t("claudeDesktop.actionReasons.pendingRecovery");
    if (!hasSavedCards) return t("claudeDesktop.card.emptyProviders");
    const configure = actionOf("applyDirect")?.enabled ? actionOf("applyDirect") : actionOf("applyRouter");
    return !configure?.enabled && configure?.disabledReason ? t(configure.disabledReason) : null;
  })();

  const savedModelChoices = data?.modelChoices?.length
    ? data.modelChoices
    : (data?.models ?? []).map((model) => ({
        modelId: model.modelId,
        labelOverride: model.label || null,
        oneMMode: model.oneM ? ("auto" as const) : ("off" as const),
      }));

  const sessionRequest = (): ClaudeDesktopActionRequest | null => {
    if (!data || (data.mode !== "router" && !data.providerId)) return null;
    return {
      ...(data.mode === "router"
        ? { routes: data.routeChoices ?? [] }
        : { providerId: data.providerId, models: savedModelChoices }),
      paths: data.paths
        ? { normalRoot: data.paths.normalRoot, thirdPartyRoot: data.paths.thirdPartyRoot }
        : {},
      rebaseline: false,
    };
  };

  /** 重启应用：后端会先完全退出再启动，使刚写入的配置立即生效。 */
  const restartApp = async () => {
    if (operationBusy) return;
    setRestarting(true);
    setRestartError(null);
    try {
      await api.claudeDesktop.restart();
      toast({
        title: t("claudeDesktop.card.restartDone"),
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
    if (operationBusy) return;
    const request = action === "reapply" ? sessionRequest() : {
      paths: data?.paths ? { normalRoot: data.paths.normalRoot, thirdPartyRoot: data.paths.thirdPartyRoot } : {},
    };
    if (!request) {
      setError("claudeDesktop.errors.providerMissing");
      setPending(null);
      return;
    }
    setBusy(true);
    setError(null);
    try {
      // 恢复直接核对已有 journal；pending 状态不能再创建配置预览。
      let revision = "";
      if (action !== "recoverRollback") {
        const preview = await api.claudeDesktop.preview(action, request);
        revision = preview.revision;
      }
      const applied = await api.claudeDesktop.commit(action, revision, request);
      toast({
        title: t(`claudeDesktop.actions.${action}`),
        description: applied.requiresRestart
          ? t("claudeDesktop.dialog.appliedDescription")
          : applied.warnings.length
            ? applied.warnings.map((key) => t(key)).join(" ")
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

  const describeFlag = (flag: string) => {
    const [kind, ...rest] = flag.split(":");
    const detail = rest.join(":");
    const text = t(FLAG_KEYS[kind] ?? "claudeDesktop.flags.unknown");
    const roleKey = `claudeDesktop.fileRoles.${detail}`;
    return detail ? `${text}（${kind === "drift" ? t(roleKey, { defaultValue: detail }) : detail}）` : text;
  };

  return (
    <section
      className="space-y-4 rounded-2xl border border-border bg-card p-4 shadow-sm sm:p-5"
      aria-labelledby="claude-desktop-title"
    >
      <FoldHeader
        open={fold.open}
        onToggle={fold.toggle}
        titleId="claude-desktop-title"
        contentId="claude-desktop-body"
        title={t("claudeDesktop.title")}
        leading={
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary/10 text-primary">
            <Monitor className="h-4 w-4" />
          </div>
        }
      />

      <FoldBody open={fold.open} id="claude-desktop-body" className="space-y-4">
      {(error || status.error) && (
        <div
          role="alert"
          className="flex items-center gap-2 rounded-xl border border-destructive/30 bg-destructive/5 p-3.5 text-sm text-destructive"
        >
          <AlertCircle className="h-4 w-4 shrink-0" />
          <span>{t(error ?? desktopErrorKey(status.error, "claudeDesktop.errors.storage"))}</span>
        </div>
      )}

      <div className="group relative overflow-hidden rounded-2xl border border-border/80 bg-card/80 p-5 shadow-sm transition-all duration-300 hover:border-primary/40 hover:shadow-md">
        {/* Subtle top indicator bar */}
        <div className="absolute left-5 top-0 h-0.5 w-10 rounded-b-sm bg-primary transition-all duration-300 group-hover:w-16" />

        {status.isPending && (
          <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
            <RefreshCw className="h-4 w-4 animate-spin text-primary" />
            <span>{t("common.loading")}</span>
          </div>
        )}

        {data && (
          <div className="space-y-4">
            {/* Top badges bar */}
            <div className="flex flex-wrap items-center gap-2">
              <Badge
                variant={data.managementState === "managed" ? "secondary" : "outline"}
                className={cn(
                  "gap-1 font-medium shadow-sm",
                  data.managementState === "managed" && "bg-primary/10 text-primary border-primary/30",
                )}
              >
                {data.managementState === "managed" && <span className="h-1.5 w-1.5 rounded-full bg-primary" />}
                {t(stateKey(data))}
              </Badge>
              <Badge variant="outline" className="text-xs font-normal">
                {t(modeKey(data))}
              </Badge>
              {data.mode === "router" && data.router?.listenerReady === false && (
                <Badge variant="destructive" className="text-xs" role="status">
                  {t("claudeDesktop.routing.listenerNotReady")}
                </Badge>
              )}
              {data.detection.version !== "unknown" && (
                <span className="rounded-md bg-muted px-1.5 py-0.5 font-mono text-xs tabular-nums text-muted-foreground">
                  v{data.detection.version}
                </span>
              )}
              {data.restartRequired && (
                <Badge variant="destructive" className="ml-auto text-xs">
                  {t("claudeDesktop.card.requiresRestart")}
                </Badge>
              )}
            </div>

            {/* Main Provider / Route targets card */}
            <div className="rounded-xl border border-border/50 bg-background/60 p-3.5 shadow-sm space-y-2">
              {data.mode !== "router" && (
                <p data-desktop-provider className="break-words text-sm font-semibold text-foreground">
                  {data.isManaged ? data.providerName ?? "—" : t("claudeDesktop.card.hintUnmanaged")}
                </p>
              )}

              {data.mode !== "router" && data.models.length > 0 && (
                <p data-desktop-models className="break-words font-mono text-xs text-muted-foreground">
                  {data.models.map((model) => model.modelId).join(" · ")}
                </p>
              )}

              {data.mode === "router" && (data.routeViews ?? []).length > 0 && (
                <ul data-desktop-route-targets className="grid gap-2 sm:grid-cols-2">
                  {(data.routeViews ?? []).map((route) => (
                    <li
                      key={route.role}
                      className="flex min-w-0 items-center justify-between gap-2 rounded-lg border border-border/40 bg-card/60 px-3 py-2 text-xs"
                    >
                      <div className="min-w-0">
                        <div className="flex items-center gap-1.5">
                          <span className="capitalize font-semibold text-foreground">{route.role}</span>
                          <span className="text-[10px] text-muted-foreground font-mono">
                            {desktopProtocolLabel(route.protocol)}
                          </span>
                        </div>
                        <div className="truncate text-muted-foreground font-mono text-[11px]" title={`${route.providerName} · ${route.modelId}`}>
                          {route.providerName} · {route.modelId}
                        </div>
                      </div>
                      {!route.available && (
                        <Badge variant="destructive" className="text-[10px] shrink-0">
                          {t("claudeDesktop.routing.unavailable")}
                        </Badge>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </div>

            {/* 未完成事务优先于一切：它阻止其它写操作。 */}
            {data.pendingRecovery && (
              <div className="space-y-2 rounded-xl border border-destructive/40 bg-destructive/5 p-3.5">
                <p role="alert" className="text-xs text-destructive font-medium">
                  {t("claudeDesktop.card.recoveryHint")}
                </p>
                <Button
                  size="sm"
                  variant="destructive"
                  disabled={operationBusy || !actionOf("recoverRollback")?.enabled}
                  onClick={() => setPending("recoverRollback")}
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  {t("claudeDesktop.actions.recoverRollback")}
                </Button>
              </div>
            )}

            {data.providerMissing && (
              <p role="alert" className="text-xs text-destructive bg-destructive/5 p-2.5 rounded-lg border border-destructive/20">
                {t("claudeDesktop.card.providerMissing")}
              </p>
            )}

            {data.conflicts.length > 0 && (
              <p role="alert" className="break-words text-xs text-destructive bg-destructive/5 p-2.5 rounded-lg border border-destructive/20">
                {t("claudeDesktop.card.conflicts")}: {data.conflicts.map(describeFlag).join(" · ")}
              </p>
            )}

            {data.staleReasons.length > 0 && (
              <p role="status" className="break-words text-xs text-amber-600 dark:text-amber-500 bg-amber-500/5 p-2.5 rounded-lg border border-amber-500/20">
                {t("claudeDesktop.card.staleReasons")}: {data.staleReasons.map(describeFlag).join(" · ")}
              </p>
            )}

            {(data.paths ||
              data.lastAppliedAt !== null ||
              data.backupCount > 0 ||
              data.warnings.some((key) => DETAIL_WARNINGS.has(key))) && (
              <details className="text-xs text-muted-foreground group/details">
                <summary className="w-fit cursor-pointer rounded-md px-2 py-1 transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">
                  {t("claudeDesktop.card.details")}
                </summary>
                <div className="mt-2 space-y-2 rounded-lg border border-border/40 bg-muted/20 p-3">
                  {data.paths && (
                    <p data-desktop-config-path className="break-all font-mono text-[11px] text-foreground">
                      {data.paths.normalConfig}
                    </p>
                  )}
                  {data.router && (
                    <p className="font-mono text-[11px] text-foreground">127.0.0.1:{data.router.boundPort}</p>
                  )}
                  <p className="text-[11px]">
                    {data.lastAppliedAt
                      ? `${t("claudeDesktop.card.lastApplied")}: ${new Date(data.lastAppliedAt * 1000).toLocaleString()}`
                      : t("claudeDesktop.card.neverApplied")}
                    {data.backupCount > 0 && ` · ${t("claudeDesktop.card.backupCount")}: ${data.backupCount}`}
                  </p>
                  {data.warnings
                    .filter((key) => DETAIL_WARNINGS.has(key))
                    .map((key) => (
                      <p key={key} className="text-amber-600 dark:text-amber-400">
                        {t(key)}
                      </p>
                    ))}
                </div>
              </details>
            )}

            {data.warnings
              .filter(
                (key) =>
                  !DETAIL_WARNINGS.has(key) &&
                  key !== "claudeDesktop.warnings.requiresRestart" &&
                  key !== "claudeDesktop.warnings.pendingRecovery",
              )
              .map((key) => (
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

            <div className="space-y-2 pt-3 border-t border-border/50">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant={data.isManaged ? "outline" : "default"}
                  className={cn(
                    "h-8 gap-1.5 rounded-lg text-xs font-semibold shadow-sm",
                    !data.isManaged && "bg-primary text-primary-foreground hover:bg-primary/90",
                  )}
                  disabled={operationBusy || !hasSavedCards || (!actionOf("applyDirect")?.enabled && !actionOf("applyRouter")?.enabled)}
                  onClick={openDialog}
                >
                  <Settings2 className="h-3.5 w-3.5" />
                  {t(data.isManaged ? "claudeDesktop.card.change" : "claudeDesktop.card.configure")}
                </Button>

                {/* 已确认启动方式后，关闭状态也可一键启动。 */}
                {data.restartCapable && (
                  <Button
                    size="sm"
                    variant={data.restartRequired ? "default" : "ghost"}
                    className={cn(
                      "h-8 gap-1.5 rounded-lg text-xs font-semibold shadow-sm",
                      data.restartRequired && "bg-destructive text-destructive-foreground hover:bg-destructive/90",
                    )}
                    disabled={operationBusy}
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

                {CONFIRMABLE.filter((action) => action !== "recoverRollback").map((action) => {
                  const view = actionOf(action);
                  if (!view) return null;
                  const disabled = operationBusy || !view.enabled;
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
                      variant={disabled ? "ghost" : "outline"}
                      className={cn(
                        "h-8 gap-1.5 rounded-lg text-xs",
                        disabled
                          ? "text-muted-foreground"
                          : "border-primary/30 bg-primary/5 text-primary shadow-sm hover:border-primary/60 hover:bg-primary/10 hover:text-primary",
                      )}
                      disabled={disabled}
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
              {restartError && <p role="alert" className="text-[11px] text-destructive">{t(restartError)}</p>}
              {blockedReason && <p role="status" className="text-[11px] text-muted-foreground">{blockedReason}</p>}
            </div>
          </div>
        )}
      </div>
      </FoldBody>

      {dialog && (
        <ClaudeDesktopDialog
          candidates={candidates.data}
          candidatesLoading={candidates.isFetching}
          candidatesError={candidates.error}
          candidatesRevision={candidates.dataUpdatedAt}
          initialProviderId={data?.providerId ?? null}
          initialModelChoices={savedModelChoices}
          initialMode={data?.mode === "router" ? "router" : "direct"}
          initialRouteChoices={data?.routeChoices ?? []}
          availableActions={(data?.actions ?? []).filter((view) => view.enabled).map((view) => view.action)}
          onClose={() => setDialog(false)}
          onApplied={refresh}
        />
      )}

      <AlertDialog
        open={pending !== null}
        onOpenChange={(open) => {
          if (!open && !busy) setPending(null);
        }}
      >
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
              onClick={(event) => {
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
