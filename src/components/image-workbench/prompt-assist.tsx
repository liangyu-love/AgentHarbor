import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen, Check, ChevronDown, Loader2, RotateCw, Settings2, Undo2, Wand2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/hooks/use-toast";
import {
  api, type AssistReference, type PromptAssistModel, type PromptAssistSettings, type PromptAssistState, type PromptGroup,
} from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { cn } from "@/lib/utils";
import { SaveAsTemplateButton, charCount, editorKeys } from "./prompt-template-dialog";
import { librarySaved, usePromptLibrary } from "./use-prompt-library";
import type { ReferenceImage } from "./workbench-types";

const ASSIST_KEY = queryKeys.imagePromptAssist;
const KINDS: { kind: PromptAssistModel["kind"]; label: string }[] = [
  { kind: "relay", label: "中转" },
  { kind: "account", label: "账号" },
  { kind: "aggregate", label: "聚合" },
];
const MAX_REQUEST_CHARS = 500;
const MAX_INSTRUCTION_CHARS = 8000;
/** Mirrors MAX_MATERIAL_CHARS in image_prompt_assist.rs. */
const MAX_MATERIAL_CHARS = 24000;
/** Long edge of the reference copy a polishing model sees; generation still uses the original. */
const REFERENCE_EDGE = 1568;
/** Several references go smaller, so the request stays light. */
const REFERENCES_EDGE = 1024;
/**
 * Data URL characters all references may take together. Some relays count an
 * image's base64 as text against their input limit (a 1568px photo came to
 * 141k "tokens"), so a busy picture is sent smaller rather than refused.
 */
const REFERENCE_BUDGET = 200_000;
const MIN_REFERENCE_EDGE = 512;
const REFERENCE_QUALITIES = [0.85, 0.72];

/** A JPEG copy of the reference within `budget` characters, flattened onto white. */
export async function shrinkReference(src: string, edge: number, budget: number): Promise<string> {
  const image = new Image();
  image.src = src;
  await image.decode();
  const longest = Math.max(image.naturalWidth, image.naturalHeight);
  const canvas = document.createElement("canvas");
  const context = canvas.getContext("2d");
  if (!context) throw new Error("无法读取底本");
  for (let size = Math.min(edge, longest); ; size = Math.max(MIN_REFERENCE_EDGE, Math.round(size * 0.8))) {
    const scale = size / longest;
    canvas.width = Math.max(1, Math.round(image.naturalWidth * scale));
    canvas.height = Math.max(1, Math.round(image.naturalHeight * scale));
    context.fillStyle = "#fff";
    context.fillRect(0, 0, canvas.width, canvas.height);
    context.drawImage(image, 0, 0, canvas.width, canvas.height);
    for (const [step, quality] of REFERENCE_QUALITIES.entries()) {
      const data = canvas.toDataURL("image/jpeg", quality);
      const last = size <= MIN_REFERENCE_EDGE && step === REFERENCE_QUALITIES.length - 1;
      if (data.length <= budget || last) return data;
    }
  }
}

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

/** About what the model reads for one material, as image_prompt_assist.rs writes it. */
const materialChars = (item: PromptGroup["templates"][number]) => charCount(item.title + item.note + item.prompt) + 5;

/**
 * What is ticked in each category: all of it when the category id is ticked, else the
 * materials ticked one by one. Ids of what was since deleted fall away.
 */
function tickedMaterials(categories: PromptGroup[], ids: string[]) {
  const set = new Set(ids);
  return categories.map(group => {
    const whole = set.has(group.id);
    return { group, whole, items: group.templates.filter(item => whole || set.has(item.id)) };
  }).filter(entry => entry.items.length > 0);
}

/** Rewrites the prompt with a text model AgentHarbor offers; the result is compared before it is used. */
export function PromptAssistPanel({ prompt, references, onPromptChange, onClose, onManageMaterials }: {
  prompt: string;
  /** The image-to-image references, which the model may see to polish against. */
  references: ReferenceImage[];
  onPromptChange: (prompt: string) => void;
  onClose: () => void;
  /** Open the materials book of the 画谱. */
  onManageMaterials: () => void;
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
  const [result, setResult] = useState<{ text: string; from: string; references: number; materials: string[] } | null>(null);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [adopted, setAdopted] = useState<{ before: string; after: string } | null>(null);
  const [instructionOpen, setInstructionOpen] = useState(false);
  const attempt = useRef(0);
  // Shrunk copies of the references sent, by size and source, so a retry does not redraw them.
  const shrunk = useRef(new Map<string, string>());
  const settingsChain = useRef<Promise<unknown>>(Promise.resolve());

  // This session's pick, else the model used last time, else the first one offered.
  const model = [picked, settings?.model].find(id => id && models.some(item => item.id === id)) ?? models[0]?.id ?? "";
  const chosen = models.find(item => item.id === model);
  // Loaded references go along unless the user once unticked them.
  const withReference = references.length > 0 && !settings?.skipReference;
  // Material categories with something in them; what is ticked goes along as text the model reads.
  const materialBook = usePromptLibrary("materials");
  const categories = materialBook.groups.filter(group => group.templates.length > 0);
  const ticked = tickedMaterials(categories, settings?.materials ?? []);
  // In library order, a whole category by its id, else each ticked material.
  const tickedIds = ticked.flatMap(entry => (entry.whole ? [entry.group.id] : entry.items.map(item => item.id)));
  const tickedNames = ticked.map(entry => (entry.whole ? entry.group.name : `${entry.group.name}（${entry.items.length} 条）`));

  useEffect(() => () => { attempt.current += 1; }, []);

  /**
   * Show the settings at once and save them in order, so a tick made before the
   * last save returned builds on it instead of writing over it.
   */
  const saveSettings = (next: PromptAssistSettings) => {
    queryClient.setQueryData<PromptAssistState>(ASSIST_KEY, current => current && { ...current, settings: next });
    const save = settingsChain.current.then(() => api.image.savePromptAssistSettings(next));
    settingsChain.current = save.catch(() => undefined);
    return save.then(
      saved => {
        // A newer change is already showing and its own save will follow.
        if (queryClient.getQueryData<PromptAssistState>(ASSIST_KEY)?.settings === next) {
          queryClient.setQueryData<PromptAssistState>(ASSIST_KEY, current => current && { ...current, settings: saved });
        }
        return true;
      },
      cause => {
        toast({ title: "润色设置保存失败", description: formatInvokeError(cause), variant: "destructive" });
        void queryClient.invalidateQueries({ queryKey: ASSIST_KEY });
        return false;
      },
    );
  };

  const saveTicked = (materials: string[]) => {
    if (settings) void saveSettings({ ...settings, materials });
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
      const sent: AssistReference[] = [];
      if (withReference) {
        const edge = references.length > 1 ? REFERENCES_EDGE : REFERENCE_EDGE;
        const budget = Math.floor(REFERENCE_BUDGET / references.length);
        // Only the copies of what is sent now are kept.
        const kept = new Map<string, string>();
        for (const item of references) {
          const key = `${edge}:${budget}:${item.src}`;
          const data = shrunk.current.get(key) ?? await shrinkReference(item.src, edge, budget);
          kept.set(key, data);
          sent.push({ data, name: item.name ?? "", note: item.note ?? "" });
        }
        shrunk.current = kept;
      }
      // The backend reads the materials from disk: an edit just made must be there first.
      if (tickedIds.length > 0) await librarySaved("materials");
      const text = await api.image.optimizePrompt(model, prompt, request.trim(), tickedIds, sent);
      if (id === attempt.current) {
        setResult({ text, from: prompt, references: sent.length, materials: tickedNames });
      }
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
        <div className="flex items-center gap-2 font-serif text-xs leading-relaxed text-muted-foreground">
          <span>没有可用的文本模型。先在「中转站」添加中转，或在「账号」页登录账号，再回来润色。</span>
          <Button type="button" variant="outline" size="xs" className="h-6 shrink-0 gap-1" disabled={state.isFetching}
            onClick={() => void state.refetch()}>
            <RotateCw className={cn("h-3 w-3", state.isFetching && "animate-spin motion-reduce:animate-none")} /> 刷新
          </Button>
        </div>
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
          <Button
            type="button"
            variant="outline"
            size="icon"
            className="h-9 w-9 shrink-0"
            disabled={running || state.isFetching}
            title="重新读取中转、账号和聚合里的模型"
            aria-label="刷新润色模型"
            onClick={() => void state.refetch()}
          >
            <RotateCw className={cn("h-3.5 w-3.5", state.isFetching && "animate-spin motion-reduce:animate-none")} />
          </Button>
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

        {references.length > 0 && settings && (
          <label className="flex cursor-pointer items-center gap-2 rounded-lg border border-border/60 bg-background/60 p-1.5 pr-2.5">
            <span className="flex shrink-0 -space-x-3">
              {references.slice(0, 4).map(item => (
                <img key={item.key} src={item.src} alt="" className="h-9 w-9 rounded bg-card object-cover ring-1 ring-border/60" />
              ))}
            </span>
            <Checkbox
              checked={withReference}
              disabled={running}
              aria-label="附带参考图"
              onCheckedChange={on => void saveSettings({ ...settings, skipReference: on !== true })}
              className="border-[#b83b38]/60 data-[state=checked]:border-[#b83b38] data-[state=checked]:bg-[#b83b38] data-[state=checked]:text-white"
            />
            <span className="min-w-0 font-serif text-xs leading-snug">
              <span className="text-foreground/90">附带参考图{references.length > 1 && ` · ${references.length} 张`}</span>
              <span className="block text-[11px] text-muted-foreground">
                {references.length > 1
                  ? "让模型看着图1、图2…润色，写成「从哪张图取什么、怎样组合」的改图指令；模型需支持识图"
                  : "让模型看着底本润色，写成「保留什么、改成什么」的改图指令；模型需支持识图"}
              </span>
            </span>
          </label>
        )}

        {settings && materialBook.ready && (
          <MaterialPicker categories={categories} ticked={ticked} disabled={running}
            onChange={saveTicked} onManage={onManageMaterials} />
        )}

        {running && (
          <p className="flex items-center gap-1.5 font-serif text-xs text-muted-foreground" aria-live="polite">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            正在请求 {chosen?.label ?? model}{withReference && `（附带 ${references.length} 张参考图）`}{tickedNames.length > 0 && `，参考${tickedNames.join("、")}素材`}，推理模型可能要多等一会儿…
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
              <span className="font-serif text-[11px] font-semibold text-foreground/90">
                润色后（可以先改再采用）
                {result.references > 0 && (
                  <span className="font-normal text-muted-foreground"> · {result.references > 1 ? `已对照 ${result.references} 张参考图` : "已对照底本"}</span>
                )}
                {result.materials.length > 0 && (
                  <span className="font-normal text-muted-foreground"> · 参考了{result.materials.join("、")}素材</span>
                )}
              </span>
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

/**
 * The materials that go along to polishing: a whole category with its box, or, opened,
 * only the materials ticked in it, so a long category costs no more than what is picked.
 */
function MaterialPicker({ categories, ticked, disabled, onChange, onManage }: {
  categories: PromptGroup[];
  ticked: ReturnType<typeof tickedMaterials>;
  disabled: boolean;
  onChange: (materials: string[]) => void;
  onManage: () => void;
}) {
  const [openId, setOpenId] = useState<string | null>(null);
  const open = categories.find(group => group.id === openId);
  const entryOf = (group: PromptGroup) => ticked.find(entry => entry.group.id === group.id);
  const tickedChars = ticked.reduce((sum, entry) =>
    sum + entry.items.reduce((size, item) => size + materialChars(item), charCount(entry.group.name) + 4), 0);
  // An estimate: the backend counts exactly and refuses past its own limit.
  const tooMany = tickedChars > MAX_MATERIAL_CHARS;

  /** Every other category's ticks, then this one's as given: all of it, or these materials. */
  const replace = (group: PromptGroup, itemIds: string[]) => {
    const others = ticked.filter(entry => entry.group.id !== group.id)
      .flatMap(entry => (entry.whole ? [entry.group.id] : entry.items.map(item => item.id)));
    const own = itemIds.length === group.templates.length ? [group.id] : itemIds;
    onChange([...others, ...own]);
  };
  const toggleItem = (group: PromptGroup, id: string, on: boolean) => {
    const current = entryOf(group)?.items.map(item => item.id) ?? [];
    const next = on ? [...current, id] : current.filter(item => item !== id);
    replace(group, group.templates.map(item => item.id).filter(item => next.includes(item)));
  };

  const box = "border-[#b83b38]/60 data-[state=checked]:border-[#b83b38] data-[state=checked]:bg-[#b83b38] data-[state=checked]:text-white data-[state=indeterminate]:border-[#b83b38] data-[state=indeterminate]:bg-[#b83b38]/15 data-[state=indeterminate]:text-[#b83b38]";
  return (
    <div data-prompt-assist-materials className="space-y-1.5 font-serif text-xs">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="text-muted-foreground" title="勾选的分类整类发给模型；点分类旁的箭头可以只勾其中几条。模型按画面挑选合适的素材写进提示词，勾得越多，每次润色用的 token 越多。素材在下方画谱的「素材」页管理">
          参考素材{ticked.length > 0 && (
            <span className={cn("font-sans text-[10px] tabular-nums", tooMany && "font-semibold text-destructive")}
              title={tooMany ? `超过单次润色约 ${MAX_MATERIAL_CHARS} 字的上限，请少勾一些` : undefined}>
              {" "}· 约 {tickedChars} 字{tooMany && "，超出上限"}
            </span>
          )}
        </span>
        {categories.map(group => {
          const entry = entryOf(group);
          const state = entry?.whole ? true : entry ? "indeterminate" : false;
          const expanded = openId === group.id;
          return (
            <span key={group.id} className={cn(
              "inline-flex items-stretch rounded-md border transition-colors",
              state ? "border-[#b83b38]/35 bg-[#b83b38]/[0.05] dark:border-[#d94f4b]/35" : "border-border/60 bg-background/60 text-muted-foreground",
              expanded && "ring-1 ring-[#b83b38]/30",
            )}>
              <label className="inline-flex cursor-pointer items-center gap-1.5 py-1 pl-2 pr-1" title={`${group.templates.length} 条素材`}>
                <Checkbox
                  checked={state}
                  disabled={disabled}
                  aria-label={`参考素材：${group.name}`}
                  onCheckedChange={next => replace(group, next === true ? group.templates.map(item => item.id) : [])}
                  className={cn("h-3.5 w-3.5", box)}
                />
                {group.name}
                <span className="font-sans text-[10px] tabular-nums opacity-60">
                  {entry && !entry.whole ? `${entry.items.length}/${group.templates.length}` : group.templates.length}
                </span>
              </label>
              <button type="button" aria-expanded={expanded} aria-label={`挑选${group.name}里的素材`}
                title="只勾其中几条" onClick={() => setOpenId(expanded ? null : group.id)}
                className="rounded-r-md px-1 text-muted-foreground hover:bg-muted/60 hover:text-foreground">
                <ChevronDown className={cn("h-3 w-3 transition-transform", expanded && "rotate-180")} />
              </button>
            </span>
          );
        })}
        {categories.length === 0 && <span className="text-[11px] text-muted-foreground">素材库是空的</span>}
        {ticked.length > 0 && (
          <button type="button" disabled={disabled} onClick={() => onChange([])}
            className="rounded px-1 text-[11px] text-muted-foreground hover:text-foreground disabled:opacity-50">
            清空
          </button>
        )}
        <button type="button" onClick={onManage} title="在下方画谱的素材页增删、修改素材和分类"
          className="inline-flex items-center gap-1 rounded px-1 text-[11px] text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]">
          <BookOpen className="h-3 w-3" /> 管理素材
        </button>
      </div>
      {open && (
        <div data-prompt-assist-category={open.name} className="rounded-lg border border-[#b83b38]/25 bg-background/70 p-2">
          <div className="mb-1.5 flex items-center justify-between gap-2 text-[11px] text-muted-foreground">
            <span className="min-w-0 truncate">{open.name} · 只有勾上的几条会发给模型</span>
            <span className="flex shrink-0 items-center gap-1">
              <button type="button" disabled={disabled} className="rounded px-1 hover:text-foreground disabled:opacity-50"
                onClick={() => replace(open, open.templates.map(item => item.id))}>全选</button>
              <button type="button" disabled={disabled} className="rounded px-1 hover:text-foreground disabled:opacity-50"
                onClick={() => replace(open, [])}>全不选</button>
            </span>
          </div>
          <div className="flex max-h-48 flex-wrap gap-1 overflow-y-auto overscroll-contain">
            {open.templates.map(item => {
              const on = Boolean(entryOf(open)?.items.some(have => have.id === item.id));
              return (
                <label key={item.id} title={item.note ? `${item.note}\n${item.prompt}` : item.prompt} className={cn(
                  "inline-flex max-w-full cursor-pointer items-center gap-1.5 rounded-md border px-1.5 py-0.5",
                  on ? "border-[#b83b38]/35 bg-[#b83b38]/[0.05] dark:border-[#d94f4b]/35" : "border-border/50 text-muted-foreground",
                )}>
                  <Checkbox checked={on} disabled={disabled} aria-label={`参考素材：${open.name} · ${item.title}`}
                    onCheckedChange={next => toggleItem(open, item.id, next === true)} className={cn("h-3 w-3", box)} />
                  <span className="truncate">{item.title}</span>
                </label>
              );
            })}
          </div>
        </div>
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
