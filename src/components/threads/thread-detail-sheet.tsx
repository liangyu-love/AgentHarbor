import { useEffect, useId, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import {
  AlertCircle,
  Archive,
  Bot,
  Brain,
  Check,
  ChevronDown,
  ChevronUp,
  Coins,
  Copy,
  Cpu,
  Download,
  FileCode,
  FileJson,
  Filter,
  Layers,
  Loader2,
  MessagesSquare,
  RefreshCw,
  Sparkles,
  Terminal,
  Trash2,
  User,
  Wrench,
} from "lucide-react";
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { formatDateTime } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { formatSize } from "./thread-utils";
import type {
  ThreadDetails,
  ThreadDiagnostics,
  ThreadMessagePage,
  ThreadSummary,
  ThreadUsage,
} from "@/types/threads";

const tabs = ["usage", "tools", "metadata", "conversation"] as const;
type Tab = typeof tabs[number];
const number = (value: number) => value.toLocaleString();

function Diagnostics({ data }: { data: ThreadDiagnostics }) {
  const { t } = useTranslation();
  const entries = Object.entries(data).filter(([, count]) => count > 0);
  if (!entries.length) return null;

  return (
    <details className="group rounded-2xl border border-amber-500/30 bg-amber-500/[0.04] p-3 text-xs transition-all shadow-xs">
      <summary className="flex cursor-pointer items-center justify-between font-medium text-amber-700 dark:text-amber-300 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-lg py-0.5">
        <span className="flex items-center gap-2">
          <AlertCircle className="h-4 w-4 text-amber-500" />
          <span>{t("threads.details.diagnostics")}</span>
        </span>
        <ChevronDown className="h-3.5 w-3.5 transition-transform duration-200 group-open:rotate-180" />
      </summary>
      <ul className="mt-2.5 space-y-1.5 border-t border-amber-500/20 pt-2 text-muted-foreground">
        {entries.map(([key, count]) => (
          <li key={key} className="flex items-start gap-1.5 leading-relaxed">
            <span className="mt-1 h-1.5 w-1.5 shrink-0 rounded-full bg-amber-500" />
            <span>{t(`threads.details.diagnostic.${key}`, { count })}</span>
          </li>
        ))}
      </ul>
    </details>
  );
}

function UsageBreakdown({ usage }: { usage: ThreadUsage }) {
  const { t } = useTranslation();
  const parts = [
    {
      key: "input",
      value: usage.cached === null ? null : Math.max(0, usage.input - usage.cached),
      color: "bg-sky-500",
      textColor: "text-sky-600 dark:text-sky-400",
      bgLight: "bg-sky-500/10",
      borderColor: "border-sky-500/20",
    },
    {
      key: "cached",
      value: usage.cached,
      color: "bg-emerald-500",
      textColor: "text-emerald-600 dark:text-emerald-400",
      bgLight: "bg-emerald-500/10",
      borderColor: "border-emerald-500/20",
    },
    {
      key: "output",
      value:
        usage.reasoning === null
          ? null
          : Math.max(0, usage.output - usage.reasoning),
      color: "bg-amber-500",
      textColor: "text-amber-600 dark:text-amber-400",
      bgLight: "bg-amber-500/10",
      borderColor: "border-amber-500/20",
    },
    {
      key: "reasoning",
      value: usage.reasoning,
      color: "bg-violet-500",
      textColor: "text-violet-600 dark:text-violet-400",
      bgLight: "bg-violet-500/10",
      borderColor: "border-violet-500/20",
    },
  ];

  const complete =
    parts.every((part) => part.value !== null) &&
    usage.cached! <= usage.input &&
    usage.reasoning! <= usage.output &&
    usage.total === usage.input + usage.output;

  return (
    <section className="space-y-4 rounded-2xl border border-border/70 bg-gradient-to-b from-card/90 to-card/50 p-4 backdrop-blur-sm shadow-xs">
      {/* Hero Metric */}
      <div>
        <div className="flex items-center justify-between gap-2">
          <p className="flex items-center gap-1.5 text-xs font-medium text-muted-foreground">
            <Coins className="h-3.5 w-3.5 text-primary" />
            <span>{t("threads.details.recordedTokens")}</span>
          </p>
          {complete && usage.cached != null && usage.input > 0 && (
            <span className="rounded-full bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold text-emerald-600 dark:text-emerald-400">
              缓存命中 {((usage.cached / usage.input) * 100).toFixed(0)}%
            </span>
          )}
        </div>
        <p className="mt-1 text-3xl font-extrabold tracking-tight tabular-nums text-foreground">
          {number(usage.total)}{" "}
          <span className="text-sm font-medium text-muted-foreground">
            Token
          </span>
        </p>
      </div>

      {/* Progress Multi-Segment Bar */}
      {complete && usage.total > 0 && (
        <div className="space-y-1">
          <div
            aria-hidden="true"
            className="flex h-3 overflow-hidden rounded-full bg-muted/60 p-0.5 shadow-inner"
          >
            {parts.map((part) =>
              part.value && part.value > 0 ? (
                <div
                  key={part.key}
                  className={cn(part.color, "rounded-xs transition-all duration-500")}
                  style={{ width: `${(part.value / usage.total) * 100}%` }}
                />
              ) : null,
            )}
          </div>
        </div>
      )}

      {/* 2x2 Grid of Token Items */}
      <dl className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs">
        {parts.map((part) => (
          <div
            key={part.key}
            className={cn(
              "flex flex-col justify-between rounded-xl border p-2.5 transition-colors",
              part.bgLight,
              part.borderColor,
            )}
          >
            <dt className="flex items-center gap-1.5 font-medium text-muted-foreground">
              <span className={cn("h-2 w-2 rounded-full", part.color)} />
              <span>{t(`threads.details.${part.key}`)}</span>
            </dt>
            <dd className="mt-1.5 flex items-baseline justify-between gap-1">
              <span className="font-mono text-sm font-bold tabular-nums text-foreground">
                {part.value === null ? t("threads.details.unknown") : number(part.value)}
              </span>
              {complete && usage.total > 0 && part.value !== null && (
                <span className={cn("font-mono text-[11px] font-semibold", part.textColor)}>
                  {((part.value / usage.total) * 100).toFixed(1)}%
                </span>
              )}
            </dd>
          </div>
        ))}
      </dl>

      {!complete && (
        <p className="text-xs text-muted-foreground/80 leading-relaxed italic">
          {t("threads.details.incompleteBreakdown", {
            input: number(usage.input),
            output: number(usage.output),
          })}
        </p>
      )}
    </section>
  );
}

function UsageHistory({ detail }: { detail: ThreadDetails }) {
  const { t } = useTranslation();
  const gradientId = useId();
  let cumulative = 0;
  const dates = detail.byDate
    .filter((item) => item.label !== "unknown")
    .map((item) => ({ ...item, cumulative: (cumulative += item.usage.total) }));
  const recent = dates.slice(-30);
  const max = Math.max(1, cumulative);
  const point = (value: number, index: number) => [
    recent.length === 1 ? 176 : 44 + (index / (recent.length - 1)) * 264,
    108 - (value / max) * 92,
  ];

  return (
    <>
      <section className="space-y-3 rounded-2xl border border-border/70 bg-card/60 p-4 backdrop-blur-sm shadow-xs">
        <div className="flex items-center justify-between gap-2">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold tracking-tight text-foreground">
            <Sparkles className="h-4 w-4 text-primary" />
            <span>{t("threads.details.cumulative")}</span>
          </h3>
          <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-mono text-muted-foreground">
            {t("threads.details.dateCount", { count: dates.length })}
          </span>
        </div>

        {recent.length ? (
          <div className="space-y-3">
            <div className="rounded-xl border border-border/50 bg-black/[0.02] p-2 dark:bg-white/[0.02]">
              <svg
                viewBox="0 0 320 120"
                className="w-full text-primary overflow-visible"
                role="img"
                aria-label={t("threads.details.cumulativeChart")}
              >
                <defs>
                  <linearGradient id={gradientId} x1="0" y1="0" x2="0" y2="1">
                    <stop offset="0%" stopColor="currentColor" stopOpacity="0.25" />
                    <stop offset="100%" stopColor="currentColor" stopOpacity="0.0" />
                  </linearGradient>
                </defs>
                {/* Horizontal guide lines */}
                {[16, 62, 108].map((y, index) => (
                  <g key={y}>
                    <line
                      x1="44"
                      x2="308"
                      y1={y}
                      y2={y}
                      className="stroke-border/70"
                      strokeDasharray={index === 2 ? undefined : "3 3"}
                    />
                    <text
                      x="38"
                      y={y + 3}
                      textAnchor="end"
                      className="fill-muted-foreground font-mono text-[9px]"
                    >
                      {new Intl.NumberFormat(undefined, {
                        notation: "compact",
                        maximumFractionDigits: 1,
                      }).format(max * (1 - index / 2))}
                    </text>
                  </g>
                ))}

                {/* Area under curve */}
                {recent.length > 1 && (
                  <polygon
                    points={`44,108 ${recent
                      .map((item, index) => point(item.cumulative, index).join(","))
                      .join(" ")} 308,108`}
                    fill={`url(#${gradientId})`}
                  />
                )}

                {/* Line */}
                {recent.length > 1 && (
                  <polyline
                    points={recent
                      .map((item, index) => point(item.cumulative, index).join(","))
                      .join(" ")}
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="2.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                )}

                {/* Points */}
                {[recent[0], ...(recent.length > 1 ? [recent[recent.length - 1]] : [])].map(
                  (item, index) => {
                    const [x, y] = point(
                      item.cumulative,
                      index ? recent.length - 1 : 0,
                    );
                    return (
                      <g key={item.label}>
                        <circle
                          cx={x}
                          cy={y}
                          r="5"
                          className="fill-background stroke-primary stroke-2"
                        />
                        <circle cx={x} cy={y} r="2.5" fill="currentColor" />
                      </g>
                    );
                  },
                )}
              </svg>

              <div className="mt-1 flex justify-between font-mono text-[11px] text-muted-foreground px-1">
                <span>{recent[0].label}</span>
                <span className="font-semibold text-foreground">
                  {number(cumulative)} Token · {recent[recent.length - 1].label}
                </span>
              </div>
            </div>

            {/* Daily Records Accordion */}
            <details className="group rounded-xl border border-border/50 bg-muted/15 p-2 text-xs">
              <summary className="flex cursor-pointer items-center justify-between font-medium text-muted-foreground hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-lg px-1.5 py-1">
                <span>{t("threads.details.dailyRecords")}</span>
                <ChevronDown className="h-3.5 w-3.5 transition-transform duration-200 group-open:rotate-180" />
              </summary>
              <div className="mt-2 max-h-48 overflow-auto border-t border-border/40 pt-2 px-1">
                <table className="w-full text-left font-mono text-[11px] tabular-nums">
                  <thead className="text-muted-foreground border-b border-border/40">
                    <tr>
                      <th className="py-1.5 font-semibold text-foreground">
                        {t("threads.details.date")}
                      </th>
                      <th className="text-right py-1.5 font-semibold text-foreground">
                        Token
                      </th>
                      <th className="text-right py-1.5 font-semibold text-foreground">
                        {t("threads.details.cumulative")}
                      </th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-border/25">
                    {dates.map((item) => (
                      <tr key={item.label} className="hover:bg-muted/30">
                        <td className="py-1.5 text-muted-foreground">{item.label}</td>
                        <td className="text-right py-1.5 font-medium">{number(item.usage.total)}</td>
                        <td className="text-right py-1.5 font-bold text-primary">
                          {number(item.cumulative)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </details>
          </div>
        ) : (
          <p className="text-xs text-muted-foreground italic">
            {t("threads.details.noDates")}
          </p>
        )}
      </section>

      {/* Model Attribution */}
      <section className="space-y-3 rounded-2xl border border-border/70 bg-card/60 p-4 backdrop-blur-sm shadow-xs">
        <div className="flex items-center justify-between gap-2">
          <h3 className="flex items-center gap-1.5 text-sm font-semibold tracking-tight text-foreground">
            <Cpu className="h-4 w-4 text-primary" />
            <span>{t("threads.details.models")}</span>
          </h3>
          <span className="rounded-full bg-muted px-2 py-0.5 text-xs font-mono text-muted-foreground">
            {detail.byModel.length} 个
          </span>
        </div>
        <p className="text-xs leading-relaxed text-muted-foreground">
          {t("threads.details.attributionNote")}
        </p>
        <dl className="divide-y divide-border/40 text-xs">
          {detail.byModel.map((item) => (
            <div
              key={item.label}
              className="flex items-center justify-between gap-3 py-2 hover:bg-muted/20 px-1 rounded-lg transition-colors"
            >
              <dt className="flex min-w-0 items-center gap-2 [overflow-wrap:anywhere]">
                <Badge variant="outline" className="font-mono text-[10px] shrink-0">
                  Model
                </Badge>
                <span className="font-mono font-medium text-foreground break-all">
                  {item.label === "unknown" ? t("threads.details.unknown") : item.label}
                </span>
              </dt>
              <dd className="shrink-0 font-mono font-bold tabular-nums text-primary">
                {number(item.usage.total)} <span className="text-[10px] font-normal text-muted-foreground">Token</span>
              </dd>
            </div>
          ))}
        </dl>
      </section>
    </>
  );
}

interface ContentBlock {
  type: "text" | "code";
  content: string;
  lang?: string;
}

function parseMessageContent(text: string): ContentBlock[] {
  if (!text.includes("```")) {
    return [{ type: "text", content: text }];
  }
  const blocks: ContentBlock[] = [];
  const regex = /```([a-zA-Z0-9_-]*)\n([\s\S]*?)```/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;

  while ((match = regex.exec(text)) !== null) {
    if (match.index > lastIndex) {
      blocks.push({
        type: "text",
        content: text.slice(lastIndex, match.index),
      });
    }
    blocks.push({
      type: "code",
      lang: match[1] || "code",
      content: match[2],
    });
    lastIndex = regex.lastIndex;
  }

  if (lastIndex < text.length) {
    blocks.push({
      type: "text",
      content: text.slice(lastIndex),
    });
  }

  return blocks;
}

function CodeBlockItem({ code, lang }: { code: string; lang: string }) {
  const [copied, setCopied] = useState(false);

  const copyCode = async (e: React.MouseEvent) => {
    e.stopPropagation();
    try {
      await navigator.clipboard.writeText(code);
      setCopied(true);
      toast({ title: "已复制代码块" });
      setTimeout(() => setCopied(false), 2000);
    } catch {
      toast({ title: "复制代码失败", variant: "destructive" });
    }
  };

  const lineCount = code.split("\n").length;

  return (
    <div className="my-2.5 overflow-hidden rounded-xl border border-border/80 bg-zinc-950 text-zinc-100 shadow-md dark:bg-black/90 dark:border-border/60">
      <div className="flex items-center justify-between border-b border-zinc-800 bg-zinc-900/90 px-3 py-1.5 text-xs text-zinc-400">
        <div className="flex items-center gap-2">
          <FileCode className="h-3.5 w-3.5 text-primary" />
          <span className="font-mono text-[11px] font-semibold text-zinc-300">
            {lang}
          </span>
          <span className="text-[10px] text-zinc-500">
            {lineCount} 行
          </span>
        </div>
        <button
          type="button"
          onClick={copyCode}
          className="flex items-center gap-1 rounded-md px-2 py-0.5 text-[11px] font-medium text-zinc-400 hover:bg-zinc-800 hover:text-zinc-200 transition-colors"
        >
          {copied ? (
            <Check className="h-3 w-3 text-emerald-400" />
          ) : (
            <Copy className="h-3 w-3" />
          )}
          <span>{copied ? "已复制" : "复制代码"}</span>
        </button>
      </div>
      <pre className="overflow-x-auto p-3 font-mono text-[12px] leading-relaxed text-zinc-200 selection:bg-primary/30">
        <code>{code}</code>
      </pre>
    </div>
  );
}

function MessageItem({
  message,
}: {
  message: ThreadMessagePage["messages"][number];
}) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const long = message.text.length > 1500;

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(message.text);
      setCopied(true);
      toast({ title: t("threads.details.copied") });
      setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      toast({
        title: t("threads.details.copyFailed"),
        description: formatInvokeError(error),
        variant: "destructive",
      });
    }
  };

  const isUser = message.role === "user";
  const isAssistant = message.role === "assistant";
  const isTool = message.role === "toolCall" || message.role === "toolResult";
  const isReasoning = message.role === "reasoning" || message.role === "compaction";

  const rawText = long && !expanded ? `${message.text.slice(0, 1500)}…` : message.text;
  const blocks = useMemo(() => parseMessageContent(rawText), [rawText]);

  return (
    <article
      className={cn(
        "group relative min-w-0 rounded-2xl border p-4 transition-all duration-200 shadow-xs",
        isUser
          ? "border-sky-500/25 bg-gradient-to-br from-sky-500/[0.04] to-transparent dark:bg-sky-500/[0.06]"
          : isAssistant
            ? "border-border/80 bg-card/90 shadow-2xs"
            : isTool
              ? "border-border/60 bg-muted/30 font-mono"
              : isReasoning
                ? "border-amber-500/25 bg-amber-500/[0.03]"
                : "border-border/60 bg-muted/20",
      )}
    >
      {/* Header bar */}
      <div className="mb-2.5 flex items-center justify-between gap-2 text-xs">
        <div className="flex items-center gap-2">
          <div
            className={cn(
              "flex h-6 w-6 items-center justify-center rounded-lg text-xs shadow-2xs",
              isUser
                ? "bg-sky-500/15 text-sky-600 dark:text-sky-400"
                : isAssistant
                  ? "bg-purple-500/15 text-purple-600 dark:text-purple-400"
                  : isTool
                    ? "bg-muted text-muted-foreground"
                    : "bg-amber-500/15 text-amber-600 dark:text-amber-400",
            )}
          >
            {isUser ? (
              <User className="h-3.5 w-3.5" />
            ) : isAssistant ? (
              <Bot className="h-3.5 w-3.5" />
            ) : isTool ? (
              <Terminal className="h-3.5 w-3.5" />
            ) : (
              <Brain className="h-3.5 w-3.5" />
            )}
          </div>
          <span className="font-bold tracking-tight text-foreground">
            {t(`threads.details.roles.${message.role}`, {
              defaultValue: message.role,
            })}
          </span>
          <span className="rounded-md bg-muted/70 px-1.5 py-0.2 font-mono text-[10px] text-muted-foreground">
            {message.text.length.toLocaleString()} 字符
          </span>
        </div>

        <div className="flex min-w-0 items-center gap-2">
          <time className="truncate font-mono text-[11px] text-muted-foreground/80">
            {message.timestamp}
          </time>
          <Button
            variant="ghost"
            size="icon-sm"
            className="h-6.5 w-6.5 shrink-0 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground active:scale-95 transition-all"
            aria-label={t("threads.details.copyMessage")}
            onClick={() => void copy()}
          >
            {copied ? (
              <Check className="h-3.5 w-3.5 text-emerald-500" />
            ) : (
              <Copy className="h-3.5 w-3.5" />
            )}
          </Button>
        </div>
      </div>

      {/* Body text / formatted code blocks */}
      <div className="relative text-sm leading-relaxed [overflow-wrap:anywhere] text-foreground/90 font-sans">
        {blocks.length === 1 && blocks[0].type === "text" ? (
          <p className="whitespace-pre-wrap break-words">{rawText}</p>
        ) : (
          <div className="space-y-1">
            {blocks.map((block, idx) =>
              block.type === "code" ? (
                <CodeBlockItem key={idx} code={block.content} lang={block.lang ?? "code"} />
              ) : (
                <p key={idx} className="whitespace-pre-wrap break-words">
                  {block.content}
                </p>
              ),
            )}
          </div>
        )}
      </div>

      {long && (
        <div className="mt-2.5 flex items-center justify-center border-t border-border/30 pt-2">
          <Button
            variant="ghost"
            size="sm"
            className="h-7 gap-1 px-3 text-xs font-semibold text-primary hover:bg-primary/10 rounded-xl"
            aria-expanded={expanded}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
            <span>{t(expanded ? "threads.details.collapse" : "threads.details.expand")}</span>
          </Button>
        </div>
      )}

      {message.truncated && (
        <div className="mt-2.5 flex items-center gap-1.5 rounded-xl border border-amber-500/20 bg-amber-500/[0.04] p-2 text-xs text-amber-600 dark:text-amber-400">
          <AlertCircle className="h-3.5 w-3.5 shrink-0" />
          <span>{t("threads.details.truncated")}</span>
        </div>
      )}
    </article>
  );
}

export function ThreadDetailSheet({
  thread,
  onClose,
  onExport,
  onDelete,
  onOpenRaw,
}: {
  thread: ThreadSummary;
  onClose: () => void;
  onExport: () => void;
  onDelete: () => void;
  onOpenRaw: () => void;
}) {
  const { t } = useTranslation();
  const opener = useRef(
    typeof document !== "undefined" && document.activeElement instanceof HTMLElement
      ? document.activeElement
      : null,
  );
  const tabList = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const [tab, setTab] = useState<Tab>("usage");
  const [offset, setOffset] = useState(0);
  const [roleFilter, setRoleFilter] = useState<"all" | "user" | "assistant" | "tools">("all");
  const available = thread.rolloutStatus !== "missing";

  useEffect(() => {
    content.current?.scrollTo(0, 0);
  }, [tab, offset]);

  const detailQuery = useQuery({
    queryKey: queryKeys.threadDetails(thread.key),
    queryFn: () => api.conversations.details("codex", thread.key, thread.id),
    enabled: available,
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const detail = detailQuery.data?.data;

  const messageQuery = useQuery({
    queryKey: queryKeys.threadMessages(
      thread.key,
      detail?.snapshotBytes ?? 0,
      offset,
    ),
    queryFn: () =>
      api.conversations.messages(
        "codex",
        thread.key,
        thread.id,
        detail!.snapshotBytes,
        offset,
      ),
    enabled: tab === "conversation" && !!detail,
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  });

  const rawMessages = messageQuery.data?.data.messages ?? [];
  const filteredMessages = useMemo(() => {
    if (roleFilter === "all") return rawMessages;
    if (roleFilter === "user") return rawMessages.filter((m) => m.role === "user");
    if (roleFilter === "assistant") return rawMessages.filter((m) => m.role === "assistant");
    if (roleFilter === "tools")
      return rawMessages.filter(
        (m) =>
          m.role === "toolCall" ||
          m.role === "toolResult" ||
          m.role === "reasoning" ||
          m.role === "compaction",
      );
    return rawMessages;
  }, [rawMessages, roleFilter]);

  const copyId = async () => {
    try {
      await navigator.clipboard.writeText(thread.id);
      toast({ title: t("threads.copiedId") });
    } catch (error) {
      toast({
        title: t("threads.details.copyFailed"),
        description: formatInvokeError(error),
        variant: "destructive",
      });
    }
  };

  const metadata = [
    ["id", thread.id],
    ["cwd", thread.cwd],
    ["file", thread.filePath],
    ["provider", thread.modelProvider],
    ["model", thread.model],
    ["created", thread.createdAt ? formatDateTime(thread.createdAt) : ""],
    ["updated", thread.updatedAt ? formatDateTime(thread.updatedAt) : ""],
    ["version", thread.cliVersion],
    ["source", `${thread.threadSource} · ${thread.source}`],
    ["size", formatSize(thread.fileSize)],
  ];

  return (
    <Sheet open onOpenChange={(open) => !open && onClose()}>
      <SheetContent
        closeLabel={t("common.close")}
        className="flex h-[100dvh] w-full flex-col gap-0 p-0 sm:w-[560px] sm:max-w-[560px] md:w-[620px] md:max-w-[620px] xl:w-[700px] xl:max-w-[700px] shadow-2xl border-l border-border/80 bg-background/95 backdrop-blur-xl"
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          if (opener.current?.isConnected) opener.current.focus();
        }}
      >
        {/* Sheet Header */}
        <SheetHeader className="shrink-0 space-y-2 border-b border-border/60 bg-muted/20 px-6 pb-4 pt-5 text-left">
          <div className="flex items-start justify-between gap-3 pr-8">
            <SheetTitle className="text-base font-bold leading-snug tracking-tight text-foreground [overflow-wrap:anywhere]">
              {thread.title || t("threads.untitled")}
            </SheetTitle>
          </div>
          <SheetDescription className="text-xs text-muted-foreground">
            {t("threads.details.readOnly")}
          </SheetDescription>

          <div className="flex flex-wrap items-center gap-2 pt-1 text-xs">
            {thread.archived ? (
              <Badge
                variant="outline"
                className="gap-1 border-amber-500/30 bg-amber-500/10 text-amber-600 dark:text-amber-400 text-[10px]"
              >
                <Archive className="h-2.5 w-2.5" />
                {t("threads.archivedBadge")}
              </Badge>
            ) : (
              <Badge
                variant="outline"
                className="gap-1 border-emerald-500/30 bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 text-[10px]"
              >
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500 animate-pulse" />
                {t("threads.activeBadge")}
              </Badge>
            )}

            <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-[10px] text-muted-foreground">
              {t("threads.details.messages", {
                count: thread.userMessages + thread.agentMessages,
              })}
            </span>

            {detail && (
              <span className="rounded-md bg-muted px-2 py-0.5 font-mono text-[10px] text-muted-foreground">
                {t("threads.details.modelCount", {
                  count: detail.byModel.length,
                })}
              </span>
            )}
          </div>
        </SheetHeader>

        {/* Tab List */}
        <div
          ref={tabList}
          role="tablist"
          aria-label={t("threads.details.tabs")}
          className="flex shrink-0 border-b border-border/60 bg-muted/10 px-4"
          onKeyDown={(event) => {
            const index = tabs.indexOf(tab);
            const next =
              event.key === "ArrowRight"
                ? (index + 1) % tabs.length
                : event.key === "ArrowLeft"
                  ? (index + tabs.length - 1) % tabs.length
                  : event.key === "Home"
                    ? 0
                    : event.key === "End"
                      ? tabs.length - 1
                      : null;
            if (next === null) return;
            event.preventDefault();
            setTab(tabs[next]);
            tabList.current
              ?.querySelectorAll<HTMLButtonElement>("button")
              [next]?.focus();
          }}
        >
          {tabs.map((name) => {
            const isSelected = tab === name;
            return (
              <button
                key={name}
                type="button"
                role="tab"
                id={`thread-tab-${name}`}
                aria-controls="thread-detail-content"
                aria-selected={isSelected}
                tabIndex={isSelected ? 0 : -1}
                className={cn(
                  "relative flex items-center gap-1.5 border-b-2 px-3.5 py-3 text-xs font-semibold tracking-tight transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
                  isSelected
                    ? "border-primary text-primary"
                    : "border-transparent text-muted-foreground hover:text-foreground",
                )}
                onClick={() => setTab(name)}
              >
                {name === "usage" && <Coins className="h-3.5 w-3.5" />}
                {name === "tools" && <Wrench className="h-3.5 w-3.5" />}
                {name === "metadata" && <Layers className="h-3.5 w-3.5" />}
                {name === "conversation" && <MessagesSquare className="h-3.5 w-3.5" />}
                <span>{t(`threads.details.${name}`)}</span>
              </button>
            );
          })}
        </div>

        {/* Tab Panel Content */}
        <div
          ref={content}
          role="tabpanel"
          id="thread-detail-content"
          aria-labelledby={`thread-tab-${tab}`}
          tabIndex={0}
          className="min-h-0 flex-1 space-y-4 overflow-y-auto overscroll-contain p-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        >
          {tab === "metadata" ? (
            <div className="rounded-2xl border border-border/70 bg-card/60 p-4 backdrop-blur-sm shadow-xs">
              <dl className="space-y-3.5 text-xs">
                {metadata.map(([key, value]) => (
                  <div
                    key={key}
                    className="flex flex-col gap-1 border-b border-border/30 pb-3 last:border-b-0 last:pb-0"
                  >
                    <dt className="font-semibold text-muted-foreground">
                      {t(`threads.details.meta.${key}`)}
                    </dt>
                    <dd className="whitespace-pre-wrap break-words font-mono text-foreground [overflow-wrap:anywhere] bg-muted/30 p-2 rounded-lg">
                      {value || t("threads.details.unknown")}
                    </dd>
                  </div>
                ))}
              </dl>
            </div>
          ) : !available ? (
            <div className="rounded-2xl border border-destructive/30 bg-destructive/5 p-6 text-center">
              <AlertCircle className="mx-auto h-8 w-8 text-destructive" />
              <p className="mt-2 text-sm font-semibold text-destructive">
                {t("threads.details.noRollout")}
              </p>
            </div>
          ) : detailQuery.isPending ? (
            <div
              role="status"
              className="flex flex-col items-center justify-center gap-3 py-16 text-center text-muted-foreground"
            >
              <Loader2 className="h-8 w-8 animate-spin text-primary" />
              <p className="text-sm font-medium">{t("threads.details.loading")}</p>
            </div>
          ) : detailQuery.isError ? (
            <div
              role="alert"
              className="rounded-2xl border border-destructive/40 bg-destructive/5 p-5 space-y-3 text-center"
            >
              <AlertCircle className="mx-auto h-8 w-8 text-destructive" />
              <p className="break-words text-sm font-medium text-destructive [overflow-wrap:anywhere]">
                {formatInvokeError(detailQuery.error)}
              </p>
              <Button
                variant="outline"
                size="sm"
                className="rounded-lg"
                onClick={() => void detailQuery.refetch()}
              >
                {t("threads.retry")}
              </Button>
            </div>
          ) : (
            detail && (
              <>
                <Diagnostics data={detail.diagnostics} />

                {tab === "usage" &&
                  (detail.usage ? (
                    <>
                      <UsageBreakdown usage={detail.usage} />
                      <UsageHistory detail={detail} />
                    </>
                  ) : (
                    <div className="rounded-2xl border border-dashed border-border/70 p-8 text-center text-muted-foreground">
                      <Coins className="mx-auto h-8 w-8 opacity-40" />
                      <p className="mt-2 text-sm">{t("threads.details.noUsage")}</p>
                    </div>
                  ))}

                {tab === "tools" &&
                  (detail.tools.length ? (
                    <div className="space-y-2">
                      <div className="flex items-center justify-between text-xs text-muted-foreground px-1 pb-1">
                        <span>工具标识符</span>
                        <span>调用频次</span>
                      </div>
                      <dl className="space-y-2 text-xs">
                        {detail.tools.map((tool) => (
                          <div
                            key={tool.name}
                            className="flex items-center justify-between gap-3 rounded-xl border border-border/70 bg-card/70 p-3 shadow-xs"
                          >
                            <dt className="flex min-w-0 items-center gap-2 font-mono font-medium text-foreground [overflow-wrap:anywhere]">
                              <Terminal className="h-3.5 w-3.5 text-primary shrink-0" />
                              <span className="truncate">{tool.name}</span>
                            </dt>
                            <dd className="shrink-0 rounded-full bg-primary/10 px-2.5 py-0.5 font-mono text-xs font-bold tabular-nums text-primary">
                              {t("threads.details.calls", { count: tool.calls })}
                            </dd>
                          </div>
                        ))}
                      </dl>
                    </div>
                  ) : (
                    <div className="rounded-2xl border border-dashed border-border/70 p-8 text-center text-muted-foreground">
                      <Wrench className="mx-auto h-8 w-8 opacity-40" />
                      <p className="mt-2 text-sm">{t("threads.details.noTools")}</p>
                    </div>
                  ))}

                {tab === "conversation" && (
                  <div className="space-y-3.5">
                    {/* Header Note */}
                    <p className="rounded-xl bg-muted/40 p-2.5 text-xs text-muted-foreground leading-relaxed border border-border/40">
                      {t("threads.details.conversationNote")}
                    </p>

                    {/* Role Filter Bar */}
                    {rawMessages.length > 0 && (
                      <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/40 pb-2">
                        <div className="flex items-center gap-1 text-xs">
                          <Filter className="h-3 w-3 text-muted-foreground mr-1" />
                          <button
                            type="button"
                            onClick={() => setRoleFilter("all")}
                            className={cn(
                              "rounded-lg px-2 py-0.5 text-xs font-medium transition-colors",
                              roleFilter === "all"
                                ? "bg-primary text-primary-foreground font-semibold"
                                : "text-muted-foreground hover:text-foreground hover:bg-muted/60",
                            )}
                          >
                            全部 ({rawMessages.length})
                          </button>
                          <button
                            type="button"
                            onClick={() => setRoleFilter("user")}
                            className={cn(
                              "rounded-lg px-2 py-0.5 text-xs font-medium transition-colors",
                              roleFilter === "user"
                                ? "bg-sky-500 text-white font-semibold"
                                : "text-muted-foreground hover:text-foreground hover:bg-muted/60",
                            )}
                          >
                            用户 ({rawMessages.filter((m) => m.role === "user").length})
                          </button>
                          <button
                            type="button"
                            onClick={() => setRoleFilter("assistant")}
                            className={cn(
                              "rounded-lg px-2 py-0.5 text-xs font-medium transition-colors",
                              roleFilter === "assistant"
                                ? "bg-purple-500 text-white font-semibold"
                                : "text-muted-foreground hover:text-foreground hover:bg-muted/60",
                            )}
                          >
                            助手 ({rawMessages.filter((m) => m.role === "assistant").length})
                          </button>
                          <button
                            type="button"
                            onClick={() => setRoleFilter("tools")}
                            className={cn(
                              "rounded-lg px-2 py-0.5 text-xs font-medium transition-colors",
                              roleFilter === "tools"
                                ? "bg-zinc-700 text-white font-semibold"
                                : "text-muted-foreground hover:text-foreground hover:bg-muted/60",
                            )}
                          >
                            工具 ({rawMessages.filter((m) => m.role !== "user" && m.role !== "assistant").length})
                          </button>
                        </div>

                        <span className="font-mono text-[11px] text-muted-foreground">
                          第 {offset / 50 + 1} 页
                        </span>
                      </div>
                    )}

                    {messageQuery.isPending ? (
                      <div
                        role="status"
                        className="flex flex-col items-center justify-center gap-3 py-16 text-center text-muted-foreground"
                      >
                        <Loader2 className="h-6 w-6 animate-spin text-primary" />
                        <p className="text-xs">{t("threads.details.loading")}</p>
                      </div>
                    ) : messageQuery.isError ? (
                      <div
                        role="alert"
                        className="rounded-xl border border-destructive/30 bg-destructive/5 p-4 space-y-2 text-center"
                      >
                        <p className="break-words text-xs text-destructive [overflow-wrap:anywhere]">
                          {formatInvokeError(messageQuery.error)}
                        </p>
                        <Button
                          variant="outline"
                          size="sm"
                          onClick={() => void messageQuery.refetch()}
                        >
                          {t("threads.retry")}
                        </Button>
                      </div>
                    ) : (
                      messageQuery.data && (
                        <>
                          {filteredMessages.length ? (
                            <div className="space-y-3">
                              {filteredMessages.map((message, index) => (
                                <MessageItem
                                  key={`${offset}-${index}`}
                                  message={message}
                                />
                              ))}
                            </div>
                          ) : (
                            <div className="rounded-xl border border-dashed p-8 text-center text-muted-foreground">
                              <MessagesSquare className="mx-auto h-8 w-8 opacity-40" />
                              <p className="mt-2 text-sm">
                                {t("threads.details.noMessages")}
                              </p>
                            </div>
                          )}

                          {/* Pagination Controls */}
                          <div className="flex items-center justify-between gap-2 border-t border-border/60 pt-3">
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={offset === 0}
                              className="rounded-xl px-3.5 text-xs font-semibold"
                              onClick={() => setOffset(Math.max(0, offset - 50))}
                            >
                              {t("threads.details.previous")}
                            </Button>
                            <span className="rounded-full bg-muted px-3 py-1 font-mono text-xs tabular-nums text-muted-foreground font-semibold">
                              {t("threads.details.page", {
                                page: offset / 50 + 1,
                              })}
                            </span>
                            <Button
                              variant="outline"
                              size="sm"
                              disabled={messageQuery.data.data.nextOffset === null}
                              className="rounded-xl px-3.5 text-xs font-semibold"
                              onClick={() =>
                                setOffset(messageQuery.data!.data.nextOffset!)
                              }
                            >
                              {t("threads.details.next")}
                            </Button>
                          </div>
                        </>
                      )
                    )}
                  </div>
                )}
              </>
            )
          )}
        </div>

        {/* Footer Actions */}
        <footer className="flex shrink-0 flex-wrap items-center gap-1.5 border-t border-border/60 bg-muted/20 px-5 py-3 backdrop-blur-md">
          <Button
            variant="outline"
            size="sm"
            className="mr-auto gap-2 rounded-xl text-xs font-semibold shadow-xs"
            disabled={!available}
            onClick={() =>
              setTab(tab === "conversation" ? "usage" : "conversation")
            }
          >
            <MessagesSquare className="h-3.5 w-3.5 text-primary" />
            <span>
              {t(
                tab === "conversation"
                  ? "threads.details.usage"
                  : "threads.details.viewConversation",
              )}
            </span>
          </Button>

          <TooltipProvider delayDuration={200}>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-8 w-8 rounded-xl text-muted-foreground hover:bg-muted hover:text-foreground active:scale-95"
                  disabled={!available || detailQuery.isFetching}
                  aria-label={t("threads.details.refresh")}
                  onClick={() => {
                    setOffset(0);
                    void detailQuery.refetch();
                    if (tab === "conversation" && offset === 0)
                      void messageQuery.refetch();
                  }}
                >
                  <RefreshCw
                    className={cn(
                      "h-3.5 w-3.5",
                      detailQuery.isFetching && "animate-spin",
                    )}
                  />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.details.refresh")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-8 w-8 rounded-xl text-muted-foreground hover:bg-muted hover:text-foreground active:scale-95"
                  disabled={!available}
                  aria-label={t("threads.exportThread")}
                  onClick={onExport}
                >
                  <Download className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.exportThread")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-8 w-8 rounded-xl text-muted-foreground hover:bg-muted hover:text-foreground active:scale-95"
                  aria-label={t("threads.copyId")}
                  onClick={() => void copyId()}
                >
                  <Copy className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.copyId")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-8 w-8 rounded-xl text-muted-foreground hover:bg-muted hover:text-foreground active:scale-95"
                  disabled={!available}
                  aria-label={t("threads.openRawFile")}
                  onClick={onOpenRaw}
                >
                  <FileJson className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.openRawFile")}</TooltipContent>
            </Tooltip>

            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  className="h-8 w-8 rounded-xl text-destructive hover:bg-destructive/10 hover:text-destructive active:scale-95"
                  aria-label={t("threads.deleteThread")}
                  onClick={() => {
                    onClose();
                    onDelete();
                  }}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </TooltipTrigger>
              <TooltipContent>{t("threads.deleteThread")}</TooltipContent>
            </Tooltip>
          </TooltipProvider>
        </footer>
      </SheetContent>
    </Sheet>
  );
}
