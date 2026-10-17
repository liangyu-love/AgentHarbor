import { useEffect, useMemo, useState, type KeyboardEvent } from "react";
import { BookmarkPlus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/hooks/use-toast";
import { storageKeys } from "@/lib/storage-keys";
import { cn } from "@/lib/utils";
import { PROMPT_LIMITS, usePromptLibrary, type PromptLibraryState, type TemplateDraft } from "./use-prompt-library";

const NEW_GROUP = "__new__";
const DEFAULT_GROUP_NAME = "常用";

/**
 * What the editor opens on: a template to change, or a new one. `draft` is what is
 * saved now; `start` is unsaved text carried over from the inline editor.
 */
export type TemplateEditorRequest = {
  templateId?: string;
  groupId?: string;
  draft: TemplateDraft;
  start?: TemplateDraft;
};

export const charCount = (text: string) => Array.from(text).length;

/** The group the user last worked in, so new templates land where they expect. */
export function rememberedGroup(): string | null {
  try {
    return localStorage.getItem(storageKeys.imagePromptGroup);
  } catch {
    return null;
  }
}

export function rememberGroup(id: string) {
  try {
    localStorage.setItem(storageKeys.imagePromptGroup, id);
  } catch {
    // Storage unavailable; the panel falls back to its first group.
  }
}

/** Shared body for the dialog and inline editors: Ctrl/⌘+Enter saves, Esc cancels. */
export function editorKeys(onSave: () => void, onCancel?: () => void) {
  return (event: KeyboardEvent) => {
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      onSave();
    } else if (event.key === "Escape" && onCancel) {
      event.preventDefault();
      event.stopPropagation();
      onCancel();
    }
  };
}

/** The wide editor for a prompt template: group, title, note and the full prompt. */
export function PromptTemplateDialog({ library, request, onClose }: {
  library: PromptLibraryState;
  request: TemplateEditorRequest | null;
  onClose: () => void;
}) {
  const { groups, templateCount } = library;
  const [groupId, setGroupId] = useState("");
  const [newGroupName, setNewGroupName] = useState(DEFAULT_GROUP_NAME);
  const [draft, setDraft] = useState<TemplateDraft>({ title: "", note: "", prompt: "" });
  const [saving, setSaving] = useState(false);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);

  // Each request starts a fresh form.
  useEffect(() => {
    if (!request) return;
    const preferred = [request.groupId, rememberedGroup()].find(id => groups.some(group => group.id === id));
    setGroupId(preferred ?? groups[0]?.id ?? NEW_GROUP);
    setNewGroupName(DEFAULT_GROUP_NAME);
    setDraft(request.start ?? request.draft);
    setSaving(false);
    // Groups only seed the selection; later changes must not reset the form.
  }, [request]);

  const creatingGroup = groupId === NEW_GROUP;
  const trimmedGroupName = newGroupName.trim();
  const groupNameTaken = creatingGroup && groups.some(group => group.name === trimmedGroupName);
  const promptLength = charCount(draft.prompt);
  const dirty = request !== null && (
    draft.title !== request.draft.title || draft.note !== request.draft.note || draft.prompt !== request.draft.prompt
  );
  const problem = useMemo(() => {
    if (!draft.prompt.trim()) return "提示词不能为空";
    if (promptLength > PROMPT_LIMITS.prompt) return `提示词最多 ${PROMPT_LIMITS.prompt} 字`;
    if (creatingGroup && !trimmedGroupName) return "请填写分组名称";
    if (groupNameTaken) return "已有同名分组，请直接选择它";
    if (creatingGroup && groups.length >= PROMPT_LIMITS.groups) return `最多 ${PROMPT_LIMITS.groups} 个分组`;
    if (!request?.templateId && templateCount >= PROMPT_LIMITS.templates) return `最多保存 ${PROMPT_LIMITS.templates} 个模版`;
    return null;
  }, [draft.prompt, promptLength, creatingGroup, trimmedGroupName, groupNameTaken, groups.length, request?.templateId, templateCount]);
  // A fresh empty form is not an error yet.
  const shownProblem = dirty || draft.prompt.trim() ? problem : null;

  const requestClose = () => {
    if (saving) return;
    if (!dirty) return onClose();
    setConfirm({
      title: "放弃未保存的修改？",
      description: "关闭后，这次对模版的修改不会保存。",
      confirmLabel: "放弃修改",
      destructive: true,
      onConfirm: () => {
        setConfirm(null);
        onClose();
      },
    });
  };

  const save = async () => {
    if (!request || problem || saving) return;
    setSaving(true);
    const targetGroup = creatingGroup ? await library.addGroup(trimmedGroupName) : groupId;
    const saved = targetGroup ? await library.saveTemplate(targetGroup, draft, request.templateId) : null;
    setSaving(false);
    if (!saved || !targetGroup) return;
    rememberGroup(targetGroup);
    const groupName = creatingGroup ? trimmedGroupName : groups.find(group => group.id === targetGroup)?.name;
    toast({ title: request.templateId ? "模版已更新" : `已存入「${groupName ?? "画谱"}」`, variant: "success" });
    onClose();
  };

  const field = "bg-background/90 font-serif text-sm";
  return (
    <>
      <Dialog open={request !== null} onOpenChange={open => !open && requestClose()}>
        <DialogContent
          className="max-w-2xl gap-5"
          onKeyDown={editorKeys(() => void save())}
          // Closing is decided by requestClose so unsaved work is never lost silently.
          onEscapeKeyDown={event => { event.preventDefault(); requestClose(); }}
        >
          <DialogHeader>
            <DialogTitle className="font-serif">{request?.templateId ? "编辑模版" : "新建模版"}</DialogTitle>
            <DialogDescription className="font-serif text-xs">
              模版保存在本机的应用数据目录，可在画谱中导出备份。
            </DialogDescription>
          </DialogHeader>

          <div className="grid gap-4 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor="prompt-template-title" className="font-serif text-xs font-semibold">标题</Label>
              <Input
                id="prompt-template-title"
                value={draft.title}
                maxLength={PROMPT_LIMITS.title}
                onChange={event => setDraft({ ...draft, title: event.target.value })}
                placeholder="留空则取提示词开头"
                className={field}
                autoFocus
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="prompt-template-group" className="font-serif text-xs font-semibold">分组</Label>
              <Select value={groupId} onValueChange={setGroupId}>
                <SelectTrigger id="prompt-template-group" className="ink-select-trigger" aria-label="分组">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="ink-select-content" position="popper">
                  {groups.map(group => (
                    <SelectItem key={group.id} value={group.id} className="ink-select-item">{group.name}</SelectItem>
                  ))}
                  <SelectItem value={NEW_GROUP} className="ink-select-item" disabled={groups.length >= PROMPT_LIMITS.groups}>
                    ＋ 新建分组…
                  </SelectItem>
                </SelectContent>
              </Select>
              {creatingGroup && (
                <Input
                  id="prompt-template-new-group"
                  aria-label="新分组名称"
                  value={newGroupName}
                  maxLength={PROMPT_LIMITS.groupName}
                  onChange={event => setNewGroupName(event.target.value)}
                  placeholder="新分组名称"
                  className={cn(field, groupNameTaken && "border-destructive/60")}
                />
              )}
            </div>
            <div className="space-y-1.5 sm:col-span-2">
              <Label htmlFor="prompt-template-note" className="font-serif text-xs font-semibold">
                备注 <span className="font-normal text-muted-foreground">(可选)</span>
              </Label>
              <Input
                id="prompt-template-note"
                value={draft.note}
                maxLength={PROMPT_LIMITS.note}
                onChange={event => setDraft({ ...draft, note: event.target.value })}
                placeholder="用途、适合的模型、要避开的坑…"
                className={field}
              />
            </div>
            <div className="space-y-1.5 sm:col-span-2">
              <div className="flex items-center justify-between">
                <Label htmlFor="prompt-template-body" className="font-serif text-xs font-semibold">提示词</Label>
                <span className={cn(
                  "font-mono text-[11px] tabular-nums text-muted-foreground",
                  promptLength > PROMPT_LIMITS.prompt && "text-destructive",
                )}>
                  {promptLength}/{PROMPT_LIMITS.prompt}
                </span>
              </div>
              <Textarea
                id="prompt-template-body"
                value={draft.prompt}
                onChange={event => setDraft({ ...draft, prompt: event.target.value })}
                placeholder="描述画面主体、构图、光线和风格…"
                className="h-[clamp(8rem,32vh,24rem)] min-h-[8rem] resize-y bg-background/90 font-serif text-sm leading-relaxed focus:border-[#b83b38]/50 focus:ring-1 focus:ring-[#b83b38]/30"
              />
            </div>
          </div>

          <DialogFooter className="items-center gap-2 sm:justify-between">
            <p role={shownProblem ? "alert" : undefined} className={cn(
              "font-serif text-[11px]",
              shownProblem ? "text-destructive" : "text-muted-foreground",
            )}>
              {shownProblem ?? "Ctrl + Enter 保存 · Esc 取消"}
            </p>
            <div className="flex gap-2">
              <Button type="button" variant="outline" size="sm" onClick={requestClose} disabled={saving}>取消</Button>
              <Button
                type="button"
                size="sm"
                className="ink-generate-button gap-1.5 text-white"
                onClick={() => void save()}
                disabled={Boolean(problem) || saving}
              >
                <BookmarkPlus className="h-3.5 w-3.5" />
                {saving ? "保存中…" : "保存模版"}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </>
  );
}

/** Saves whatever is in the prompt box as a new template. */
export function SaveAsTemplateButton({ prompt, className }: { prompt: string; className?: string }) {
  const library = usePromptLibrary();
  const [request, setRequest] = useState<TemplateEditorRequest | null>(null);
  if (!library.ready) return null;
  return (
    <>
      <Button
        type="button"
        variant="ghost"
        size="xs"
        className={cn(
          "h-6 gap-1 px-1.5 font-serif text-[11px] text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]",
          className,
        )}
        disabled={!prompt.trim()}
        title="把当前提示词存进画谱"
        onClick={() => setRequest({ draft: { title: "", note: "", prompt } })}
      >
        <BookmarkPlus className="h-3.5 w-3.5" />
        存为模版
      </Button>
      <PromptTemplateDialog library={library} request={request} onClose={() => setRequest(null)} />
    </>
  );
}
