import { useEffect, useMemo, useRef, useState } from "react";
import {
  ArrowDownToLine, ArrowUpFromLine, Check, ChevronDown, ChevronUp, Copy, Feather, GripVertical,
  ListPlus, Maximize2, PackagePlus, Pencil, Pin, PinOff, Plus, RotateCw, Search, Sparkles, Star, Trash2, X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import type { PromptGroup, PromptLibraryKind, PromptTemplate } from "@/lib/api";
import { cn } from "@/lib/utils";
import {
  PromptTemplateDialog, charCount, editorKeys, rememberGroup, rememberedGroup, type TemplateEditorRequest,
} from "./prompt-template-dialog";
import { usePointerSort } from "./use-pointer-sort";
import {
  PROMPT_LIMITS, rememberBook, usePromptLibrary, type PromptLibraryState, type TemplateDraft,
} from "./use-prompt-library";

const iconButton = "h-7 w-7 text-muted-foreground";
// Secondary card actions appear while the card is hovered or focused.
const revealed = "opacity-0 transition-opacity group-hover/card:opacity-100 group-focus-within/card:opacity-100";

const draftOf = (template: PromptTemplate): TemplateDraft => ({
  title: template.title,
  note: template.note,
  prompt: template.prompt,
});

const BOOK_TABS: { kind: PromptLibraryKind; label: string; title: string }[] = [
  { kind: "prompts", label: "提示词灵感", title: "整段的提示词模版，点一下填入提示词框" },
  { kind: "materials", label: "素材", title: "服装、动作、视角等描述片段；AI 润色时勾选分类，模型会从中挑选" },
];

/** Punctuation a material can follow without a comma of its own. */
const CLAUSE_ENDS = ["，", ",", "。", ".", "；", ";", "、"];

/** A material joins the end of the prompt instead of replacing it. */
const withMaterial = (prompt: string, material: string) => {
  const base = prompt.trimEnd();
  if (!base) return material;
  return CLAUSE_ENDS.some(mark => base.endsWith(mark)) ? base + material : `${base}，${material}`;
};

/** The 画谱: the user's own prompt templates, and the materials AI polishing draws on. */
export function PromptLibrary({ kind, onKindChange, prompt, onPromptChange }: {
  /** The book shown; the polishing panel can open the materials from outside. */
  kind: PromptLibraryKind;
  onKindChange: (kind: PromptLibraryKind) => void;
  prompt: string;
  onPromptChange: (prompt: string) => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  // Opening a book, from its tab or from elsewhere, shows it.
  useEffect(() => {
    setCollapsed(false);
    rememberBook(kind);
  }, [kind]);
  // Each book starts with its own search, selection and editors.
  return (
    <PromptBook key={kind} kind={kind} onKindChange={onKindChange} collapsed={collapsed} onCollapsedChange={setCollapsed}
      prompt={prompt} onPromptChange={onPromptChange} />
  );
}

/** One book of the 画谱: groups and templates, or categories and materials. */
function PromptBook({ kind, onKindChange, collapsed, onCollapsedChange: setCollapsed, prompt, onPromptChange }: {
  kind: PromptLibraryKind;
  onKindChange: (kind: PromptLibraryKind) => void;
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  prompt: string;
  onPromptChange: (prompt: string) => void;
}) {
  const library = usePromptLibrary(kind);
  const { groups, book } = library;
  const materials = kind === "materials";
  const [activeId, setActiveId] = useState<string | null>(() => (materials ? null : rememberedGroup()));
  const [query, setQuery] = useState("");
  const [editor, setEditor] = useState<TemplateEditorRequest | null>(null);
  const [inlineId, setInlineId] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [naming, setNaming] = useState<{ groupId: string | null } | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const active = groups.find(group => group.id === activeId) ?? groups[0];
  const searching = query.trim().length > 0;

  const results = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return groups.flatMap(group => group.templates
      .filter(template => [group.name, template.title, template.note, template.prompt]
        .some(text => text.toLowerCase().includes(q)))
      .map(template => ({ group, template })));
  }, [groups, query]);

  const groupSort = usePointerSort({
    onMove: (from, to) => void library.moveGroup(from, to),
    disabled: naming !== null,
  });
  const templateSort = usePointerSort({
    onMove: (from, to) => active && void library.moveTemplate(active.id, from, to),
    disabled: searching || inlineId !== null,
    scrollRef: listRef,
  });

  const selectGroup = (id: string) => {
    setActiveId(id);
    if (!materials) rememberGroup(id);
    setInlineId(null);
  };

  const nameTaken = (name: string, exceptId: string | null) =>
    groups.some(group => group.name === name && group.id !== exceptId);

  const submitGroupName = async (name: string) => {
    const target = naming;
    setNaming(null);
    if (!target) return;
    if (target.groupId) {
      await library.renameGroup(target.groupId, name);
      return;
    }
    const id = await library.addGroup(name);
    if (id) selectGroup(id);
  };

  const confirmDeleteGroup = (group: PromptGroup) => setConfirm({
    title: `删除${book.group}「${group.name}」？`,
    description: group.templates.length > 0
      ? `其中 ${group.templates.length} ${book.unit}${book.item}会一并删除，且无法恢复。可以先导出${book.name}备份。`
      : `这个${book.group}还没有${book.item}。`,
    confirmLabel: `删除${book.group}`,
    destructive: true,
    onConfirm: () => {
      setConfirm(null);
      void library.deleteGroup(group.id);
    },
  });

  const confirmDeleteTemplate = (template: PromptTemplate) => setConfirm({
    title: `删除${book.item}「${template.title}」？`,
    description: "删除后无法恢复。",
    confirmLabel: `删除${book.item}`,
    destructive: true,
    onConfirm: () => {
      setConfirm(null);
      if (inlineId === template.id) setInlineId(null);
      void library.deleteTemplate(template.id);
    },
  });

  const fill = (template: PromptTemplate) => {
    onPromptChange(materials ? withMaterial(prompt, template.prompt) : template.prompt);
    setExpandedId(template.id);
  };

  const copy = async (template: PromptTemplate) => {
    try {
      await navigator.clipboard.writeText(template.prompt);
      setCopiedId(template.id);
      setTimeout(() => setCopiedId(current => (current === template.id ? null : current)), 1600);
    } catch {
      fill(template);
    }
  };

  const openDialog = (group: PromptGroup, template: PromptTemplate, start?: TemplateDraft) => {
    setInlineId(null);
    setEditor({ templateId: template.id, groupId: group.id, draft: draftOf(template), start });
  };

  const card = (group: PromptGroup, template: PromptTemplate, index: number | null) => {
    if (inlineId === template.id) {
      return (
        <InlineTemplateEditor
          key={template.id}
          template={template}
          book={book}
          onCancel={() => setInlineId(null)}
          onSave={async draft => {
            if (await library.saveTemplate(group.id, draft, template.id)) setInlineId(null);
          }}
          onExpand={draft => openDialog(group, template, draft)}
        />
      );
    }
    const expanded = expandedId === template.id;
    const filled = materials ? prompt.includes(template.prompt) : prompt === template.prompt;
    const sortable = index !== null && inlineId === null;
    const dragging = index !== null && templateSort.drag?.from === index;
    const dropTarget = index !== null && templateSort.drag && templateSort.drag.over === index && !dragging;
    return (
      <div
        key={template.id}
        data-prompt-template={template.title}
        ref={index === null ? undefined : templateSort.itemRef(index)}
        className={cn(
          "group/card shrink-0 rounded-lg border border-border/60 bg-background/70 p-2 transition-[border-color,background-color,opacity,box-shadow] sm:p-2.5",
          filled
            ? "border-[#b83b38]/45 bg-[#b83b38]/[0.04] dark:border-[#d94f4b]/40 dark:bg-[#d94f4b]/[0.06]"
            : "hover:border-[#b83b38]/35",
          dragging && "opacity-50",
          dropTarget && "ring-2 ring-[#b83b38]/40",
        )}
      >
        <div className="flex items-start gap-1">
          {index !== null && (
            <button
              type="button"
              aria-label={`拖动排序「${template.title}」，或按 Alt + 方向键`}
              title="拖动排序 · Alt + ↑↓"
              className={cn(
                "-ml-1 mt-0.5 shrink-0 cursor-grab touch-none select-none rounded p-0.5 text-muted-foreground/45 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 active:cursor-grabbing",
                !sortable && "pointer-events-none opacity-0",
              )}
              {...templateSort.handleProps(index, group.templates.length)}
            >
              <GripVertical className="h-3.5 w-3.5" />
            </button>
          )}
          <div className="min-w-0 flex-1">
            <div className="flex items-start gap-1">
              <div className="min-w-0 flex-1">
                <p className="flex min-w-0 items-center gap-1 font-serif text-xs font-semibold text-foreground/90">
                  {template.pinned && <Star className="h-3 w-3 shrink-0 fill-amber-400 text-amber-500" aria-label="已置顶" />}
                  <span className="truncate">{template.title}</span>
                  {index === null && (
                    <button
                      type="button"
                      onClick={() => { setQuery(""); selectGroup(group.id); }}
                      className="ml-1 shrink-0 rounded bg-muted px-1 py-px text-[9px] font-normal text-muted-foreground hover:text-foreground"
                      title={`打开这个${book.group}`}
                    >
                      {group.name}
                    </button>
                  )}
                </p>
                {template.note && (
                  <p className="mt-0.5 font-serif text-[11px] leading-snug text-muted-foreground/85">{template.note}</p>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-0.5">
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "hover:text-amber-500")}
                  title={template.pinned ? "取消置顶" : "置顶"} aria-label={template.pinned ? "取消置顶" : "置顶"}
                  onClick={() => void library.togglePin(template.id)}>
                  {template.pinned ? <PinOff className="h-3.5 w-3.5" /> : <Pin className="h-3.5 w-3.5" />}
                </Button>
                {materials ? (
                  <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "hover:text-[#b83b38] dark:hover:text-[#fca5a5]")}
                    title={filled ? "已在提示词中" : "追加到提示词末尾"} aria-label={filled ? "已在提示词中" : "追加到提示词"}
                    disabled={filled} onClick={() => fill(template)}>
                    {filled ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <ListPlus className="h-3.5 w-3.5" />}
                  </Button>
                ) : (
                  <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "hover:text-[#b83b38] dark:hover:text-[#fca5a5]")}
                    title={filled ? "已填入" : "填入提示词"} aria-label={filled ? "已填入" : "填入提示词"}
                    onClick={() => fill(template)}>
                    {filled ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Feather className="h-3.5 w-3.5" />}
                  </Button>
                )}
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "hover:text-foreground")}
                  title={`复制${book.content}`} aria-label={`复制${book.content}`} onClick={() => void copy(template)}>
                  {copiedId === template.id ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                </Button>
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, revealed, "hover:text-foreground")}
                  title="在卡片上编辑" aria-label="在卡片上编辑" onClick={() => setInlineId(template.id)}>
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, revealed, "hover:text-foreground")}
                  title="在弹窗中编辑" aria-label="在弹窗中编辑" onClick={() => openDialog(group, template)}>
                  <Maximize2 className="h-3.5 w-3.5" />
                </Button>
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, revealed, "hover:text-destructive")}
                  title={`删除${book.item}`} aria-label={`删除${book.item}`} onClick={() => confirmDeleteTemplate(template)}>
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            </div>
            <button type="button" className="mt-1 w-full text-left" onClick={() => setExpandedId(expanded ? null : template.id)}>
              <p className={cn(
                "whitespace-pre-wrap break-words font-serif text-[11px] leading-relaxed text-muted-foreground",
                !expanded && "line-clamp-2",
              )}>
                {template.prompt}
              </p>
              <span className="mt-0.5 inline-block font-serif text-[10px] text-muted-foreground/60 hover:text-foreground">
                {expanded ? "收起" : "展开全文"}
              </span>
            </button>
          </div>
        </div>
      </div>
    );
  };

  const body = () => {
    if (library.loadError) {
      return (
        <div role="alert" className="flex flex-col items-center gap-2 py-6 text-center">
          <p className="font-serif text-xs text-destructive">{book.name}读取失败：{library.loadError}</p>
          <Button type="button" variant="outline" size="xs" className="gap-1 font-serif" onClick={library.reload}>
            <RotateCw className="h-3 w-3" /> 重新读取
          </Button>
        </div>
      );
    }
    if (!library.ready) {
      return (
        <p className="py-6 text-center font-serif text-xs text-muted-foreground">
          {library.loading ? `正在展开${book.name}…` : `${book.name}保存在本机，仅在桌面端可用。`}
        </p>
      );
    }
    if (searching) {
      return (
        <div ref={listRef} className="flex max-h-96 min-h-[9rem] flex-col gap-2 overflow-y-auto overscroll-contain pr-1">
          {results.length === 0 ? (
            <p className="py-6 text-center font-serif text-xs text-muted-foreground">{book.name}中没有匹配「{query.trim()}」的{book.item}</p>
          ) : results.map(({ group, template }) => card(group, template, null))}
        </div>
      );
    }
    return (
      <>
        {groupTabs()}
        {!active ? (
          <div className="flex flex-col items-center gap-2 py-6 text-center">
            <p className="max-w-xs font-serif text-xs leading-relaxed text-muted-foreground">
              {materials
                ? "素材库是空的。先建一个分类（如服装、动作、视角），再把常用的描述片段收进来；也可以导入之前导出的素材。"
                : "画谱还是空白的。先建一个分组，再把常用的生图提示词收进来；也可以导入之前导出的画谱。"}
            </p>
            <div className="flex flex-wrap items-center justify-center gap-1">
              <Button type="button" variant="ghost" size="xs" className="gap-1 font-serif text-[11px]" onClick={() => void library.addPresets()}>
                <PackagePlus className="h-3 w-3" /> 添加{book.presets}
              </Button>
              <Button type="button" variant="ghost" size="xs" className="gap-1 font-serif text-[11px]" onClick={() => void library.importLibrary()}>
                <ArrowDownToLine className="h-3 w-3" /> 导入{book.name}
              </Button>
            </div>
          </div>
        ) : (
          <>
            <div className="flex items-center justify-between gap-2">
              <p className="min-w-0 truncate font-serif text-[11px] text-muted-foreground">
                {active.templates.length} {book.unit}{book.item}{active.templates.length > 1 && " · 拖动左侧把手排序"}
              </p>
              <div className="flex shrink-0 items-center gap-0.5">
                <Button type="button" variant="ghost" size="xs"
                  className="h-6 gap-1 px-1.5 font-serif text-[11px] text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]"
                  disabled={library.templateCount >= PROMPT_LIMITS.templates}
                  onClick={() => setEditor({ groupId: active.id, draft: { title: "", note: "", prompt: "" } })}>
                  <Plus className="h-3.5 w-3.5" /> 新建{book.item}
                </Button>
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "h-6 w-6 hover:text-foreground")}
                  title={`重命名${book.group}`} aria-label={`重命名${book.group}`} onClick={() => setNaming({ groupId: active.id })}>
                  <Pencil className="h-3 w-3" />
                </Button>
                <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "h-6 w-6 hover:text-destructive")}
                  title={`删除${book.group}`} aria-label={`删除${book.group}`} onClick={() => confirmDeleteGroup(active)}>
                  <Trash2 className="h-3 w-3" />
                </Button>
              </div>
            </div>
            <div ref={listRef} className={cn(
              "flex max-h-96 min-h-[9rem] flex-col gap-2 overflow-y-auto overscroll-contain pr-1",
              templateSort.drag && "select-none",
            )}>
              {active.templates.length === 0 ? (
                <p className="py-6 text-center font-serif text-xs leading-relaxed text-muted-foreground">
                  {materials
                    ? "这个分类还没有素材。点「新建素材」添加一条描述片段。"
                    : <>这个分组还没有模版。点「新建模版」，<br />或在提示词框旁点「存为模版」把写好的提示词收进来。</>}
                </p>
              ) : active.templates.map((template, index) => card(active, template, index))}
            </div>
          </>
        )}
      </>
    );
  };

  // Called, not rendered as <GroupTabs />: a component defined here would remount
  // on every render and drop the focus of a group name being typed.
  const groupTabs = () => (
    <div role="tablist" aria-label={materials ? "素材分类" : "提示词分组"} className={cn(
      "flex flex-wrap gap-1 rounded-lg bg-background/45 p-1 ring-1 ring-border/45",
      groupSort.drag && "select-none",
    )}>
      {groups.map((group, index) => {
        const selected = group.id === active?.id;
        if (naming?.groupId === group.id) {
          return (
            <GroupNameInput key={group.id} label={book.group} initial={group.name} taken={name => nameTaken(name, group.id)}
              onSubmit={name => void submitGroupName(name)} onCancel={() => setNaming(null)} />
          );
        }
        const dragging = groupSort.drag?.from === index;
        const dropTarget = groupSort.drag?.over === index && !dragging;
        return (
          <button
            key={group.id}
            ref={groupSort.itemRef(index)}
            type="button"
            role="tab"
            aria-selected={selected}
            data-prompt-group={group.name}
            title="单击切换 · 双击重命名 · 拖动或 Alt + ←→ 排序"
            onClick={() => selectGroup(group.id)}
            onDoubleClick={() => setNaming({ groupId: group.id })}
            {...groupSort.handleProps(index, groups.length)}
            className={cn(
              "inline-flex h-7 max-w-[12rem] shrink-0 touch-none select-none items-center justify-center gap-1 whitespace-nowrap rounded-md px-2.5 font-serif text-[11px] outline-none transition-[background-color,color,box-shadow,opacity] focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 focus-visible:ring-offset-1",
              selected
                ? "bg-background font-bold text-[#b83b38] shadow-xs ring-1 ring-[#b83b38]/20 dark:bg-white/[0.06] dark:text-[#fca5a5] dark:ring-[#d94f4b]/25"
                : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
              dragging && "opacity-50",
              dropTarget && "ring-2 ring-[#b83b38]/40",
            )}
          >
            <span className="truncate">{group.name}</span>
            <span className={cn("min-w-3.5 text-center font-sans text-[9px] tabular-nums opacity-60", selected && "opacity-75")}>
              {group.templates.length}
            </span>
          </button>
        );
      })}
      {naming?.groupId === null ? (
        <GroupNameInput label={book.group} initial="" taken={name => nameTaken(name, null)}
          onSubmit={name => void submitGroupName(name)} onCancel={() => setNaming(null)} />
      ) : (
        <button
          type="button"
          onClick={() => setNaming({ groupId: null })}
          disabled={groups.length >= PROMPT_LIMITS.groups}
          className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md border border-dashed border-border/70 px-2 font-serif text-[11px] text-muted-foreground outline-none transition-colors hover:border-[#b83b38]/40 hover:text-foreground focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 disabled:opacity-50"
        >
          <Plus className="h-3 w-3" /> 新建{book.group}
        </button>
      )}
    </div>
  );

  return (
    <div data-prompt-library className="space-y-2.5 rounded-xl bg-black/[0.025] p-2.5 ring-1 ring-border/50 dark:bg-white/[0.02] sm:space-y-3 sm:p-3">
      <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1 text-xs">
        <span className="inline-flex min-w-0 items-center gap-1.5 font-serif font-semibold text-muted-foreground">
          <Sparkles className="h-3.5 w-3.5 shrink-0 text-[#b83b38] dark:text-[#d94f4b]" />
          <span className="shrink-0">画谱 ·</span>
          <span role="tablist" aria-label="画谱分册" className="inline-flex shrink-0 items-center gap-0.5">
            {BOOK_TABS.map(tab => (
              <button key={tab.kind} type="button" role="tab" aria-selected={tab.kind === kind} title={tab.title}
                onClick={() => { onKindChange(tab.kind); setCollapsed(false); }}
                className={cn(
                  "rounded px-1 py-px font-serif text-xs outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[#b83b38]/35",
                  tab.kind === kind
                    ? "font-semibold text-[#b83b38] underline decoration-[#b83b38]/40 underline-offset-4 dark:text-[#fca5a5]"
                    : "font-normal text-muted-foreground hover:text-foreground",
                )}>
                {tab.label}
              </button>
            ))}
          </span>
          {library.templateCount > 0 && (
            <span className="shrink-0 rounded-full bg-[#b83b38]/10 px-1.5 py-px text-[10px] font-normal text-[#b83b38] dark:bg-[#d94f4b]/15 dark:text-[#fca5a5]">
              {library.templateCount} {book.unit}{book.item}
            </span>
          )}
        </span>
        <div className="flex items-center gap-0.5">
          {library.ready && !collapsed && (
            <>
              <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                title={`把 AgentHarbor 自带的${book.presets}加进来：同名${book.group}合并，已有的跳过，删掉的可以找回`}
                onClick={() => void library.addPresets()}>
                <PackagePlus className="h-3.5 w-3.5" /> 预置
              </Button>
              <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                title={`从 JSON 文件导入${book.group}和${book.item}（同名${book.group}合并，重复${book.item}跳过）`} onClick={() => void library.importLibrary()}>
                <ArrowDownToLine className="h-3.5 w-3.5" /> 导入
              </Button>
              <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
                title={materials ? "把全部素材导出为 JSON 文件" : "把整本画谱导出为 JSON 文件"} disabled={groups.length === 0} onClick={() => void library.exportLibrary()}>
                <ArrowUpFromLine className="h-3.5 w-3.5" /> 导出
              </Button>
            </>
          )}
          <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
            onClick={() => setCollapsed(!collapsed)}>
            <span>{collapsed ? "展开画谱" : "收起画谱"}</span>
            {collapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
          </Button>
        </div>
      </div>

      {!collapsed && (
        <>
          {library.ready && (library.templateCount > 0 || searching) && (
            <div className="relative">
              <Search className="pointer-events-none absolute left-2 top-1/2 h-3 w-3 -translate-y-1/2 text-muted-foreground/70" />
              <input
                value={query}
                onChange={event => setQuery(event.target.value)}
                placeholder={`检索${book.name}：标题 / ${book.note} / ${book.content} / ${book.group}`}
                className="h-8 w-full min-w-0 rounded-md border border-border/60 bg-background/80 pl-7 pr-7 text-xs outline-none transition-colors focus:border-[#b83b38]/40"
                aria-label={`检索${book.name}`}
              />
              {query && (
                <button type="button" onClick={() => setQuery("")} aria-label="清除检索"
                  className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground/70 hover:text-foreground">
                  <X className="h-3 w-3" />
                </button>
              )}
            </div>
          )}
          {body()}
        </>
      )}

      <PromptTemplateDialog library={library} request={editor} onClose={() => setEditor(null)} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}

/** A group name being typed: Enter or leaving the field keeps it, Esc drops it. */
function GroupNameInput({ label, initial, taken, onSubmit, onCancel }: {
  label: string;
  initial: string;
  taken: (name: string) => boolean;
  onSubmit: (name: string) => void;
  onCancel: () => void;
}) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const name = value.trim();
  const error = !name ? `${label}名称不能为空` : taken(name) ? `已有同名${label}` : null;
  const finish = (keep: boolean) => {
    if (done.current) return;
    done.current = true;
    if (keep && !error && name !== initial) onSubmit(name);
    else onCancel();
  };
  return (
    <input
      autoFocus
      value={value}
      maxLength={PROMPT_LIMITS.groupName}
      onChange={event => setValue(event.target.value)}
      onKeyDown={event => {
        if (event.key === "Enter") {
          event.preventDefault();
          // A taken name stays open to be fixed; an empty one just closes.
          if (!error || !name) finish(true);
        }
        if (event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      }}
      onBlur={() => finish(true)}
      placeholder={`${label}名称`}
      aria-label={`${label}名称`}
      aria-invalid={Boolean(error && name)}
      title={error && name ? error : "回车确认 · Esc 取消"}
      className={cn(
        "h-7 w-32 rounded-md border bg-background px-2 font-serif text-[11px] outline-none focus:border-[#b83b38]/50",
        error && name ? "border-destructive/60" : "border-border/70",
      )}
    />
  );
}

/** Edit a template in place; the expand button carries the draft into the dialog. */
function InlineTemplateEditor({ template, book, onCancel, onSave, onExpand }: {
  template: PromptTemplate;
  book: PromptLibraryState["book"];
  onCancel: () => void;
  onSave: (draft: TemplateDraft) => Promise<void>;
  onExpand: (draft: TemplateDraft) => void;
}) {
  const [draft, setDraft] = useState<TemplateDraft>(() => draftOf(template));
  const [saving, setSaving] = useState(false);
  const length = charCount(draft.prompt);
  const invalid = !draft.prompt.trim() || length > PROMPT_LIMITS.prompt;
  const save = async () => {
    if (invalid || saving) return;
    setSaving(true);
    await onSave(draft);
    setSaving(false);
  };
  const field = "h-8 bg-background/90 font-serif text-xs";
  return (
    <div
      data-prompt-template-editor={template.title}
      onKeyDown={editorKeys(() => void save(), onCancel)}
      className="shrink-0 space-y-1.5 rounded-lg border border-[#b83b38]/40 bg-background/90 p-2 shadow-sm sm:p-2.5"
    >
      <Input value={draft.title} maxLength={PROMPT_LIMITS.title} aria-label="标题" placeholder={`标题（留空则取${book.content}开头）`}
        onChange={event => setDraft({ ...draft, title: event.target.value })} className={cn(field, "font-semibold")} />
      <Input value={draft.note} maxLength={PROMPT_LIMITS.note} aria-label={book.note} placeholder={`${book.note}（可选）`}
        onChange={event => setDraft({ ...draft, note: event.target.value })} className={field} />
      <Textarea
        autoFocus
        value={draft.prompt}
        aria-label={book.content}
        onChange={event => setDraft({ ...draft, prompt: event.target.value })}
        onFocus={event => event.currentTarget.setSelectionRange(event.currentTarget.value.length, event.currentTarget.value.length)}
        className="max-h-80 min-h-[7rem] resize-y bg-background/90 font-serif text-xs leading-relaxed [field-sizing:content] focus:border-[#b83b38]/50 focus:ring-1 focus:ring-[#b83b38]/30"
      />
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className={cn("font-mono text-[10px] tabular-nums text-muted-foreground", length > PROMPT_LIMITS.prompt && "text-destructive")}>
          {length}/{PROMPT_LIMITS.prompt} · Ctrl + Enter 保存 · Esc 取消
        </span>
        <div className="flex items-center gap-1">
          <Button type="button" size="icon" variant="ghost" className={cn(iconButton, "hover:text-foreground")}
            title="在弹窗中继续编辑" aria-label="在弹窗中继续编辑" onClick={() => onExpand(draft)}>
            <Maximize2 className="h-3.5 w-3.5" />
          </Button>
          <Button type="button" variant="outline" size="xs" className="h-7 font-serif" onClick={onCancel} disabled={saving}>取消</Button>
          <Button type="button" size="xs" className="ink-generate-button h-7 font-serif text-white" onClick={() => void save()} disabled={invalid || saving}>
            {saving ? "保存中…" : "保存"}
          </Button>
        </div>
      </div>
    </div>
  );
}
