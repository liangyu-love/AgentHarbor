import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "@/lib/i18n";
import { AlertTriangle, CheckCircle2, Eye, Loader2, MessageSquare, ShieldAlert } from "lucide-react";
import { api } from "@/lib/api";
import { desktopErrorKey, desktopErrorOf } from "@/lib/claude-desktop";
import { toast } from "@/hooks/use-toast";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
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
  ClaudeDesktopPreview,
  DesktopAction,
  OneMMode,
} from "@/types/claude-desktop";

/**
 * 后端原因码是 snake_case（`provider_missing`、`unsafe_model_name`），
 * 本地化 key 是 camelCase。`code:param,param` 形态只取冒号前的码，
 * 避免把地址等参数拼进 key。
 */
function reasonKey(reason: string | null | undefined): string {
  const code = (reason ?? "").split(":")[0]?.trim();
  if (!code) return "claudeDesktop.errors.storage";
  const camel = code.replace(/_([a-z0-9])/g, (_, ch: string) => ch.toUpperCase());
  const key = `claudeDesktop.errors.${camel}`;
  return i18n.exists(key) ? key : "claudeDesktop.errors.storage";
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
  initialProviderId: string | null;
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
  initialProviderId,
  availableActions,
  onClose,
  onApplied,
}: Props) {
  const { t } = useTranslation();
  const [providerId, setProviderId] = useState<string | null>(initialProviderId);
  const [pathId, setPathId] = useState<string | null>(null);
  const [selected, setSelected] = useState<Record<string, ModelRow>>({});
  const [rebaseline, setRebaseline] = useState(false);
  const [preview, setPreview] = useState<ClaudeDesktopPreview | null>(null);
  const [probe, setProbe] = useState<ClaudeDesktopBearerReport | null>(null);
  const [messages, setMessages] = useState<ClaudeDesktopMessagesReport | null>(null);
  const [busy, setBusy] = useState<null | "preview" | "apply" | "probe" | "messages">(null);
  const [error, setError] = useState<string | null>(null);
  // 递增序号忽略迟到的异步预览/检测结果，避免旧响应覆盖新表单。
  const seq = useRef(0);

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

  // 任意表单输入变化都立即作废旧预览，并让在途响应失效。
  useEffect(() => {
    seq.current += 1;
    setPreview(null);
  }, [providerId, pathId, selected, rebaseline]);

  if (!candidates) {
    return (
      <Dialog open onOpenChange={open => { if (!open) onClose(); }}>
        <DialogContent className="sm:max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("claudeDesktop.dialog.title")}</DialogTitle>
            <DialogDescription>{t("claudeDesktop.dialog.description")}</DialogDescription>
          </DialogHeader>
          <p className="text-sm text-muted-foreground">
            {candidatesLoading ? t("common.loading") : t("claudeDesktop.dialog.noEligibleProvider")}
          </p>
        </DialogContent>
      </Dialog>
    );
  }

  const requestOf = (): ClaudeDesktopActionRequest => ({
    providerId,
    models: chosenModels.map(modelId => ({
      modelId,
      labelOverride: selected[modelId]?.label.trim() ? selected[modelId].label : null,
      oneMMode: selected[modelId]?.oneMMode ?? "auto",
    })),
    paths: pathsChoice,
    rebaseline,
  });

  const runPreview = async () => {
    const token = ++seq.current;
    const snapshot = requestOf();
    setBusy("preview");
    setError(null);
    try {
      const result = await api.claudeDesktop.preview("applyDirect", snapshot);
      if (token !== seq.current) return;
      setPreview(result);
    } catch (cause) {
      if (token !== seq.current) return;
      setPreview(null);
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
    } finally {
      if (token === seq.current) setBusy(null);
    }
  };

  const runApply = async () => {
    if (!preview) return;
    const token = seq.current;
    setBusy("apply");
    setError(null);
    try {
      await api.claudeDesktop.commit("applyDirect", preview.revision, requestOf());
      if (token !== seq.current) return;
      toast({
        title: t("claudeDesktop.dialog.appliedTitle"),
        description: t("claudeDesktop.dialog.appliedDescription"),
        variant: "success",
      });
      await onApplied();
      onClose();
    } catch (cause) {
      const structured = desktopErrorOf(cause);
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
      // `changed` / 过期：保留表单，要求重新预览，不能复用旧 ticket。
      if (structured?.code === "changed" || structured?.code === "preview_expired") {
        setPreview(null);
      }
    } finally {
      setBusy(null);
    }
  };

  const runProbe = async (checkMessages: boolean) => {
    if (!providerId) return;
    setBusy(checkMessages ? "messages" : "probe");
    setError(null);
    try {
      const outcome = await api.claudeDesktop.probe({
        providerId,
        modelIds: chosenModels,
        paths: pathsChoice,
        checkMessages,
      });
      setProbe(outcome.bearer);
      if (outcome.messages) setMessages(outcome.messages);
    } catch (cause) {
      setError(desktopErrorKey(cause, "claudeDesktop.errors.storage"));
    } finally {
      setBusy(null);
    }
  };

  const pickProvider = (id: string) => {
    setProviderId(id);
    setSelected({});
    setProbe(null);
  };

  const endpoint = candidate?.endpoint ?? null;
  // 提交按钮不可用时说明原因，而不是只把它变灰。
  const submitBlockedReason = (() => {
    if (busy !== null) return t("claudeDesktop.dialog.busyHint");
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
          <DialogDescription>{t("claudeDesktop.dialog.description")}</DialogDescription>
        </DialogHeader>

        <div data-claude-desktop-scroll className="-mx-1 min-h-0 flex-1 space-y-4 overflow-y-auto px-1">
          <p role="status" className="rounded-lg border bg-muted/40 px-3 py-2 text-xs text-muted-foreground">
            {t("claudeDesktop.banner.independent")}
          </p>

          {!candidates.detector.installed && (
            <p role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-xs text-destructive">
              {t("claudeDesktop.notInstalled")}
            </p>
          )}

          <fieldset disabled={busy !== null} className="min-w-0 space-y-4">
            {/* 1. 供应商 */}
            <div className="space-y-2">
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
                    <button
                      key={entry.providerId}
                      type="button"
                      aria-pressed={active}
                      disabled={!entry.eligible}
                      onClick={() => pickProvider(entry.providerId)}
                      className={[
                        "min-w-0 rounded-lg border px-3 py-2 text-left text-sm transition-colors",
                        active ? "border-primary bg-primary/5" : "border-border hover:bg-muted/50",
                        entry.eligible ? "" : "cursor-not-allowed opacity-60",
                      ].join(" ")}
                    >
                      <span className="flex min-w-0 flex-wrap items-center gap-2">
                        <span className="min-w-0 truncate font-medium">{entry.name}</span>
                        {!entry.enabled && (
                          <Badge variant="secondary" className="shrink-0 text-[10px]">
                            {t("claudeDesktop.dialog.providerDisabled")}
                          </Badge>
                        )}
                      </span>
                      <span className="mt-1 block break-all font-mono text-[11px] text-muted-foreground">
                        {entry.endpoint?.profileBaseUrl ?? t(reasonKey(entry.reason))}
                      </span>
                    </button>
                  );
                })}
              </div>
              <p className="text-xs text-muted-foreground">{t("claudeDesktop.dialog.savedKeyHint")}</p>
            </div>

            {/* 路径：多渠道时必须由用户明确选择 */}
            {candidates.pathCandidates.length > 1 && (
              <div className="space-y-2">
                <Label>{t("claudeDesktop.pathsLabel")}</Label>
                <Select value={pathId ?? ""} onValueChange={setPathId}>
                  <SelectTrigger className="text-xs">
                    <SelectValue placeholder={candidates.paths.normalRoot} />
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

            {endpoint && (
              <div className="space-y-1 rounded-lg border bg-card/60 px-3 py-2">
                <p className="break-all font-mono text-[11px]">{endpoint.messagesUrl}</p>
                <p className="text-[11px] text-muted-foreground">{t("claudeDesktop.dialog.previewHint")}</p>
              </div>
            )}

            {/* 2. 模型与能力 */}
            <div className="space-y-2">
              <StepHeading step="2" text={t("claudeDesktop.dialog.stepModels")} />
              <p className="text-xs text-muted-foreground">{t("claudeDesktop.dialog.modelsHint")}</p>
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
                              else next[model.modelId] = { label: model.label, oneMMode: "auto" };
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
                        {!model.protocolVerified && (
                          <Badge variant="outline" className="shrink-0 text-[10px]">
                            {t("claudeDesktop.dialog.protocolInferred")}
                          </Badge>
                        )}
                        {model.supports1m && (
                          <Badge variant="secondary" className="shrink-0 text-[10px]">1M</Badge>
                        )}
                      </div>
                      {row && (
                        <div className="mt-2 flex flex-wrap items-center gap-2 pl-6">
                          <Input
                            value={row.label}
                            maxLength={64}
                            aria-label={t("claudeDesktop.dialog.labelPlaceholder")}
                            placeholder={t("claudeDesktop.dialog.labelPlaceholder")}
                            className="h-8 min-w-0 flex-1 text-xs"
                            onChange={event =>
                              setSelected(current => ({
                                ...current,
                                [model.modelId]: { ...row, label: event.target.value },
                              }))
                            }
                          />
                          <Select
                            value={row.oneMMode}
                            onValueChange={value =>
                              setSelected(current => ({
                                ...current,
                                [model.modelId]: { ...row, oneMMode: value as OneMMode },
                              }))
                            }
                          >
                            <SelectTrigger className="h-8 w-36 text-xs">
                              <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                              <SelectItem value="auto">{t("claudeDesktop.dialog.oneMAuto")}</SelectItem>
                              <SelectItem value="off">{t("claudeDesktop.dialog.oneMOff")}</SelectItem>
                            </SelectContent>
                          </Select>
                        </div>
                      )}
                    </li>
                  );
                })}
              </ul>
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <Checkbox
                id="desktop-rebaseline"
                checked={rebaseline}
                onCheckedChange={value => setRebaseline(value === true)}
              />
              <label htmlFor="desktop-rebaseline" className="cursor-pointer text-xs">
                {t("claudeDesktop.dialog.rebaseline")}
              </label>
              <span className="text-[11px] text-muted-foreground">
                {t("claudeDesktop.dialog.rebaselineHint")}
              </span>
            </div>

            <div className="space-y-2">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  disabled={!providerId}
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
                  disabled={!providerId || chosenModels.length === 0}
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
              <p className="text-[11px] text-muted-foreground">{t("claudeDesktop.dialog.probeHint")}</p>
              <p className="text-[11px] text-muted-foreground">
                {t("claudeDesktop.dialog.messagesCheckCost")}
              </p>
            </div>

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

                <p className="text-[11px] text-muted-foreground">
                  {t("claudeDesktop.dialog.messagesCheckResultHint")}
                </p>
              </div>
            )}

            {/* 3. 预览 */}
            <div className="space-y-2">
              <StepHeading step="3" text={t("claudeDesktop.dialog.stepPreview")} />
              {!preview && <p className="text-xs text-muted-foreground">{t("claudeDesktop.dialog.previewHint")}</p>}
              {preview && (
                <div className="space-y-2">
                  {preview.messagesUrl && (
                    <p className="break-all font-mono text-[11px] text-muted-foreground">
                      {preview.messagesUrl}
                    </p>
                  )}
                  {preview.files.map(file => (
                    <div key={file.role} className="rounded-lg border px-3 py-2">
                      <div className="flex flex-wrap items-center gap-2">
                        <Badge variant="outline" className="text-[10px]">{file.kind}</Badge>
                        <span className="text-xs font-medium">{file.role}</span>
                      </div>
                      <p className="mt-1 break-all font-mono text-[11px] text-muted-foreground">{file.path}</p>
                      <ul className="mt-1 space-y-0.5">
                        {file.managedChanges.map(change => (
                          <li key={change.path} className="break-all font-mono text-[11px]">
                            <span className="text-muted-foreground">{change.path}</span>
                            {": "}
                            <span className="text-muted-foreground">{change.before ?? "—"}</span>
                            {" → "}
                            <span>{change.after ?? "—"}</span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  ))}
                  {preview.externalProfileCount > 0 && (
                    <p className="text-[11px] text-muted-foreground">
                      {t("claudeDesktop.dialog.externalProfiles")}: {preview.externalProfileCount}
                    </p>
                  )}
                  {preview.warnings.map(key => (
                    <p
                      key={key}
                      role="status"
                      className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-muted-foreground"
                    >
                      {t(key)}
                    </p>
                  ))}
                  {preview.requiresRestart && (
                    <p role="status" className="text-[11px] text-muted-foreground">
                      {t("claudeDesktop.dialog.requiresRestart")}
                    </p>
                  )}
                </div>
              )}
            </div>

            <div className="space-y-1">
              <Label>{t("claudeDesktop.versionLabel")}</Label>
              <p className="break-all font-mono text-[11px] text-muted-foreground">
                {candidates.detector.version === "unknown"
                  ? t("claudeDesktop.versionUnknown")
                  : candidates.detector.version}
                {" · "}
                {t(candidates.detector.versionSource)}
              </p>
            </div>

            {candidates.detector.notes.map(note => (
              <p key={note} role="status" className="text-[11px] text-muted-foreground">
                {t(note)}
              </p>
            ))}

            {eligible.length === 0 && (
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
            disabled={submitBlockedReason !== null}
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
