import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { Copy, Download, FileJson, Loader2, MessagesSquare, RefreshCw, Trash2 } from "lucide-react";
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { api } from "@/lib/api";
import { queryKeys } from "@/lib/query-keys";
import { formatInvokeError } from "@/lib/invoke-error";
import { formatDateTime } from "@/lib/format-time";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { formatSize } from "./thread-utils";
import type { ThreadDetails, ThreadDiagnostics, ThreadMessagePage, ThreadSummary, ThreadUsage } from "@/types/threads";

const tabs = ["usage", "tools", "metadata", "conversation"] as const;
type Tab = typeof tabs[number];
const number = (value: number) => value.toLocaleString();

function Diagnostics({ data }: { data: ThreadDiagnostics }) {
  const { t } = useTranslation();
  const entries = Object.entries(data).filter(([, count]) => count > 0);
  if (!entries.length) return null;
  return <details className="rounded-lg border border-amber-500/25 bg-amber-500/5 px-3 py-2 text-xs">
    <summary className="cursor-pointer font-medium">{t("threads.details.diagnostics")}</summary>
    <ul className="mt-2 space-y-1 text-muted-foreground">
      {entries.map(([key, count]) => <li key={key}>{t(`threads.details.diagnostic.${key}`, { count })}</li>)}
    </ul>
  </details>;
}

function UsageBreakdown({ usage }: { usage: ThreadUsage }) {
  const { t } = useTranslation();
  const parts = [
    { key: "input", value: usage.cached === null ? null : Math.max(0, usage.input - usage.cached), color: "bg-sky-500" },
    { key: "cached", value: usage.cached, color: "bg-emerald-500" },
    { key: "output", value: usage.reasoning === null ? null : Math.max(0, usage.output - usage.reasoning), color: "bg-amber-500" },
    { key: "reasoning", value: usage.reasoning, color: "bg-violet-500" },
  ];
  const complete = parts.every(part => part.value !== null) && usage.cached! <= usage.input && usage.reasoning! <= usage.output && usage.total === usage.input + usage.output;
  return <section className="space-y-4">
    <div>
      <p className="text-xs text-muted-foreground">{t("threads.details.recordedTokens")}</p>
      <p className="mt-1 text-3xl font-semibold tracking-tight tabular-nums">{number(usage.total)} <span className="text-sm font-normal text-muted-foreground">Token</span></p>
    </div>
    {complete && usage.total > 0 && <div aria-hidden="true" className="flex h-2 overflow-hidden rounded-full bg-muted">
      {parts.map(part => <div key={part.key} className={part.color} style={{ width: `${part.value! / usage.total * 100}%` }} />)}
    </div>}
    <dl className="space-y-2.5 text-sm">
      {parts.map(part => <div key={part.key} className="flex items-center justify-between gap-3">
        <dt className="flex items-center gap-2 text-muted-foreground"><span className={cn("h-2 w-2 rounded-full", part.color)} />{t(`threads.details.${part.key}`)}</dt>
        <dd className="shrink-0 tabular-nums">{part.value === null ? t("threads.details.unknown") : number(part.value)}
          {complete && usage.total > 0 && <span className="ml-3 inline-block w-12 text-right text-xs text-muted-foreground">{(part.value! / usage.total * 100).toFixed(1)}%</span>}
        </dd>
      </div>)}
    </dl>
    {!complete && <p className="text-xs text-muted-foreground">{t("threads.details.incompleteBreakdown", { input: number(usage.input), output: number(usage.output) })}</p>}
  </section>;
}

function UsageHistory({ detail }: { detail: ThreadDetails }) {
  const { t } = useTranslation();
  let cumulative = 0;
  const dates = detail.byDate.filter(item => item.label !== "unknown").map(item => ({ ...item, cumulative: cumulative += item.usage.total }));
  const recent = dates.slice(-30);
  const max = Math.max(1, cumulative);
  const point = (value: number, index: number) => [recent.length === 1 ? 176 : 44 + index / (recent.length - 1) * 264, 108 - value / max * 92];
  return <>
    <section className="space-y-3 border-t pt-5">
      <div className="flex items-center justify-between gap-2"><h3 className="text-sm font-medium">{t("threads.details.cumulative")}</h3><span className="text-xs text-muted-foreground">{t("threads.details.dateCount", { count: dates.length })}</span></div>
      {recent.length ? <>
        <svg viewBox="0 0 320 120" className="w-full text-primary" role="img" aria-label={t("threads.details.cumulativeChart")}>
          {[16, 62, 108].map((y, index) => <g key={y}><line x1="44" x2="308" y1={y} y2={y} className="stroke-border" /><text x="38" y={y + 3} textAnchor="end" className="fill-muted-foreground text-[9px]">{new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 }).format(max * (1 - index / 2))}</text></g>)}
          {recent.length > 1 && <polyline points={recent.map((item, index) => point(item.cumulative, index).join(",")).join(" ")} fill="none" stroke="currentColor" strokeWidth="2" />}
          {[recent[0], ...(recent.length > 1 ? [recent[recent.length - 1]] : [])].map((item, index) => {
            const [x,y] = point(item.cumulative, index ? recent.length - 1 : 0);
            return <circle key={item.label} cx={x} cy={y} r="3" fill="currentColor" />;
          })}
        </svg>
        <div className="flex justify-between text-xs text-muted-foreground"><span>{recent[0].label}</span><span>{number(cumulative)} Token · {recent[recent.length - 1].label}</span></div>
        <details className="text-xs"><summary className="cursor-pointer text-muted-foreground">{t("threads.details.dailyRecords")}</summary>
          <div className="mt-2 max-h-48 overflow-auto"><table className="w-full text-left tabular-nums">
            <thead className="text-muted-foreground"><tr><th className="py-1 font-normal">{t("threads.details.date")}</th><th className="text-right font-normal">Token</th><th className="text-right font-normal">{t("threads.details.cumulative")}</th></tr></thead>
            <tbody>{dates.map(item => <tr key={item.label}><td className="py-1">{item.label}</td><td className="text-right">{number(item.usage.total)}</td><td className="text-right">{number(item.cumulative)}</td></tr>)}</tbody>
          </table></div>
        </details>
      </> : <p className="text-xs text-muted-foreground">{t("threads.details.noDates")}</p>}
    </section>
    <section className="space-y-3 border-t pt-5">
      <h3 className="text-sm font-medium">{t("threads.details.models")}</h3>
      <p className="text-xs leading-relaxed text-muted-foreground">{t("threads.details.attributionNote")}</p>
      <dl className="divide-y text-sm">{detail.byModel.map(item => <div key={item.label} className="flex items-start justify-between gap-3 py-2.5">
        <dt className="min-w-0 break-words [overflow-wrap:anywhere]">{item.label === "unknown" ? t("threads.details.unknown") : item.label}</dt><dd className="shrink-0 tabular-nums">{number(item.usage.total)}</dd>
      </div>)}</dl>
    </section>
  </>;
}

function MessageItem({ message }: { message: ThreadMessagePage["messages"][number] }) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const long = message.text.length > 1500;
  const copy = async () => {
    try { await navigator.clipboard.writeText(message.text); toast({ title: t("threads.details.copied") }); }
    catch (error) { toast({ title: t("threads.details.copyFailed"), description: formatInvokeError(error), variant: "destructive" }); }
  };
  return <article className={cn("min-w-0 rounded-lg border p-3", message.role === "user" ? "border-primary/25 bg-primary/5" : "bg-muted/20")}>
    <div className="mb-2 flex items-center justify-between gap-2 text-xs">
      <span className="font-medium">{t(`threads.details.roles.${message.role}`, { defaultValue: message.role })}</span>
      <div className="flex min-w-0 items-center gap-2"><time className="truncate text-muted-foreground">{message.timestamp}</time>
        <Button variant="ghost" size="icon-sm" className="h-7 w-7 shrink-0" aria-label={t("threads.details.copyMessage")} onClick={() => void copy()}><Copy className="h-3.5 w-3.5" /></Button></div>
    </div>
    <p className="whitespace-pre-wrap break-words text-sm leading-relaxed [overflow-wrap:anywhere]">{long && !expanded ? `${message.text.slice(0, 1500)}…` : message.text}</p>
    {long && <Button variant="ghost" size="sm" className="mt-2 h-7 px-1 text-xs" aria-expanded={expanded} onClick={() => setExpanded(!expanded)}>{t(expanded ? "threads.details.collapse" : "threads.details.expand")}</Button>}
    {message.truncated && <p className="mt-2 text-xs text-muted-foreground">{t("threads.details.truncated")}</p>}
  </article>;
}

export function ThreadDetailSheet({ thread, onClose, onExport, onDelete, onOpenRaw }: {
  thread: ThreadSummary; onClose: () => void; onExport: () => void; onDelete: () => void; onOpenRaw: () => void;
}) {
  const { t } = useTranslation();
  const opener = useRef(document.activeElement instanceof HTMLElement ? document.activeElement : null);
  const tabList = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const [tab, setTab] = useState<Tab>("usage");
  const [offset, setOffset] = useState(0);
  const available = thread.rolloutStatus !== "missing";
  useEffect(() => { content.current?.scrollTo(0, 0); }, [tab, offset]);
  const detailQuery = useQuery({ queryKey: queryKeys.threadDetails(thread.key), queryFn: () => api.conversations.details("codex", thread.key, thread.id), enabled: available, gcTime: 0, retry: false, refetchOnWindowFocus: false });
  const detail = detailQuery.data?.data;
  const messageQuery = useQuery({ queryKey: queryKeys.threadMessages(thread.key, detail?.snapshotBytes ?? 0, offset),
    queryFn: () => api.conversations.messages("codex", thread.key, thread.id, detail!.snapshotBytes, offset), enabled: tab === "conversation" && !!detail, gcTime: 0, retry: false, refetchOnWindowFocus: false });
  const copyId = async () => {
    try { await navigator.clipboard.writeText(thread.id); toast({ title: t("threads.copiedId") }); }
    catch (error) { toast({ title: t("threads.details.copyFailed"), description: formatInvokeError(error), variant: "destructive" }); }
  };
  const metadata = [
    ["id", thread.id], ["cwd", thread.cwd], ["file", thread.filePath], ["provider", thread.modelProvider], ["model", thread.model],
    ["created", thread.createdAt ? formatDateTime(thread.createdAt) : ""], ["updated", thread.updatedAt ? formatDateTime(thread.updatedAt) : ""], ["version", thread.cliVersion],
    ["source", `${thread.threadSource} · ${thread.source}`], ["size", formatSize(thread.fileSize)],
  ];
  return <Sheet open onOpenChange={open => !open && onClose()}>
    <SheetContent closeLabel={t("common.close")} className="flex h-[100dvh] w-full flex-col gap-0 p-0 sm:w-[480px] sm:max-w-[480px]" onCloseAutoFocus={event => { event.preventDefault(); if (opener.current?.isConnected) opener.current.focus(); }}>
      <SheetHeader className="shrink-0 space-y-2 border-b px-5 pb-4 pt-5 text-left">
        <SheetTitle className="pr-8 text-base leading-snug [overflow-wrap:anywhere]">{thread.title || t("threads.untitled")}</SheetTitle>
        <SheetDescription>{t("threads.details.readOnly")}</SheetDescription>
        <div className="flex flex-wrap gap-2 text-xs text-muted-foreground"><span>{t(thread.archived ? "threads.archivedBadge" : "threads.activeBadge")}</span><span>· {t("threads.details.messages", { count: thread.userMessages + thread.agentMessages })}</span>
          {detail && <span>· {t("threads.details.modelCount", { count: detail.byModel.length })}</span>}
        </div>
      </SheetHeader>
      <div ref={tabList} role="tablist" aria-label={t("threads.details.tabs")} className="flex shrink-0 border-b px-3" onKeyDown={event => {
        const index = tabs.indexOf(tab);
        const next = event.key === "ArrowRight" ? (index + 1) % tabs.length : event.key === "ArrowLeft" ? (index + tabs.length - 1) % tabs.length : event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : null;
        if (next === null) return; event.preventDefault(); setTab(tabs[next]); tabList.current?.querySelectorAll<HTMLButtonElement>("button")[next]?.focus();
      }}>
        {tabs.map(name => <button key={name} type="button" role="tab" id={`thread-tab-${name}`} aria-controls="thread-detail-content" aria-selected={tab === name} tabIndex={tab === name ? 0 : -1}
          className={cn("border-b-2 px-3 py-3 text-sm transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring", tab === name ? "border-primary font-medium text-foreground" : "border-transparent text-muted-foreground hover:text-foreground")}
          onClick={() => setTab(name)}>{t(`threads.details.${name}`)}</button>)}
      </div>
      <div ref={content} role="tabpanel" id="thread-detail-content" aria-labelledby={`thread-tab-${tab}`} tabIndex={0} className="min-h-0 flex-1 space-y-5 overflow-y-auto overscroll-contain p-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring">
        {tab === "metadata" ? <dl className="space-y-4 text-sm">{metadata.map(([key, value]) => <div key={key}><dt className="mb-1 text-xs text-muted-foreground">{t(`threads.details.meta.${key}`)}</dt><dd className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">{value || t("threads.details.unknown")}</dd></div>)}</dl>
          : !available ? <p className="text-sm text-muted-foreground">{t("threads.details.noRollout")}</p>
          : detailQuery.isPending ? <p role="status" className="flex items-center gap-2 text-sm text-muted-foreground"><Loader2 className="h-4 w-4 animate-spin" />{t("threads.details.loading")}</p>
          : detailQuery.isError ? <div role="alert" className="space-y-3"><p className="break-words text-sm [overflow-wrap:anywhere]">{formatInvokeError(detailQuery.error)}</p><Button variant="outline" size="sm" onClick={() => void detailQuery.refetch()}>{t("threads.retry")}</Button></div>
          : detail && <>
            <Diagnostics data={detail.diagnostics} />
            {tab === "usage" && (detail.usage ? <><UsageBreakdown usage={detail.usage} /><UsageHistory detail={detail} /></> : <p className="text-sm text-muted-foreground">{t("threads.details.noUsage")}</p>)}
            {tab === "tools" && (detail.tools.length ? <dl className="divide-y text-sm">{detail.tools.map(tool => <div key={tool.name} className="flex items-center justify-between gap-3 py-3"><dt className="min-w-0 break-words font-mono [overflow-wrap:anywhere]">{tool.name}</dt><dd className="shrink-0 tabular-nums">{t("threads.details.calls", { count: tool.calls })}</dd></div>)}</dl> : <p className="text-sm text-muted-foreground">{t("threads.details.noTools")}</p>)}
            {tab === "conversation" && <>
              <p className="text-xs text-muted-foreground">{t("threads.details.conversationNote")}</p>
              {messageQuery.isPending ? <p role="status" className="text-sm">{t("threads.details.loading")}</p>
                : messageQuery.isError ? <div role="alert" className="space-y-2"><p className="break-words text-sm [overflow-wrap:anywhere]">{formatInvokeError(messageQuery.error)}</p><Button variant="outline" size="sm" onClick={() => void messageQuery.refetch()}>{t("threads.retry")}</Button></div>
                : messageQuery.data && <>
                  {messageQuery.data.data.messages.length ? messageQuery.data.data.messages.map((message, index) => <MessageItem key={`${offset}-${index}`} message={message} />) : <p className="text-sm text-muted-foreground">{t("threads.details.noMessages")}</p>}
                  <div className="flex items-center justify-between gap-2 border-t pt-3">
                    <Button variant="outline" size="sm" disabled={offset === 0} onClick={() => setOffset(Math.max(0, offset - 50))}>{t("threads.details.previous")}</Button>
                    <span className="text-xs tabular-nums text-muted-foreground">{t("threads.details.page", { page: offset / 50 + 1 })}</span>
                    <Button variant="outline" size="sm" disabled={messageQuery.data.data.nextOffset === null} onClick={() => setOffset(messageQuery.data!.data.nextOffset!)}>{t("threads.details.next")}</Button>
                  </div>
                </>}
            </>}
          </>}
      </div>
      <footer className="flex shrink-0 flex-wrap items-center gap-1 border-t bg-background px-4 py-3">
        <Button variant="outline" size="sm" className="mr-auto gap-2" disabled={!available} onClick={() => setTab(tab === "conversation" ? "usage" : "conversation")}><MessagesSquare className="h-4 w-4" />{t(tab === "conversation" ? "threads.details.usage" : "threads.details.viewConversation")}</Button>
        <Button variant="ghost" size="icon-sm" disabled={!available || detailQuery.isFetching} aria-label={t("threads.details.refresh")} onClick={() => { setOffset(0); void detailQuery.refetch(); if (tab === "conversation" && offset === 0) void messageQuery.refetch(); }}><RefreshCw className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!available} aria-label={t("threads.exportThread")} onClick={onExport}><Download className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon-sm" aria-label={t("threads.copyId")} onClick={() => void copyId()}><Copy className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon-sm" disabled={!available} aria-label={t("threads.openRawFile")} onClick={onOpenRaw}><FileJson className="h-4 w-4" /></Button>
        <Button variant="ghost" size="icon-sm" className="text-destructive hover:text-destructive" aria-label={t("threads.deleteThread")} onClick={() => { onClose(); onDelete(); }}><Trash2 className="h-4 w-4" /></Button>
      </footer>
    </SheetContent>
  </Sheet>;
}
