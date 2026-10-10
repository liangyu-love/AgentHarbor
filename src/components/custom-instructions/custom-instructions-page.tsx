import { useEffect, useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  FileCode2,
  History,
  RotateCw,
  Search,
  Save,
} from "lucide-react";
import { PageHero } from "@/components/layout/page-hero";
import { BentoCard } from "@/components/ui/bento-card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Badge } from "@/components/ui/badge";
import { SegmentedOptions } from "@/components/ui/segmented-options";
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
import { api } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import {
  instructionTemplates,
  loadSavedTemplates,
  saveTemplates,
  type CustomInstructionTemplate,
} from "@/lib/custom-instruction-templates";
import { toast } from "@/hooks/use-toast";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import type {
  CustomInstructionPreviewPayload,
  CustomInstructionStatePayload,
} from "@/types";
import { HistoryList } from "./history-list";
import { PreviewDialog } from "./preview-dialog";
import { TemplateCard } from "./template-card";

type Source = "codex" | "claudeCode";
type Apply = {
  content: string;
  templateCode?: string;
  templateTitle?: string;
  source: string;
};

export function CustomInstructionsPage() {
  const [source, setSource] = useState<Source>(() =>
    localStorage.getItem(storageKeys.instructionSource) === "claudeCode"
      ? "claudeCode"
      : "codex",
  );
  const [busy, setBusy] = useState(false);
  return (
    <div className="space-y-4">
      <SegmentedOptions
        value={source}
        items={[
          { value: "codex", label: "Codex", disabled: busy },
          { value: "claudeCode", label: "Claude Code", disabled: busy },
        ]}
        onChange={(value) => {
          localStorage.setItem(storageKeys.instructionSource, value);
          setSource(value as Source);
        }}
      />
      <InstructionsWorkspace key={source} source={source} onBusy={setBusy} />
    </div>
  );
}

function InstructionsWorkspace({
  source,
  onBusy,
}: {
  source: Source;
  onBusy: (busy: boolean) => void;
}) {
  const { t, i18n } = useTranslation();
  const queryClient = useQueryClient();
  const stateKey = [...queryKeys.customInstructionState, source];
  const draftKey = `agentharbor.instruction-draft.${source}`;
  const [tab, setTab] = useState("configure");
  const [draft, setDraft] = useState<string | null>(() =>
    localStorage.getItem(draftKey),
  );
  const [selected, setSelected] = useState<CustomInstructionTemplate | null>(
    null,
  );
  const [saved, setSaved] = useState(() => loadSavedTemplates(source));
  const [presetName, setPresetName] = useState("");
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("all");
  const [preview, setPreview] =
    useState<CustomInstructionPreviewPayload | null>(null);
  const [pending, setPending] = useState<Apply | null>(null);
  const [clearOpen, setClearOpen] = useState(false);
  const stateQuery = useQuery({
    queryKey: stateKey,
    queryFn: () => api.loadCustomInstructionState(source),
  });
  const state = stateQuery.data?.data;
  const current = state?.current;
  const content = draft ?? current?.managedContent ?? "";
  const protectedMode = current?.protectionState === "protected";
  const fail = (error: unknown) =>
    toast({
      title: t("common.error"),
      description: formatInvokeError(error),
      variant: "destructive",
    });
  const edit = (value: string) => {
    setDraft(value);
    localStorage.setItem(draftKey, value);
  };
  const sync = (payload: CustomInstructionStatePayload) => {
    queryClient.setQueryData(stateKey, { ...stateQuery.data, data: payload });
    setDraft(null);
    localStorage.removeItem(draftKey);
    setSelected(null);
  };
  const previewMutation = useMutation({
    mutationFn: (params: Apply) =>
      api.previewCustomInstructionApply(params.content, source),
    onSuccess: (response) => setPreview(response.data),
    onError: (error) => {
      setPending(null);
      fail(error);
    },
  });
  const applyMutation = useMutation({
    mutationFn: (params: Apply) =>
      api.applyCustomInstruction(
        { ...params, expectedContent: preview?.currentManagedContent },
        source,
      ),
    onSuccess: (response) => {
      sync(response.data);
      setPreview(null);
      setPending(null);
      toast({
        title: t("customInstructions.applySuccess"),
        variant: "success",
      });
    },
    onError: fail,
  });
  const clearMutation = useMutation({
    mutationFn: () => api.clearCustomInstructionBlock(source),
    onSuccess: (response) => {
      sync(response.data);
      setClearOpen(false);
      toast({
        title: t("customInstructions.clearSuccess"),
        variant: "success",
      });
    },
    onError: fail,
  });
  const rollbackMutation = useMutation({
    mutationFn: (id: string) => api.rollbackCustomInstruction(id, source),
    onSuccess: (response) => {
      sync(response.data);
      toast({
        title: t("customInstructions.rollbackSuccess"),
        variant: "success",
      });
    },
    onError: fail,
  });
  const busy =
    previewMutation.isPending ||
    applyMutation.isPending ||
    clearMutation.isPending ||
    rollbackMutation.isPending;
  useEffect(() => {
    onBusy(busy || preview !== null || clearOpen);
    return () => onBusy(false);
  }, [busy, preview, clearOpen, onBusy]);
  const templates = useMemo(() => {
    const counts = new Map<string, number>();
    for (const item of state?.history ?? [])
      if (item.action === "apply" && item.templateCode)
        counts.set(item.templateCode, (counts.get(item.templateCode) ?? 0) + 1);
    return [...instructionTemplates(i18n.language), ...saved].map((item) => ({
      ...item,
      applyCount: counts.get(item.code) ?? 0,
    }));
  }, [i18n.language, saved, state?.history]);
  const filtered = templates.filter(
    (item) =>
      (category === "all" ||
        (category === "saved"
          ? item.source === "saved"
          : item.category === category)) &&
      `${item.title} ${item.summary} ${item.tags.join(" ")}`
        .toLowerCase()
        .includes(search.trim().toLowerCase()),
  );
  const beginPreview = (params: Apply) => {
    if (!current || busy || protectedMode) return;
    setPending(params);
    previewMutation.mutate(params);
  };
  const savePreset = () => {
    if (!presetName.trim() || !content.trim() || saved.length >= 40) return;
    const next = [
      ...saved,
      {
        code: `saved-${crypto.randomUUID()}`,
        title: presetName.trim(),
        summary:
          content
            .split("\n")
            .find((line) => line.trim())
            ?.slice(0, 100) ?? "",
        body: content,
        tags: [],
        source: "saved" as const,
        category: "saved",
      },
    ];
    try {
      saveTemplates(source, next);
      setSaved(next);
      setPresetName("");
      toast({ title: t("customInstructions.presetSaved"), variant: "success" });
    } catch (error) {
      fail(error);
    }
  };

  return (
    <div className="space-y-5" data-instruction-workspace={source}>
      <PageHero kicker="Instructions" icon={<FileCode2 />} title={t("customInstructions.title")}
        description={t("customInstructions.description")} actions={<>
          <SegmentedOptions
            value={tab}
            onChange={setTab}
            items={[
              {
                value: "configure",
                label: t("customInstructions.configureTab"),
              },
              {
                value: "templates",
                label: t("customInstructions.templatesTab"),
              },
            ]}
          />
          <Button
            variant="outline"
            size="icon"
            aria-label={t("common.refresh")}
            disabled={busy || stateQuery.isFetching}
            onClick={() => void stateQuery.refetch()}
          >
            <RotateCw className="h-4 w-4" />
          </Button>
      </>} />
      {stateQuery.isError ? (
        <BentoCard role="alert">
          <p>{t("customInstructions.loadFailed")}</p>
          <p className="mt-2 text-sm text-muted-foreground">
            {formatInvokeError(stateQuery.error)}
          </p>
          <Button className="mt-3" onClick={() => void stateQuery.refetch()}>
            {t("threads.retry")}
          </Button>
        </BentoCard>
      ) : stateQuery.isPending ? (
        <p role="status">{t("baseline.loading")}</p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <FileCode2 className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="min-w-0 break-all font-mono text-xs text-muted-foreground">
              {current?.globalPath}
            </span>
            <Badge variant={protectedMode ? "destructive" : "outline"}>
              {t(
                protectedMode
                  ? "customInstructions.stateProtected"
                  : current?.managedBlockPresent
                    ? "customInstructions.stateManaged"
                    : "customInstructions.stateUnmanaged",
              )}
            </Badge>
          </div>
          {protectedMode && (
            <div role="alert" className="flex gap-2 text-sm text-destructive">
              <AlertTriangle className="h-4 w-4 shrink-0" />
              {current?.issueMessage}
            </div>
          )}
          <BentoCard>
            <h2 id={`instruction-current-${source}`} className="mb-3 font-semibold">
              {t("customInstructions.currentTitle")}
            </h2>
            {current?.fileContent ? (
              <pre
                aria-labelledby={`instruction-current-${source}`}
                data-instruction-current
                className="max-h-64 overflow-y-auto whitespace-pre-wrap break-words rounded-xl bg-muted/40 p-4 font-mono text-sm [overflow-wrap:anywhere]"
              >
                {current.fileContent}
              </pre>
            ) : (
              <p className="text-sm text-muted-foreground">
                {t("customInstructions.emptyFile")}
              </p>
            )}
          </BentoCard>
          {tab === "templates" ? (
            <BentoCard>
              <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                <div
                  className="flex flex-wrap gap-1"
                  role="group"
                  aria-label={t("customInstructions.categoryLabel")}
                >
                  {["all", "development", "review", "writing", "saved"].map(
                    (value) => (
                      <Button
                        key={value}
                        size="sm"
                        variant={category === value ? "soft" : "ghost"}
                        aria-pressed={category === value}
                        onClick={() => setCategory(value)}
                      >
                        {t(`customInstructions.categories.${value}`)}
                      </Button>
                    ),
                  )}
                </div>
                <div className="relative w-full sm:w-64">
                  <Search className="absolute left-3 top-3 h-4 w-4 text-muted-foreground" />
                  <Input
                    aria-label={t("customInstructions.searchTemplates")}
                    placeholder={t("customInstructions.searchTemplates")}
                    className="pl-9"
                    value={search}
                    onChange={(event) => setSearch(event.target.value)}
                  />
                </div>
              </div>
              {filtered.length ? (
                <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
                  {filtered.map((template) => (
                    <TemplateCard
                      key={template.code}
                      template={template}
                      isSelected={selected?.code === template.code}
                      disabled={busy || protectedMode}
                      onSelect={() => {
                        setSelected(template);
                        edit(template.body);
                        setTab("configure");
                      }}
                      onPreview={() =>
                        beginPreview({
                          content: template.body,
                          templateCode: template.code,
                          templateTitle: template.title,
                          source: "template",
                        })
                      }
                      onDelete={
                        template.source === "saved"
                          ? () => {
                              try {
                                const next = saved.filter(
                                  (item) => item.code !== template.code,
                                );
                                saveTemplates(source, next);
                                setSaved(next);
                              } catch (error) {
                                fail(error);
                              }
                            }
                          : undefined
                      }
                    />
                  ))}
                </div>
              ) : (
                <p className="py-12 text-center text-sm text-muted-foreground">
                  {t("customInstructions.noTemplates")}
                </p>
              )}
            </BentoCard>
          ) : (
            <>
              <BentoCard>
                <div className="mb-4 flex flex-wrap items-center justify-between gap-3">
                  <label
                    htmlFor={`instruction-editor-${source}`}
                    className="font-medium"
                  >
                    {t("customInstructions.editorTitle")}
                  </label>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => setTab("templates")}
                  >
                    {t("customInstructions.chooseTemplate")}
                  </Button>
                </div>
                {selected && (
                  <p className="mb-3 text-xs text-muted-foreground">
                    {t("customInstructions.selectedTemplate", {
                      name: selected.title,
                    })}
                  </p>
                )}
                <Textarea
                  id={`instruction-editor-${source}`}
                  value={content}
                  onChange={(event) => {
                    edit(event.target.value);
                    setSelected(null);
                  }}
                  disabled={protectedMode || busy}
                  placeholder={t("customInstructions.editorPlaceholder")}
                  className="min-h-[320px] resize-y font-mono text-sm leading-relaxed"
                />
                <div className="mt-4 flex flex-wrap items-center justify-between gap-3">
                  <div className="flex flex-wrap gap-2">
                    <Button
                      disabled={
                        busy ||
                        protectedMode ||
                        !content.trim() ||
                        content === current?.managedContent
                      }
                      onClick={() =>
                        beginPreview({
                          content,
                          templateCode: selected?.code,
                          templateTitle: selected?.title,
                          source: selected ? "edit_then_apply" : "manual",
                        })
                      }
                    >
                      {t("customInstructions.previewAndApply")}
                    </Button>
                    <Button
                      variant="outline"
                      disabled={busy}
                      onClick={() => {
                        setDraft(null);
                        localStorage.removeItem(draftKey);
                        setSelected(null);
                      }}
                    >
                      {t("customInstructions.restoreCurrent")}
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={busy || !current?.fileExists}
                      onClick={() => {
                        if (current)
                          void api.openPath(current.globalPath).catch(fail);
                      }}
                    >
                      {t("customInstructions.openGlobalFile")}
                    </Button>
                    <Button
                      variant="ghost"
                      disabled={
                        busy || protectedMode || !current?.managedBlockPresent
                      }
                      onClick={() => setClearOpen(true)}
                    >
                      {t("customInstructions.clearManagedBlock")}
                    </Button>
                  </div>
                  <span className="text-xs tabular-nums text-muted-foreground">
                    {t(
                      current?.managedBlockPresent &&
                        content === current.managedContent
                        ? "customInstructions.synced"
                        : content.trim()
                          ? "customInstructions.unsaved"
                          : "customInstructions.noManagedContent",
                    )}
                  </span>
                </div>
                <div className="mt-5 flex flex-wrap items-center gap-2 border-t border-border pt-4">
                  <Input
                    value={presetName}
                    maxLength={80}
                    onChange={(event) => setPresetName(event.target.value)}
                    aria-label={t("customInstructions.presetName")}
                    placeholder={t("customInstructions.presetName")}
                    className="w-full sm:w-64"
                  />
                  <Button
                    variant="outline"
                    disabled={
                      !presetName.trim() ||
                      !content.trim() ||
                      saved.length >= 40
                    }
                    onClick={savePreset}
                  >
                    <Save className="h-4 w-4" />
                    {t("customInstructions.savePreset")}
                  </Button>
                </div>
              </BentoCard>
              <BentoCard>
                <div className="mb-4 flex items-center gap-2 font-medium">
                  <History className="h-4 w-4 text-muted-foreground" />
                  {t("customInstructions.historyTitle")}
                </div>
                <HistoryList
                  items={state?.history ?? []}
                  disabled={busy || protectedMode}
                  rollbackingId={
                    rollbackMutation.isPending
                      ? (rollbackMutation.variables ?? null)
                      : null
                  }
                  onRollback={(id) => rollbackMutation.mutate(id)}
                />
              </BentoCard>
            </>
          )}
        </>
      )}
      <PreviewDialog
        open={preview !== null}
        preview={preview}
        applying={applyMutation.isPending}
        onOpenChange={(open) => {
          if (!open && !applyMutation.isPending) {
            setPreview(null);
            setPending(null);
          }
        }}
        onApply={() => {
          if (pending && !applyMutation.isPending)
            applyMutation.mutate(pending);
        }}
      />
      <AlertDialog
        open={clearOpen}
        onOpenChange={(open) => {
          if (!clearMutation.isPending) setClearOpen(open);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("customInstructions.clearConfirmTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("customInstructions.clearConfirmDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={clearMutation.isPending}>
              {t("common.cancel")}
            </AlertDialogCancel>
            <AlertDialogAction
              disabled={clearMutation.isPending}
              onClick={(event) => {
                event.preventDefault();
                if (!clearMutation.isPending) clearMutation.mutate();
              }}
            >
              {t("customInstructions.clearManagedBlock")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
