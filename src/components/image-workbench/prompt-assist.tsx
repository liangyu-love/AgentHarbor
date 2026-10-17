import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2, RotateCw, Settings2, Undo2, Wand2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/hooks/use-toast";
import { api, type PromptAssistModel, type PromptAssistSettings, type PromptAssistState } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { cn } from "@/lib/utils";
import { SaveAsTemplateButton, charCount, editorKeys } from "./prompt-template-dialog";

const ASSIST_KEY = queryKeys.imagePromptAssist;
const KINDS: { kind: PromptAssistModel["kind"]; label: string }[] = [
  { kind: "relay", label: "中转" },
  { kind: "account", label: "账号" },
  { kind: "aggregate", label: "聚合" },
];
const MAX_REQUEST_CHARS = 500;
const MAX_INSTRUCTION_CHARS = 8000;

/** The 「AI 润色」 switch beside the prompt label. */
export function PromptAssistToggle({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return (
    <Button
      type="button"
      variant="ghost"
      size="xs"
      aria-expanded={open}
      className={cn(
        "h-6 gap-1 px-1.5 font-serif text-[11px] text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]",
        open && "text-[#b83b38] dark:text-[#fca5a5]",
      )}
      title="用中转、账号或聚合里的模型优化提示词"
      onClick={onToggle}
    >
      <Wand2 className="h-3.5 w-3.5" />
      AI 润色
    </Button>
  );
}

/** Rewrites the prompt with a text model AgentHarbor offers; the result is compared before it is used. */
export function PromptAssistPanel({ prompt, onPromptChange, onClose }: {
  prompt: string;
  onPromptChange: (prompt: string) => void;
  onClose: () => void;
}) {
  const queryClient = useQueryClient();
  const state = useQuery({
    queryKey: ASSIST_KEY,
    queryFn: () => api.image.loadPromptAssist(),
    enabled: isTauriRuntime(),
    // Relays and accounts change elsewhere; read them afresh each time the panel opens.
    staleTime: 0,
  });
  const models = useMemo(() => state.data?.models ?? [], [state.data]);
  const settings = state.data?.settings;
  const [picked, setPicked] = useState("");
  const [request, setRequest] = useState("");
  const [result, setResult] = useState<{ text: string; from: string } | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [adopted, setAdopted] = useState<{ before: string; after: string } | null>(null);
  const [instructionOpen, setInstructionOpen] = useState(false);
  const attempt = useRef(0);

  // This session's pick, else the model used last time, else the first one offered.
  const model = [picked, settings?.model].find(id => id && models.some(item => item.id === id)) ?? models[0]?.id ?? "";
  const chosen = models.find(item => item.id === model);

  useEffect(() => () => { attempt.current += 1; }, []);

  const saveSettings = async (next: PromptAssistSettings) => {
    try {
      const saved = await api.image.savePromptAssistSettings(next);
      queryClient.setQueryData<PromptAssistState>(ASSIST_KEY, current => current && { ...current, settings: saved });
      return true;
    } catch (cause) {
      toast({ title: "润色设置保存失败", description: formatInvokeError(cause), variant: "destructive" });
      return false;
    }
  };

  const chooseModel = (id: string) => {
    setPicked(id);
    if (settings) void saveSettings({ ...settings, model: id });
  };

  const polish = async () => {
    if (!model || !prompt.trim() || running) return;
    const id = ++attempt.current;
    setRunning(true);
    setError(null);
    setAdopted(null);
    try {
      const text = await api.image.optimizePrompt(model, prompt, request.trim());
      if (id === attempt.current) setResult({ text, from: prompt });
    } catch (cause) {
      if (id === attempt.current) setError(formatInvokeError(cause));
    } finally {
      if (id === attempt.current) setRunning(false);
    }
  };

  // The request keeps running in the backend; its answer is simply not shown.
  const cancel = () => {
    attempt.current += 1;
    setRunning(false);
  };

  const adopt = () => {
    if (!result?.text.trim()) return;
    setAdopted({ before: prompt, after: result.text });
    onPromptChange(result.text);
    setResult(null);
  };

  const undo = () => {
    if (!adopted) return;
    onPromptChange(adopted.before);
    setAdopted(null);
  };

  const body = () => {
    if (!isTauriRuntime()) {
      return <p className="font-serif text-xs text-muted-foreground">AI 润色需要在桌面端使用。</p>;
    }
    if (state.isError) {
      return (
        <div role="alert" className="flex items-center gap-2 font-serif text-xs text-destructive">
          <span>读取可用模型失败：{formatInvokeError(state.error)}</span>
          <Button type="button" variant="outline" size="xs" className="h-6 gap-1" onClick={() => void state.refetch()}>
            <RotateCw className="h-3 w-3" /> 重试
          </Button>
        </div>
      );
    }
    if (state.isPending) {
      return <p className="font-serif text-xs text-muted-foreground">正在读取可用模型…</p>;
    }
    if (models.length === 0) {
      return (
        <p className="font-serif text-xs leading-relaxed text-muted-foreground">
          没有可用的文本模型。先在「中转站」添加中转，或在「账号」页登录账号，再回来润色。
        </p>
      );
    }
    return (
      <>
        <div className="flex flex-col gap-2 sm:flex-row">
          <Select value={model} onValueChange={chooseModel} disabled={running}>
            <SelectTrigger className="ink-select-trigger h-9 min-w-0 sm:w-56 sm:shrink-0" aria-label="润色模型">
              <SelectValue placeholder="选择模型" />
            </SelectTrigger>
            <SelectContent className="ink-select-content max-h-80" position="popper">
              {KINDS.map(({ kind, label }) => {
                const group = models.filter(item => item.kind === kind);
                if (group.length === 0) return null;
                return (
                  <SelectGroup key={kind}>
                    <SelectLabel className="font-serif text-[11px] text-muted-foreground">{label}</SelectLabel>
                    {group.map(item => (
                      <SelectItem key={item.id} value={item.id} className="ink-select-item">
                        {item.label}
                        {item.sourceName !== item.label && (
                          <span className="text-muted-foreground"> · {item.sourceName}</span>
                        )}
                      </SelectItem>
                    ))}
                  </SelectGroup>
                );
              })}
            </SelectContent>
          </Select>
          <Input
            value={request}
            maxLength={MAX_REQUEST_CHARS}
            onChange={event => setRequest(event.target.value)}
            onKeyDown={event => {
              if (event.key === "Enter") {
                event.preventDefault();
                void polish();
              }
            }}
            placeholder="这次的额外要求（可选），如：改成赛博朋克风"
            aria-label="额外要求"
            disabled={running}
            className="h-9 min-w-0 flex-1 bg-background/90 font-serif text-xs"
          />
          {running ? (
            <Button type="button" variant="outline" size="sm" className="h-9 shrink-0 gap-1.5 font-serif" onClick={cancel}>
              <X className="h-3.5 w-3.5" /> 取消
            </Button>
          ) : (
            <Button
              type="button"
              size="sm"
              className="ink-generate-button h-9 shrink-0 gap-1.5 font-serif text-white"
              disabled={!prompt.trim() || !model}
              title={prompt.trim() ? undefined : "先写一点提示词"}
              onClick={() => void polish()}
            >
              <Wand2 className="h-3.5 w-3.5" /> {result ? "重新润色" : "润色"}
            </Button>
          )}
        </div>

        {running && (
          <p className="flex items-center gap-1.5 font-serif text-xs text-muted-foreground" aria-live="polite">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            正在请求 {chosen?.label ?? model}，推理模型可能要多等一会儿…
          </p>
        )}
        {error && !running && (
          <div role="alert" className="flex flex-wrap items-center gap-2 rounded-lg border border-destructive/30 bg-destructive/5 p-2 font-serif text-xs text-destructive">
            <span className="min-w-0 flex-1 break-words">润色失败：{error}</span>
            <Button type="button" variant="outline" size="xs" className="h-6 gap-1" onClick={() => void polish()}>
              <RotateCw className="h-3 w-3" /> 重试
            </Button>
          </div>
        )}
        {adopted && prompt === adopted.after && (
          <p className="flex items-center gap-2 font-serif text-xs text-muted-foreground" aria-live="polite">
            <Check className="h-3.5 w-3.5 text-emerald-500" /> 已采用润色结果
            <button type="button" onClick={undo} className="inline-flex items-center gap-1 text-[#b83b38] hover:underline dark:text-[#fca5a5]">
              <Undo2 className="h-3 w-3" /> 撤销
            </button>
          </p>
        )}

        {result && !running && (
          <div data-prompt-assist-result className="space-y-2 rounded-lg border border-[#b83b38]/30 bg-background/80 p-2.5">
            <details className="group/source">
              <summary className="cursor-pointer select-none font-serif text-[11px] text-muted-foreground hover:text-foreground">
                原文对照{result.from !== prompt && " · 提示词在润色后又改过"}
              </summary>
              <p className="mt-1 whitespace-pre-wrap break-words rounded-md bg-muted/50 p-2 font-serif text-[11px] leading-relaxed text-muted-foreground">
                {result.from}
              </p>
            </details>
            <div className="flex items-center justify-between">
              <span className="font-serif text-[11px] font-semibold text-foreground/90">润色后（可以先改再采用）</span>
              <span className="font-mono text-[10px] tabular-nums text-muted-foreground">{charCount(result.text)} 字</span>
            </div>
            <Textarea
              value={result.text}
              aria-label="润色结果"
              onChange={event => setResult({ ...result, text: event.target.value })}
              onKeyDown={editorKeys(adopt)}
              className="max-h-80 min-h-[7rem] resize-y bg-background/90 font-serif text-sm leading-relaxed [field-sizing:content] focus:border-[#b83b38]/50 focus:ring-1 focus:ring-[#b83b38]/30"
            />
            <div className="flex flex-wrap items-center gap-1.5">
              <Button type="button" size="xs" className="ink-generate-button h-7 gap-1 font-serif text-white"
                disabled={!result.text.trim()} onClick={adopt} title="Ctrl + Enter">
                <Check className="h-3.5 w-3.5" /> 采用
              </Button>
              <Button type="button" variant="outline" size="xs" className="h-7 gap-1 font-serif" onClick={() => void polish()}>
                <RotateCw className="h-3.5 w-3.5" /> 重新生成
              </Button>
              <SaveAsTemplateButton prompt={result.text} />
              <Button type="button" variant="ghost" size="xs" className="h-7 font-serif text-muted-foreground" onClick={() => setResult(null)}>
                放弃
              </Button>
            </div>
          </div>
        )}
      </>
    );
  };

  return (
    <div data-prompt-assist className="space-y-2.5 rounded-xl bg-black/[0.025] p-2.5 ring-1 ring-[#b83b38]/25 dark:bg-white/[0.02] sm:p-3">
      <div className="flex items-center justify-between gap-2">
        <span className="inline-flex items-center gap-1.5 font-serif text-xs font-semibold text-muted-foreground">
          <Wand2 className="h-3.5 w-3.5 text-[#b83b38] dark:text-[#d94f4b]" />
          AI 润色
        </span>
        <div className="flex items-center gap-0.5">
          {state.data && (
            <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
              onClick={() => setInstructionOpen(true)} title="修改发给模型的润色指令">
              <Settings2 className="h-3.5 w-3.5" />
              {state.data.settings.instruction ? "自定义指令" : "润色指令"}
            </Button>
          )}
          <Button type="button" size="icon" variant="ghost" className="h-6 w-6 text-muted-foreground hover:text-foreground"
            aria-label="收起 AI 润色" onClick={onClose}>
            <X className="h-3.5 w-3.5" />
          </Button>
        </div>
      </div>
      {body()}
      {state.data && (
        <InstructionDialog
          open={instructionOpen}
          settings={state.data.settings}
          defaultInstruction={state.data.defaultInstruction}
          onClose={() => setInstructionOpen(false)}
          onSave={async instruction => {
            if (await saveSettings({ ...state.data!.settings, instruction })) {
              setInstructionOpen(false);
              toast({ title: instruction ? "已保存自定义润色指令" : "已恢复默认润色指令", variant: "success" });
            }
          }}
        />
      )}
    </div>
  );
}

/** Edit the instruction sent as the system prompt; saving the default text stores "use the default". */
function InstructionDialog({ open, settings, defaultInstruction, onClose, onSave }: {
  open: boolean;
  settings: PromptAssistSettings;
  defaultInstruction: string;
  onClose: () => void;
  onSave: (instruction: string) => Promise<void>;
}) {
  const [text, setText] = useState("");
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (open) setText(settings.instruction || defaultInstruction);
    // Seeded once per opening; later setting changes must not reset the text being edited.
  }, [open]);
  const length = charCount(text);
  const invalid = !text.trim() || length > MAX_INSTRUCTION_CHARS;
  const save = async () => {
    if (invalid || saving) return;
    setSaving(true);
    await onSave(text.trim() === defaultInstruction.trim() ? "" : text);
    setSaving(false);
  };
  return (
    <Dialog open={open} onOpenChange={next => !next && !saving && onClose()}>
      <DialogContent className="max-w-2xl gap-4" onKeyDown={editorKeys(() => void save())}>
        <DialogHeader>
          <DialogTitle className="font-serif">润色指令</DialogTitle>
          <DialogDescription className="font-serif text-xs">
            作为系统提示词发给润色模型。写下你习惯的风格、语言或结构；每次润色时填的额外要求会附在提示词后面。
          </DialogDescription>
        </DialogHeader>
        <div className="space-y-1.5">
          <div className="flex justify-end">
            <span className={cn("font-mono text-[11px] tabular-nums text-muted-foreground", length > MAX_INSTRUCTION_CHARS && "text-destructive")}>
              {length}/{MAX_INSTRUCTION_CHARS}
            </span>
          </div>
          <Textarea
            value={text}
            aria-label="润色指令"
            onChange={event => setText(event.target.value)}
            className="h-[clamp(10rem,40vh,26rem)] resize-y bg-background/90 font-serif text-sm leading-relaxed"
          />
        </div>
        <DialogFooter className="items-center gap-2 sm:justify-between">
          <Button type="button" variant="ghost" size="sm" className="font-serif" onClick={() => setText(defaultInstruction)}
            disabled={text === defaultInstruction}>
            恢复默认
          </Button>
          <div className="flex gap-2">
            <Button type="button" variant="outline" size="sm" onClick={onClose} disabled={saving}>取消</Button>
            <Button type="button" size="sm" className="ink-generate-button text-white" onClick={() => void save()} disabled={invalid || saving}>
              {saving ? "保存中…" : "保存指令"}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
