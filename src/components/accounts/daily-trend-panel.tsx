import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { BarChart3 } from "lucide-react";

import type { TokenUsagePayload } from "@/types";

type DailyUsage = TokenUsagePayload["local"]["daily"][number];
type ChartPoint = { x: number; y: number };
type ModelUsage = {
  key: string;
  title: string;
  totals: { totalTokens: number };
};
type DonutPart = {
  key: string;
  label: string;
  value: number;
  stroke: string;
  dot: string;
};

const MODEL_COLORS = [
  { stroke: "stroke-primary", dot: "bg-primary" },
  { stroke: "stroke-sky-500", dot: "bg-sky-500" },
  { stroke: "stroke-emerald-500", dot: "bg-emerald-500" },
  { stroke: "stroke-violet-500", dot: "bg-violet-500" },
  { stroke: "stroke-rose-500", dot: "bg-rose-500" },
] as const;

const CHART_WIDTH = 720;
const CHART_HEIGHT = 260;
const CHART_MARGIN = { top: 18, right: 14, bottom: 38, left: 52 };

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

function smoothPath(points: ChartPoint[], minY = Number.NEGATIVE_INFINITY, maxY = Number.POSITIVE_INFINITY) {
  if (points.length === 0) return "";
  if (points.length === 1) return "M " + points[0].x + " " + points[0].y;

  const clampY = (value: number) => Math.min(maxY, Math.max(minY, value));
  let path = "M " + points[0].x + " " + points[0].y;
  for (let index = 0; index < points.length - 1; index += 1) {
    const before = points[index - 1] ?? points[index];
    const current = points[index];
    const next = points[index + 1];
    const after = points[index + 2] ?? next;
    const controlOneX = current.x + (next.x - before.x) / 6;
    const controlOneY = clampY(current.y + (next.y - before.y) / 6);
    const controlTwoX = next.x - (after.x - current.x) / 6;
    const controlTwoY = clampY(next.y - (after.y - current.y) / 6);
    path +=
      " C " +
      controlOneX +
      " " +
      controlOneY +
      ", " +
      controlTwoX +
      " " +
      controlTwoY +
      ", " +
      next.x +
      " " +
      next.y;
  }
  return path;
}

export function DailyTrendPanel({ daily, models }: { daily: DailyUsage[]; models: ModelUsage[] }) {
  const { t } = useTranslation();
  const [activeIndex, setActiveIndex] = useState<number | null>(null);

  const chart = useMemo(() => {
    const plotWidth = CHART_WIDTH - CHART_MARGIN.left - CHART_MARGIN.right;
    const plotHeight = CHART_HEIGHT - CHART_MARGIN.top - CHART_MARGIN.bottom;
    const baseline = CHART_HEIGHT - CHART_MARGIN.bottom;
    const peak = Math.max(0, ...daily.map((day) => day.totals.totalTokens));
    const maximum = Math.max(1, peak * 1.14);
    const xStep = daily.length > 1 ? plotWidth / (daily.length - 1) : 0;
    const xAt = (index: number) => CHART_MARGIN.left + index * xStep;
    const yAt = (value: number) => CHART_MARGIN.top + plotHeight * (1 - value / maximum);
    const makePoints = (valueAt: (day: DailyUsage) => number) =>
      daily.map((day, index) => ({ x: xAt(index), y: yAt(valueAt(day)) }));

    const points = {
      total: makePoints((day) => day.totals.totalTokens),
      input: makePoints((day) => day.totals.inputTokens),
      output: makePoints((day) => day.totals.outputTokens + day.totals.reasoningTokens),
      cache: makePoints((day) => day.totals.cacheReadTokens + day.totals.cacheWriteTokens),
    };
    const totalPath = smoothPath(points.total, CHART_MARGIN.top + 2, baseline);
    const areaPath = points.total.length
      ? totalPath +
        " L " +
        points.total[points.total.length - 1].x +
        " " +
        baseline +
        " L " +
        points.total[0].x +
        " " +
        baseline +
        " Z"
      : "";

    return { maximum, peak, baseline, plotHeight, xAt, points, totalPath, areaPath };
  }, [daily]);

  const totals = useMemo(
    () =>
      daily.reduce(
        (sum, day) => ({
          total: sum.total + day.totals.totalTokens,
          input: sum.input + day.totals.inputTokens,
          output: sum.output + day.totals.outputTokens + day.totals.reasoningTokens,
          cache: sum.cache + day.totals.cacheReadTokens + day.totals.cacheWriteTokens,
        }),
        { total: 0, input: 0, output: 0, cache: 0 },
      ),
    [daily],
  );

  if (daily.length === 0) return null;

  const dailyAverage = totals.total / daily.length;
  const activeDay = activeIndex == null ? null : daily[activeIndex];
  const activePoint = activeIndex == null ? null : chart.points.total[activeIndex];
  const tooltipWidth = 178;
  const tooltipHeight = 92;
  const tooltipX = activePoint
    ? Math.min(
        Math.max(activePoint.x - tooltipWidth / 2, CHART_MARGIN.left),
        CHART_WIDTH - CHART_MARGIN.right - tooltipWidth,
      )
    : 0;
  const tooltipY = activePoint
    ? Math.max(4, Math.min(activePoint.y - tooltipHeight - 12, chart.baseline - tooltipHeight - 8))
    : 0;

  const groupedModels = new Map<string, { key: string; label: string; value: number }>();
  for (const model of models) {
    const label = model.title.split("::").pop() || model.title;
    const groupKey = label.trim().toLowerCase();
    const current = groupedModels.get(groupKey);
    if (current) {
      current.value += model.totals.totalTokens;
      current.key += "+" + model.key;
    } else {
      groupedModels.set(groupKey, { key: model.key, label, value: model.totals.totalTokens });
    }
  }
  const sortedModels = Array.from(groupedModels.values()).sort((left, right) => right.value - left.value);
  const modelTotal = sortedModels.reduce((sum, model) => sum + model.value, 0);
  const visibleModels = sortedModels.slice(0, MODEL_COLORS.length);
  const otherTokens = sortedModels.slice(MODEL_COLORS.length).reduce((sum, model) => sum + model.value, 0);
  const circumference = Math.PI * 2 * 46;
  const donutParts: DonutPart[] = visibleModels.map((model, index) => ({
    ...model,
    ...MODEL_COLORS[index],
  }));
  if (otherTokens > 0) {
    donutParts.push({
      key: "other-models",
      label: t("accounts.otherModels"),
      value: otherTokens,
      stroke: "stroke-slate-400",
      dot: "bg-slate-400",
    });
  }
  let donutOffset = 0;

  return (
    <section className="rounded-2xl border border-border/80 bg-card/85 p-4 shadow-xs backdrop-blur-md sm:p-5">
      <div className="flex flex-col gap-3 border-b border-border/50 pb-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="flex items-center gap-2">
          <span className="rounded-lg bg-primary/10 p-2 text-primary">
            <BarChart3 className="h-4 w-4" />
          </span>
          <div>
            <h4 className="text-xs font-semibold uppercase tracking-[0.12em] text-foreground/90">
              {t("accounts.dailyTrend")}
            </h4>
            <p className="mt-0.5 text-[10px] text-muted-foreground">{t("accounts.dailyAggregate")}</p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2 text-[10px]">
          <span className="rounded-md bg-muted/70 px-2 py-1 text-muted-foreground">
            {t("accounts.dailyAverage")}{" "}
            <strong className="ml-1 font-mono text-foreground">{formatTokens(dailyAverage)}</strong>
          </span>
          <span className="rounded-md bg-primary/10 px-2 py-1 text-primary">
            {t("accounts.peakUsage")}{" "}
            <strong className="ml-1 font-mono">{formatTokens(chart.peak)}</strong>
          </span>
        </div>
      </div>

      <div className="mt-4 grid gap-4 lg:grid-cols-[minmax(0,1fr)_260px]">
        <div className="min-w-0 rounded-xl border border-border/60 bg-background/45 p-2.5 sm:p-3">
          <svg
            viewBox={'0 0 ' + CHART_WIDTH + ' ' + CHART_HEIGHT}
            className="block h-auto w-full overflow-visible"
            role="img"
            aria-label={t("accounts.dailyTrendChartLabel")}
            onMouseLeave={() => setActiveIndex(null)}
          >
            <defs>
              <linearGradient id="daily-total-fill" x1="0" y1="0" x2="0" y2="1">
                <stop offset="0%" stopColor="currentColor" stopOpacity="0.2" className="text-primary" />
                <stop offset="100%" stopColor="currentColor" stopOpacity="0.01" className="text-primary" />
              </linearGradient>
              <clipPath id="daily-trend-clip">
                <rect
                  x={CHART_MARGIN.left - 4}
                  y={CHART_MARGIN.top - 4}
                  width={CHART_WIDTH - CHART_MARGIN.left - CHART_MARGIN.right + 8}
                  height={chart.plotHeight + 8}
                />
              </clipPath>
            </defs>

            {[1, 0.66, 0.33, 0].map((ratio) => {
              const y = CHART_MARGIN.top + chart.plotHeight * (1 - ratio);
              return (
                <g key={ratio}>
                  <line
                    x1={CHART_MARGIN.left}
                    y1={y}
                    x2={CHART_WIDTH - CHART_MARGIN.right}
                    y2={y}
                    className="stroke-border/70"
                    strokeWidth="1"
                    strokeDasharray={ratio === 0 ? undefined : "3 5"}
                  />
                  <text
                    x={CHART_MARGIN.left - 9}
                    y={y + 3}
                    textAnchor="end"
                    className="fill-muted-foreground font-mono text-[9px]"
                  >
                    {formatTokens(chart.maximum * ratio)}
                  </text>
                </g>
              );
            })}

            <g clipPath="url(#daily-trend-clip)">
              {chart.areaPath && <path d={chart.areaPath} fill="url(#daily-total-fill)" />}
              <path d={chart.totalPath} fill="none" stroke="currentColor" strokeWidth="2.5" className="text-primary" />
              <path d={smoothPath(chart.points.input, CHART_MARGIN.top + 2, chart.baseline)} fill="none" stroke="currentColor" strokeWidth="1.8" className="text-sky-500" />
              <path d={smoothPath(chart.points.output, CHART_MARGIN.top + 2, chart.baseline)} fill="none" stroke="currentColor" strokeWidth="1.8" className="text-amber-500" />
              <path d={smoothPath(chart.points.cache, CHART_MARGIN.top + 2, chart.baseline)} fill="none" stroke="currentColor" strokeWidth="1.8" className="text-emerald-500" />
            </g>

            {daily.map((day, index) => {
              const x = chart.xAt(index);
              const showLabel = index % 2 === 0 || index === daily.length - 1;
              const isToday = index === daily.length - 1;
              const totalPoint = chart.points.total[index];
              const label =
                day.date +
                ", " +
                t("accounts.totalShort") +
                " " +
                formatTokens(day.totals.totalTokens) +
                ", " +
                t("accounts.agentTurns", { count: day.totals.turnCount });
              return (
                <g key={day.date}>
                  {showLabel && (
                    <text
                      x={x}
                      y={CHART_HEIGHT - 11}
                      textAnchor="middle"
                      className={
                        isToday
                          ? "fill-primary font-mono text-[9px] font-semibold"
                          : "fill-muted-foreground font-mono text-[9px]"
                      }
                    >
                      {isToday ? t("accounts.todayShort") : day.date.slice(5)}
                    </text>
                  )}
                  {isToday && (
                    <circle
                      cx={totalPoint.x}
                      cy={totalPoint.y}
                      r="4"
                      className="fill-primary stroke-background"
                      strokeWidth="2"
                    />
                  )}
                  <circle
                    cx={totalPoint.x}
                    cy={totalPoint.y}
                    r="12"
                    fill="transparent"
                    className="cursor-pointer outline-none focus-visible:stroke-primary"
                    tabIndex={0}
                    aria-label={label}
                    onMouseEnter={() => setActiveIndex(index)}
                    onFocus={() => setActiveIndex(index)}
                    onBlur={() => setActiveIndex(null)}
                    onClick={() => setActiveIndex(index)}
                  />
                </g>
              );
            })}

            {activeDay && activePoint && (
              <g pointerEvents="none" aria-hidden="true">
                <line
                  x1={activePoint.x}
                  y1={CHART_MARGIN.top}
                  x2={activePoint.x}
                  y2={chart.baseline}
                  className="stroke-primary/40"
                  strokeWidth="1"
                  strokeDasharray="3 4"
                />
                <circle
                  cx={activePoint.x}
                  cy={activePoint.y}
                  r="4.5"
                  className="fill-primary stroke-background"
                  strokeWidth="2"
                />
                <rect
                  x={tooltipX}
                  y={tooltipY}
                  width={tooltipWidth}
                  height={tooltipHeight}
                  rx="8"
                  className="fill-popover stroke-border"
                  strokeWidth="1"
                />
                <text x={tooltipX + 11} y={tooltipY + 17} className="fill-popover-foreground text-[10px] font-semibold">
                  {activeDay.date}
                </text>
                <text
                  x={tooltipX + tooltipWidth - 11}
                  y={tooltipY + 17}
                  textAnchor="end"
                  className="fill-primary font-mono text-[10px] font-semibold"
                >
                  {formatTokens(activeDay.totals.totalTokens)}
                </text>
                <line
                  x1={tooltipX + 10}
                  y1={tooltipY + 25}
                  x2={tooltipX + tooltipWidth - 10}
                  y2={tooltipY + 25}
                  className="stroke-border/70"
                />
                <text x={tooltipX + 11} y={tooltipY + 42} className="fill-sky-500 text-[9px]">
                  {t("accounts.inputShort") + "  " + formatTokens(activeDay.totals.inputTokens)}
                </text>
                <text x={tooltipX + 11} y={tooltipY + 58} className="fill-amber-500 text-[9px]">
                  {t("accounts.outputShort") +
                    "  " +
                    formatTokens(activeDay.totals.outputTokens + activeDay.totals.reasoningTokens)}
                </text>
                <text x={tooltipX + 11} y={tooltipY + 74} className="fill-emerald-500 text-[9px]">
                  {t("accounts.cacheShort") +
                    "  " +
                    formatTokens(activeDay.totals.cacheReadTokens + activeDay.totals.cacheWriteTokens)}
                </text>
              </g>
            )}
          </svg>

          <div className="mt-1 flex flex-wrap items-center justify-center gap-x-4 gap-y-1 border-t border-border/50 pt-2 text-[10px] text-muted-foreground">
            <span className="flex items-center gap-1.5"><i className="h-2 w-2 rounded-full bg-primary" />{t("accounts.totalShort")}</span>
            <span className="flex items-center gap-1.5"><i className="h-2 w-2 rounded-full bg-sky-500" />{t("accounts.inputShort")}</span>
            <span className="flex items-center gap-1.5"><i className="h-2 w-2 rounded-full bg-amber-500" />{t("accounts.outputShort")}</span>
            <span className="flex items-center gap-1.5"><i className="h-2 w-2 rounded-full bg-emerald-500" />{t("accounts.cacheShort")}</span>
          </div>
        </div>

        <aside className="rounded-xl border border-border/60 bg-background/45 p-4">
          <h5 className="text-center text-[10px] font-semibold uppercase tracking-[0.12em] text-muted-foreground">
            {t("accounts.modelComposition")}
          </h5>
          <div className="relative mx-auto mt-3 h-36 w-36">
            <svg viewBox="0 0 120 120" className="h-full w-full" aria-hidden="true">
              <circle cx="60" cy="60" r="46" fill="none" strokeWidth="12" className="stroke-muted/70" />
              {donutParts.map((part) => {
                const segmentLength = (part.value / Math.max(1, modelTotal)) * circumference;
                const offset = donutOffset;
                donutOffset += segmentLength;
                return (
                  <circle
                    key={part.key}
                    cx="60"
                    cy="60"
                    r="46"
                    fill="none"
                    strokeWidth="12"
                    strokeDasharray={segmentLength + " " + circumference}
                    strokeDashoffset={-offset}
                    transform="rotate(-90 60 60)"
                    className={part.stroke}
                  />
                );
              })}
            </svg>
            <div className="absolute inset-0 flex flex-col items-center justify-center text-center">
              <span className="text-[10px] text-muted-foreground">{t("accounts.totalShort")}</span>
              <strong className="mt-0.5 font-mono text-lg tracking-tight">{formatTokens(modelTotal)}</strong>
              <span className="mt-0.5 text-[9px] text-muted-foreground">{t("accounts.allUsage")}</span>
            </div>
          </div>
          {donutParts.length === 0 ? (
            <p className="mt-4 text-center text-[10px] text-muted-foreground">{t("accounts.localUsageEmpty")}</p>
          ) : (
            <div className="mt-3 space-y-2">
              {donutParts.map((part) => (
                <div key={part.key} className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-2 text-[10px]">
                  <i className={'h-2 w-2 rounded-full ' + part.dot} />
                  <span className="truncate text-muted-foreground" title={part.label}>{part.label}</span>
                  <span className="font-mono text-foreground">
                    {formatTokens(part.value)} · {modelTotal > 0 ? Math.round((part.value / modelTotal) * 100) : 0}%
                  </span>
                </div>
              ))}
            </div>
          )}
        </aside>
      </div>
    </section>
  );
}
