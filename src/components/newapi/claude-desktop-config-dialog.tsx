import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "@/lib/i18n";
import { AlertTriangle, CheckCircle2, Eye, Loader2, MessageSquare, ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { desktopErrorKey } from "@/lib/claude-desktop";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { ClaudeDesktopRouteMapping, desktopRouteSupports1m } from "./claude-desktop-route-mapping";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import type {
  ClaudeDesktopActionRequest,
  ClaudeDesktopBearerReport,
  ClaudeDesktopMessagesReport,
  ClaudeDesktopCandidates,
  ClaudeDesktopPathsChoice,
  ClaudeDesktopModelChoice,
  ClaudeDesktopPreview,
  ClaudeDesktopRouteChoice,
  DesktopAction,
  OneMMode,
} from "@/types/claude-desktop";

/**
 * 后端原因码是 snake_case（`provider_missing`、`unsafe_model_name`），
 * 本地化 key 是 camelCase。`code:param,param` 形态只取冒号前的码，
 * 避免把地址等参数拼进 key。
 */
function reasonKey(reason: string | null | undefined): string {
  if (reason && i18n.exists(reason)) return reason;
  const code = (reason ?? "").split(":")[0]?.trim();
  if (!code) return "claudeDesktop.errors.storage";
  const camel = code.replace(/_([a-z0-9])/g, (_, ch: string) => ch.toUpperCase());
  const key = `claudeDesktop.errors.${camel}`;
  return i18n.exists(key) ? key : "claudeDesktop.errors.storage";
}

function formatManagedValue(value: string | null): string {
  if (value === null) return "—";
  try { return JSON.stringify(JSON.parse(value), null, 2); }
  catch { return value; }
}

/** 三个步骤共用的标题样式：编号 + 文案，视觉上明确“到哪一步了”。 */
function StepHeading({ step, text }: { step: string; text: string }) {
  return (
    <h4 className="flex items-center gap-2 text-sm font-medium">
      <span className="grid h-5 w-5 shrink-0 place-content-center rounded-full bg-muted text-[11px] tabular-nums text-muted-foreground">
        {step}
      </span>
      <span className="min-w-0 break-words">{text}</span>
    </h4>
  );
}

interface ModelRow {
  label: string;
  oneMMode: OneMMode;
}

interface Props {
  candidates: ClaudeDesktopCandidates | undefined;
  candidatesLoading: boolean;
  candidatesError: unknown;
  /** 即使凭据变更后的候选内容相同，新的查询结果也使检测失效。 */
  candidatesRevision: number;
  initialProviderId: string | null;
  initialModelChoices: ClaudeDesktopModelChoice[];
  initialMode?: "direct" | "router";
  initialRouteChoices?: ClaudeDesktopRouteChoice[];
  /** 后端给出的可选操作；对话框只在直连可用时允许提交。 */
  availableActions: DesktopAction[];
  onClose: () => void;
  onApplied: () => Promise<void>;
}

/**
 * 配置对话框：选择已有卡片 → 模型/能力 → 预览 → 应用。
 *
 * 不索要地址、密钥或余额 token：一切都来自已保存的卡片。
 */
export function ClaudeDesktopDialog({
  candidates,
  candidatesLoading,
  candidatesError,
  candidatesRevision,
  initialProviderId,
  initialModelChoices,
  initialMode = "direct",
  initialRouteChoices = [],
  availableActions,
  onClose,
  onApplied,
}: Props) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"direct" | "router">(initialMode);
  const [routes, setRoutes] = useState<ClaudeDesktopRouteChoice[]>(initialRouteChoices);
  const [rotateGatewayToken, setRotateGatewayToken] = useState(false);
  const [providerId, setProviderId] = useState<string | null>(initialProviderId);
  const [pathId, setPathId] = useState<string | null>(null);
  const [selected, setSelected] = useState<Record<string, ModelRow>>(() => Object.fromEntries(
    initialModelChoices.map(model => [model.modelId, {
      label: model.labelOverride ?? "",
      oneMMode: model.oneMMode,
    }]),
  ));
  const [rebaseline, setRebaseline] = useState(false);
  const [preview, setPreview] = useState<ClaudeDesktopPreview | null>(null);
  const [probe, setProbe] = useState<ClaudeDesktopBearerReport | null>(null);
  const [messages, setMessages] = useState<ClaudeDesktopMessagesReport | null>(null);
  const [busy, setBusy] = useState<null | "preview" | "apply" | "probe" | "messages">(null);
  const [error, setError] = useState<string | null>(null);
  // 递增序号忽略迟到的异步预览/检测结果，避免旧响应覆盖新表单。
  const seq = useRef(0);
  const pendingToken = useRef<number | null>(null);
  const previewRef = useRef<ClaudeDesktopPreview | null>(null);
  const discardPreview = (value: ClaudeDesktopPreview | null) => {
    if (value?.action === "applyRouter") void api.claudeDesktop.cancelPreview(value.revision).catch(() => {});
  };

  // 首次进入时把候选的默认选择固化到表单。
  useEffect(() => {
    if (!candidates) return;
    setProviderId(current => current ?? candidates.selectedProviderId);
    setPathId(current => current ?? candidates.selectedPathId);
  }, [candidates]);

  const pathsChoice = useMemo<ClaudeDesktopPathsChoice>(() => {
    const chosen = candidates?.pathCandidates.find(entry => entry.id === pathId);
    if (!chosen) return {};
    return { normalRoot: chosen.normalRoot, thirdPartyRoot: chosen.thirdPartyRoot };
  }, [candidates, pathId]);

  const candidate = useMemo(
    () => candidates?.providers.find(entry => entry.providerId === providerId) ?? null,
    [candidates, providerId],
  );
  const models = useMemo(
    () => (candidate?.models ?? []).filter(model => model.selectable),
    [candidate],
  );
  const chosenModels = Object.keys(selected);
  const unavailableChoices = chosenModels.filter(modelId => !models.some(model => model.modelId === modelId));
  const excludedModels = (candidate?.models ?? []).filter(model => !model.selectable && !selected[model.modelId]);
  const pathSelected = !!pathsChoice.normalRoot && !!pathsChoice.thirdPartyRoot;
  const routeProviders = candidates?.routeProviders ?? [];
  const routeRoles = candidates?.routeRoles ?? [];
  const routeInputs = routes.map(route => {
    const provider = routeProviders.find(entry => entry.providerId === route.providerId);
    const model = provider?.models.find(entry => entry.modelId === route.modelId);
    return { providerId: route.providerId, eligible: provider?.eligible, name: provider?.name,
      model: model ? { modelId: model.modelId, label: model.label, protocol: model.protocol,
        selectable: model.selectable, supports1m: model.supports1m } : null };
  });
  const routesValid = routes.length > 0 && routes.every(route => {
    const provider = routeProviders.find(entry => entry.providerId === route.providerId);
    const model = provider?.models.find(entry => entry.modelId === route.modelId);
    return routeRoles.some(role => role.role === route.role && role.roleAlias === route.roleAlias)
      && provider?.eligible && model?.selectable && (route.oneMMode !== "auto" || desktopRouteSupports1m(model));
  });
  const action: DesktopAction = mode === "router" ? "applyRouter" : "applyDirect";
  const formSnapshot = JSON.stringify({
    mode, pathId, rebaseline, pathsChoice, detector: candidates?.detector,
    ...(mode === "router" ? { routes, routeInputs, routeRoles, rotateGatewayToken }
      : { providerId, selected, candidatesRevision, candidate }),
  });
  const currentSnapshot = useRef(formSnapshot);
  currentSnapshot.current = formSnapshot;

  // 任意表单输入变化都立即作废旧预览，并让在途响应失效。
  useEffect(() => {
    seq.current += 1;
    discardPreview(previewRef.current);
    previewRef.current = null;
    setPreview(null);
    setProbe(null);
    setMessages(null);
  }, [formSnapshot]);

  // 卸载后的响应也失效；组件状态关闭不取消已经授权的后端操作。
  useEffect(() => () => { seq.current += 1; discardPreview(previewRef.current); previewRef.current = null; }, []);

  if (!candidates) {
    return (
      <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("claudeDesktop.dialog.title")}</DialogTitle>
            <DialogDescription className="sr-only">{t("claudeDesktop.dialog.description")}</DialogDescription>
          </DialogHeader>
          <p role={candidatesError ? "alert" : "status"} className="text-sm text-muted-foreground">
            {candidatesError
              ? t(desktopErrorKey(candidatesError, "claudeDesktop.errors.storage"))
              : candidatesLoading ? t("common.loading") : t("claudeDesktop.dialog.noEligibleProvider")}
          </p>
        </DialogContent>
      </Dialog>
    );
  }

  const requestOf = (): ClaudeDesktopActionRequest => ({
    ...(mode === "router" ? { routes: routes.map(route => ({ ...route,
      labelOverride: route.labelOverride?.trim() ? route.labelOverride : null })), rotateGatewayToken }
      : { providerId,
        models: chosenModels.map(modelId => ({
          modelId,
          labelOverride: selected[modelId]?.label.trim() ? selected[modelId].label : null,
          oneMMode: selected[modelId]?.oneMMode ?? "off",
        })),
      }),
    paths: pathsChoice,
    rebaseline,
  });

  const runPreview = async () => {
    const token = ++seq.current;
    const form = formSnapshot;
    const snapshot = requestOf();
    pendingToken.current = token;
    setBusy("preview");
    setError(null);
    try {
      const result = await api.claudeDesktop.preview(action, snapshot);
      if (token !== seq.current || form !== currentSnapshot.current) { discardPreview(result); return; }
      previewRef.current = result;
      setPreview(result);
    } catch (cause) {
      if (token !== seq.current || form !== currentSnapshot.current) return;
      setPreview(null);
      previewRef.current = null;
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
    } finally {
      if (pendingToken.current === token) {
        pendingToken.current = null;
        setBusy(null);
      }
    }
  };

  const runApply = async () => {
    if (!preview) return;
    const token = ++seq.current;
    pendingToken.current = token;
    setBusy("apply");
    setError(null);
    try {
      await api.claudeDesktop.commit(action, preview.revision, requestOf());
      toast({
        title: t("claudeDesktop.dialog.appliedTitle"),
        description: t("claudeDesktop.dialog.appliedDescription"),
        variant: "success",
      });
      await onApplied();
      onClose();
    } catch (cause) {
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
      // 提交已消费票据；失败后保留表单，下一次重新预览。
      setPreview(null);
      previewRef.current = null;
    } finally {
      pendingToken.current = null;
      setBusy(null);
    }
  };

  const runProbe = async (checkMessages: boolean) => {
    if (!providerId || !pathSelected) return;
    const token = ++seq.current;
    const form = formSnapshot;
    pendingToken.current = token;
    setBusy(checkMessages ? "messages" : "probe");
    setError(null);
    setProbe(null);
    setMessages(null);
    try {
      const outcome = await api.claudeDesktop.probe({
        providerId,
        modelIds: chosenModels,
        paths: pathsChoice,
        checkMessages,
      });
      if (token !== seq.current || form !== currentSnapshot.current) return;
      setProbe(outcome.bearer);
      setMessages(outcome.messages);
    } catch (cause) {
      if (token !== seq.current || form !== currentSnapshot.current) return;
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
    } finally {
      if (pendingToken.current === token) {
        pendingToken.current = null;
        setBusy(null);
      }
    }
  };

  const pickProvider = (id: string) => {
    if (id === providerId) return;
    setProviderId(id);
    setSelected({});
    setProbe(null);
    setMessages(null);
  };

  // 提交按钮不可用时说明原因，而不是只把它变灰。
  const submitBlockedReason = (() => {
    if (busy !== null) return t("claudeDesktop.dialog.busyHint");
    if (!pathSelected) return t("claudeDesktop.dialog.needPaths");
    if (mode === "router") {
      if (!routes.length) return t("claudeDesktop.routing.needRoute");
      if (!routesValid) return t("claudeDesktop.routing.needValidRoutes");
      if (!availableActions.includes("applyRouter")) return t("claudeDesktop.actionReasons.pendingRecovery");
      return null;
    }
    if (!providerId) return t("claudeDesktop.dialog.needProvider");
    if (chosenModels.length === 0) return t("claudeDesktop.dialog.needModels");
    if (!candidate?.eligible) return t("claudeDesktop.dialog.needEligible");
    if (!preview && !availableActions.includes("applyDirect")) {
      return t("claudeDesktop.actionReasons.pendingRecovery");
    }
    return null;
  })();
  const eligible = candidates.providers.filter(entry => entry.eligible);

  return (
    <Dialog open onOpenChange={open => { if (!open && !busy) onClose(); }}>
      <DialogContent className="flex max-h-[90vh] flex-col overflow-hidden sm:max-w-3xl">
        <DialogHeader className="shrink-0">
          <DialogTitle>{t("claudeDesktop.dialog.title")}</DialogTitle>
          <DialogDescription className="sr-only">{t("claudeDesktop.dialog.description")}</DialogDescription>
        </DialogHeader>

        <div data-claude-desktop-scroll className="-mx-1 min-h-0 flex-1 space-y-4 overflow-y-auto px-1">
          {!candidates.detector.installed && (
            <p role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {t("claudeDesktop.notInstalled")}
            </p>
          )}

          {!!candidatesError && (
            <p role="alert" className="text-xs text-destructive">
              {t(desktopErrorKey(candidatesError, "claudeDesktop.errors.storage"))}
            </p>
          )}

          <fieldset disabled={busy !== null} className="min-w-0 space-y-4">
            <div aria-label={t("claudeDesktop.routing.modeLabel")} className="inline-flex rounded-lg border bg-muted/30 p-1">
              {(["direct", "router"] as const).map(value => (
                <button key={value} type="button" aria-pressed={mode === value}
                  onClick={() => setMode(value)}
                  className={`rounded-md px-4 py-1.5 text-xs font-medium transition-colors ${mode === value ? "bg-background text-foreground shadow-sm" : "text-muted-foreground hover:text-foreground"}`}>
                  {t(value === "router" ? "claudeDesktop.routing.modeRouter" : "claudeDesktop.routing.modeDirect")}
                </button>
              ))}
            </div>
            {/* 1. 供应商 */}
            {mode === "direct" && <div className="space-y-2">
              <StepHeading step="1" text={t("claudeDesktop.dialog.stepProvider")} />
              {candidatesLoading && <p className="text-sm text-muted-foreground">{t("common.loading")}</p>}
              {!candidatesLoading && candidates.providers.length === 0 && (
                <p role="status" className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
                  {t("claudeDesktop.card.emptyProviders")}
                </p>
              )}
              <div className="grid gap-2 sm:grid-cols-2">
                {candidates.providers.map(entry => {
                  const active = entry.providerId === providerId;
                  return (
                    <div key={entry.providerId} className="min-w-0 space-y-1">
                      <button
                        type="button"
                        data-desktop-provider-choice
                        aria-pressed={active}
                        disabled={!entry.eligible}
                        onClick={() => pickProvider(entry.providerId)}
                        className={[
                          "w-full",
                          "min-w-0 rounded-lg border px-3 py-2 text-left text-sm transition-colors",
                          active ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50",
                          entry.eligible ? "" : "cursor-not-allowed opacity-60",
                        ].join(" ")}
                      >
                        <span className="flex min-w-0 flex-wrap items-center gap-2">
                          <span className="min-w-0 truncate font-medium">{entry.name}</span>
                          {!entry.enabled && (
                            <Badge variant="secondary" className="text-[10px]">
                              {t("claudeDesktop.dialog.providerDisabledBadge")}
                            </Badge>
                          )}
                        </span>
                        <span className="mt-1 block break-all font-mono text-[11px] text-muted-foreground">
                          {entry.endpoint?.profileBaseUrl}
                        </span>
                      </button>
                      {!entry.enabled && (
                        <p className="break-words text-[11px] text-muted-foreground">{t("claudeDesktop.dialog.providerDisabled")}</p>
                      )}
                      {!entry.eligible && (
                        <p className="break-words text-[11px] text-destructive">
                          {t(reasonKey(entry.reason ?? entry.models.find(model => model.excludeReason)?.excludeReason ?? "provider_not_eligible"))}
                        </p>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>}

            {mode === "router" && <div className="space-y-2">
              <StepHeading step="1" text={t("claudeDesktop.routing.stepRoutes")} />
              <ClaudeDesktopRouteMapping roles={[...routeRoles, ...routes.filter(route => !routeRoles.some(role => role.role === route.role))]}
                providers={routeProviders} choices={routes} onChange={setRoutes} />
              {routeProviders.length === 0 && <p role="status" className="text-xs text-muted-foreground">{t("claudeDesktop.routing.noProviders")}</p>}
              <div className="flex items-center gap-2 pt-1">
                <Checkbox id="desktop-rotate-gateway-token" checked={rotateGatewayToken} onCheckedChange={checked => setRotateGatewayToken(checked === true)} />
                <label htmlFor="desktop-rotate-gateway-token" className="cursor-pointer text-xs">{t("claudeDesktop.routing.rotateToken")}</label>
              </div>
            </div>}

            {/* 路径：多渠道时必须由用户明确选择 */}
            {candidates.pathCandidates.length > 1 && (
              <div className="space-y-2">
                <Label>{t("claudeDesktop.pathsLabel")}</Label>
                <Select value={pathId ?? ""} onValueChange={setPathId}>
                  <SelectTrigger className="text-xs">
                    <SelectValue placeholder={t("claudeDesktop.dialog.needPaths")} />
                  </SelectTrigger>
                  <SelectContent>
                    {candidates.pathCandidates.map(entry => (
                      <SelectItem key={entry.id} value={entry.id}>
                        {entry.normalRoot}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>
            )}

            {/* 2. 模型与能力 */}
            {mode === "direct" && <div className="space-y-2">
              <StepHeading step="2" text={t("claudeDesktop.dialog.stepModels")} />
              {models.length === 0 && (
                <p role="status" className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
                  {t("claudeDesktop.dialog.noModels")}
                </p>
              )}
              {models.length > 0 && (
                <p className="text-[11px] text-muted-foreground">
                  {t("claudeDesktop.dialog.selectedCount", { count: chosenModels.length })}
                </p>
              )}
              <ul className="space-y-2">
                {models.map(model => {
                  const row = selected[model.modelId];
                  const inputId = `desktop-model-${model.modelId}`;
                  return (
                    <li key={model.modelId} className="rounded-lg border px-3 py-2">
                      <div className="flex min-w-0 flex-wrap items-center gap-2">
                        <Checkbox
                          id={inputId}
                          checked={!!row}
                          onCheckedChange={() =>
                            setSelected(current => {
                              const next = { ...current };
                              if (next[model.modelId]) delete next[model.modelId];
                              else next[model.modelId] = { label: "", oneMMode: model.supports1m ? "auto" : "off" };
                              return next;
                            })
                          }
                        />
                        <label
                          htmlFor={inputId}
                          className="min-w-0 flex-1 cursor-pointer truncate font-mono text-xs"
                        >
                          {model.modelId}
                        </label>
                      </div>
                      {row && (
                        <div className="mt-2 flex flex-wrap items-center gap-2 pl-6">
                          <Input
                            value={row.label}
                            maxLength={64}
                            aria-label={t("claudeDesktop.dialog.labelPlaceholder")}
                            placeholder={model.label}
                            className="h-8 min-w-0 flex-1 text-xs"
                            onChange={event =>
                              setSelected(current => ({
                                ...current,
                                [model.modelId]: { ...row, label: event.target.value },
                              }))
                            }
                          />
                          <div className="flex items-center gap-2">
                            <Checkbox
                              id={`desktop-onem-${model.modelId}`}
                              checked={row.oneMMode === "auto"}
                              disabled={!model.supports1m && row.oneMMode !== "auto"}
                              onCheckedChange={checked =>
                                setSelected(current => ({
                                  ...current,
                                  [model.modelId]: { ...row, oneMMode: checked === true ? "auto" : "off" },
                                }))
                              }
                            />
                            <label htmlFor={`desktop-onem-${model.modelId}`} className="cursor-pointer text-xs">
                              {t("claudeDesktop.dialog.oneMOptions")}
                            </label>
                          </div>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
              {unavailableChoices.length > 0 && (
                <div data-desktop-unavailable-choices className="space-y-2">
                  <p className="text-xs font-medium text-destructive">{t("claudeDesktop.dialog.unavailableChoices")}</p>
                  {unavailableChoices.map(modelId => {
                    const previous = selected[modelId];
                    const excluded = candidate?.models.find(model => model.modelId === modelId);
                    return (
                      <div key={modelId} className="flex min-w-0 items-start gap-2 rounded-lg border border-destructive/30 px-3 py-2">
                        <div className="min-w-0 flex-1 space-y-1">
                          <p className="break-all font-mono text-xs">{modelId}</p>
                          {previous.label && previous.label !== modelId && <p className="break-words text-xs">{previous.label}</p>}
                          <p className="text-[11px] text-muted-foreground">
                            {t(reasonKey(excluded?.excludeReason ?? "model_removed"))}
                          </p>
                          <p className="text-[11px] text-muted-foreground">
                            {t("claudeDesktop.dialog.oneMOptions")}: {t(previous.oneMMode === "auto" ? "claudeDesktop.dialog.oneMAuto" : "claudeDesktop.dialog.oneMOff")}
                          </p>
                        </div>
                        <Button
                          type="button"
                          variant="ghost"
                          size="sm"
                          aria-label={t("claudeDesktop.dialog.removeUnavailableModel", { model: modelId })}
                          onClick={() => setSelected(current => {
                            const next = { ...current };
                            delete next[modelId];
                            return next;
                          })}
                        >
                          {t("claudeDesktop.dialog.removeSelection")}
                        </Button>
                      </div>
                    );
                  })}
                </div>
              )}
              {excludedModels.length > 0 && (
                <div data-desktop-excluded-models className="space-y-1 rounded-lg border bg-muted/20 px-3 py-2">
                  <p className="text-xs font-medium">{t("claudeDesktop.dialog.modelExcluded")}</p>
                  {excludedModels.map(model => (
                    <p key={model.modelId} className="break-all text-[11px] text-muted-foreground">
                      <span className="font-mono">{model.modelId}</span>{" · "}{t(reasonKey(model.excludeReason))}
                    </p>
                  ))}
                </div>
              )}
            </div>}

            <div className="flex flex-wrap items-center gap-2">
              <Checkbox
                id="desktop-rebaseline"
                checked={rebaseline}
                onCheckedChange={value => setRebaseline(value === true)}
              />
              <label htmlFor="desktop-rebaseline" className="cursor-pointer text-xs">
                {t("claudeDesktop.dialog.rebaseline")}
              </label>
            </div>

            {mode === "direct" && <div className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!providerId || !pathSelected}
                  onClick={() => void runProbe(false)}
                >
                  {busy === "probe" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <ShieldAlert className="h-3.5 w-3.5" />
                  )}
                  {t(busy === "probe" ? "claudeDesktop.dialog.probing" : "claudeDesktop.dialog.probe")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={!providerId || !pathSelected || chosenModels.length === 0}
                  title={t("claudeDesktop.dialog.messagesCheckCost")}
                  onClick={() => void runProbe(true)}
                >
                  {busy === "messages" ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : (
                    <MessageSquare className="h-3.5 w-3.5" />
                  )}
                  {t(
                    busy === "messages"
                      ? "claudeDesktop.dialog.messagesChecking"
                      : "claudeDesktop.dialog.messagesCheck",
                  )}
                </Button>
              </div>
              <p className="text-[11px] text-muted-foreground">
                {t("claudeDesktop.dialog.messagesCheckCost")}
              </p>
            </div>}

            {(probe || messages) && (
              <div className="space-y-2 rounded-lg border bg-muted/20 px-3 py-2">
                <p className="text-xs font-medium">{t("claudeDesktop.dialog.connectivityTitle")}</p>

                {probe && (
                  <div className="space-y-0.5">
                    <p className="flex flex-wrap items-center gap-2 text-xs">
                      {probe.status === "modelsAccessible" ? (
                        <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-600" />
                      ) : (
                        <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-600" />
                      )}
                      <span className="text-muted-foreground">
                        {t("claudeDesktop.dialog.modelsCheckLabel")}
                      </span>
                      <span className="min-w-0 break-words">{t(`claudeDesktop.probe.${probe.status}`)}</span>
                      {probe.httpStatus !== null && (
                        <Badge variant="outline" className="text-[10px]">{probe.httpStatus}</Badge>
                      )}
                    </p>
                    <p className="break-all pl-5 font-mono text-[11px] text-muted-foreground">{probe.url}</p>
                    {probe.note && (
                      <p className="pl-5 text-[11px] text-muted-foreground">{t(probe.note)}</p>
                    )}
                    {probe.modelsMissing.length > 0 && (
                      <p className="break-words pl-5 text-[11px] text-amber-600 dark:text-amber-500">
                        {t("claudeDesktop.dialog.probeMissingModels")}: {probe.modelsMissing.join(", ")}
                      </p>
                    )}
                  </div>
                )}

                {messages && (
                  <div className="space-y-0.5">
                    <p className="flex flex-wrap items-center gap-2 text-xs">
                      {messages.status === "messagesAccessible" ? (
                        <CheckCircle2 className="h-3.5 w-3.5 shrink-0 text-emerald-600" />
                      ) : (
                        <AlertTriangle className="h-3.5 w-3.5 shrink-0 text-amber-600" />
                      )}
                      <span className="text-muted-foreground">
                        {t("claudeDesktop.dialog.messagesCheckLabel")}
                      </span>
                      <span className="min-w-0 break-words">
                        {t(`claudeDesktop.messages.${messages.status}`)}
                      </span>
                      {messages.httpStatus !== null && (
                        <Badge variant="outline" className="text-[10px]">{messages.httpStatus}</Badge>
                      )}
                    </p>
                    <p className="break-all pl-5 font-mono text-[11px] text-muted-foreground">{messages.url}</p>
                    {messages.responseModel && (
                      <p className="break-all pl-5 text-[11px] text-muted-foreground">
                        {t("claudeDesktop.dialog.messagesResponseModel")}: {messages.responseModel}
                      </p>
                    )}
                    {messages.note && (
                      <p className="pl-5 text-[11px] text-muted-foreground">{messages.note}</p>
                    )}
                  </div>
                )}

              </div>
            )}

            {/* 3. 预览 */}
            <div className="space-y-2">
              <StepHeading step={mode === "router" ? "2" : "3"} text={t("claudeDesktop.dialog.stepPreview")} />
              {preview && (
                <div className="space-y-2">
                  <div data-desktop-preview-files className="space-y-3 text-xs">
                      {preview.files.map(file => (
                        <div key={file.role} className="space-y-2 border-t pt-3">
                          <div className="flex flex-wrap items-center gap-2">
                            <span className="font-medium">{t(`claudeDesktop.fileRoles.${file.role}`)}</span>
                            <Badge variant="outline" className="text-[10px]">{t(`claudeDesktop.fileKinds.${file.kind}`)}</Badge>
                          </div>
                          <p className="break-all font-mono text-[11px] text-muted-foreground">{file.path}</p>
                          <dl className="space-y-2">
                            {file.managedChanges.map(change => (
                              <div key={change.path} data-desktop-managed-change={change.path} className="space-y-1">
                                <dt className="break-all font-mono text-[11px]">{change.path}</dt>
                                <dd className="grid gap-2 sm:grid-cols-2">
                                  {(["before", "after"] as const).map(side => (
                                    <div key={side} data-desktop-change-side={side} className="min-w-0 bg-muted/40 px-2 py-1.5">
                                      <p className="mb-1 text-[11px] text-muted-foreground">{t(`claudeDesktop.dialog.${side}`)}</p>
                                      <pre className="whitespace-pre-wrap break-all font-mono text-[11px]">{formatManagedValue(change[side])}</pre>
                                    </div>
                                  ))}
                                </dd>
                              </div>
                            ))}
                          </dl>
                        </div>
                      ))}
                      {preview.externalProfileCount > 0 && (
                        <p className="text-muted-foreground">{t("claudeDesktop.dialog.externalProfiles")}: {preview.externalProfileCount}</p>
                      )}
                  </div>
                  {preview.warnings.filter(key => key !== "claudeDesktop.warnings.requiresRestart").map(key => (
                    <p
                      key={key}
                      role="status"
                      className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-muted-foreground"
                    >
                      {t(key)}
                    </p>
                  ))}
                </div>
              )}
            </div>

            <details className="text-xs text-muted-foreground">
              <summary className="w-fit cursor-pointer rounded-sm hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring">{t("claudeDesktop.dialog.environmentDetails")}</summary>
              <div className="mt-2 space-y-1">
              <Label>{t("claudeDesktop.versionLabel")}</Label>
              <p className="break-all font-mono text-[11px] text-muted-foreground">
                {candidates.detector.version === "unknown"
                  ? t("claudeDesktop.versionUnknown")
                  : candidates.detector.version}
                {" · "}
                {t(candidates.detector.versionSource)}
              </p>
              {candidates.detector.notes.map(note => (
              <p key={note} role="status" className="text-[11px] text-muted-foreground">
                {t(note)}
              </p>
              ))}
              </div>
            </details>

            {mode === "direct" && eligible.length === 0 && (
              <p role="status" className="text-[11px] text-muted-foreground">
                {t("claudeDesktop.dialog.noEligibleProvider")}
              </p>
            )}
          </fieldset>

          {error && (
            <p role="alert" className="text-sm text-destructive">
              {t(error)}
            </p>
          )}
        </div>

        <DialogFooter className="shrink-0 gap-2">
          <Button variant="outline" onClick={onClose} disabled={busy !== null}>
            {t("common.cancel")}
          </Button>
          {submitBlockedReason && (
            <p role="status" className="mr-auto max-w-[60%] text-[11px] text-muted-foreground">
              {submitBlockedReason}
            </p>
          )}
          <Button
            disabled={submitBlockedReason !== null || (mode === "direct" && unavailableChoices.length > 0)}
            onClick={() => void (preview ? runApply() : runPreview())}
          >
            {busy === "preview" || busy === "apply" ? (
              <Loader2 className="h-4 w-4 animate-spin" />
            ) : (
              <Eye className="h-4 w-4" />
            )}
            {t(
              busy === "apply"
                ? "claudeDesktop.dialog.applying"
                : preview
                  ? "claudeDesktop.dialog.apply"
                  : "claudeDesktop.dialog.preview",
            )}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
