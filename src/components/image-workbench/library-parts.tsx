import { useRef, useState, type ReactNode } from "react";
import { ChevronDown, ChevronUp, Sparkles } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import { PROMPT_LIMITS, type LibraryBook } from "./use-prompt-library";

const BOOK_TABS: { kind: LibraryBook; label: string; title: string }[] = [
  { kind: "prompts", label: "提示词灵感", title: "整段的提示词模版，点一下填入提示词框" },
  { kind: "materials", label: "素材", title: "服装、动作、视角等描述片段；AI 润色时勾选分类或条目，模型会从中挑选" },
  { kind: "images", label: "图片", title: "自己准备的参考图片，点一下加入图生图参考图；AI 润色时会连名字和备注一起给模型看" },
];

/** The 画谱's title row: the book tabs, a count, the book's own actions and the fold. */
export function BookHeader({ kind, onKindChange, count, countLabel, collapsed, onCollapsedChange, actions }: {
  kind: LibraryBook;
  onKindChange: (kind: LibraryBook) => void;
  count: number;
  countLabel: string;
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  /** Shown while the book is open. */
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-2 gap-y-1 text-xs">
      <span className="inline-flex min-w-0 items-center gap-1.5 font-serif font-semibold text-muted-foreground">
        <Sparkles className="h-3.5 w-3.5 shrink-0 text-[#b83b38] dark:text-[#d94f4b]" />
        <span className="shrink-0">画谱 ·</span>
        <span role="tablist" aria-label="画谱分册" className="inline-flex shrink-0 items-center gap-0.5">
          {BOOK_TABS.map(tab => (
            <button key={tab.kind} type="button" role="tab" aria-selected={tab.kind === kind} title={tab.title}
              onClick={() => { onKindChange(tab.kind); onCollapsedChange(false); }}
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
        {count > 0 && (
          <span className="shrink-0 rounded-full bg-[#b83b38]/10 px-1.5 py-px text-[10px] font-normal text-[#b83b38] dark:bg-[#d94f4b]/15 dark:text-[#fca5a5]">
            {count} {countLabel}
          </span>
        )}
      </span>
      <div className="flex items-center gap-0.5">
        {!collapsed && actions}
        <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
          onClick={() => onCollapsedChange(!collapsed)}>
          <span>{collapsed ? "展开画谱" : "收起画谱"}</span>
          {collapsed ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronUp className="h-3.5 w-3.5" />}
        </Button>
      </div>
    </div>
  );
}

/** A group name being typed: Enter or leaving the field keeps it, Esc drops it. */
export function GroupNameInput({ label, initial, taken, onSubmit, onCancel }: {
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
