import { useMemo, useState, type CSSProperties, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { BarChart3 } from "lucide-react";

import type { TokenUsagePayload } from "@/types";

type DailyUsage = TokenUsagePayload["local"]["daily"][number];
type Point = { x: number; y: number };

const WIDTH = 1000;
const HEIGHT = 320;
const LEFT = 64;
const RIGHT = 24;
const TOP = 25;
const BASELINE = 275;
const MODEL_COLORS = ["#ed7c35", "#54a7d9", "#54b98b", "#a67be7", "#e66880", "#e6ad4f"];

function formatTokens(value: number) {
  return new Intl.NumberFormat(undefined, {
    notation: value >= 10_000 ? "compact" : "standard",
    maximumFractionDigits: 1,
  }).format(value);
}

function smoothPath(points: Point[]) {
  if (!points.length) return "";
  if (points.length === 1) return `M ${points[0].x} ${points[0].y}`;
  let path = `M ${points[0].x} ${points[0].y}`;
  for (let index = 0; index < points.length - 1; index += 1) {
    const current = points[index];
    const next = points[index + 1];
    const mid = (current.x + next.x) / 2;
    path += ` C ${mid} ${current.y}, ${mid} ${next.y}, ${next.x} ${next.y}`;
  }
  return path;
}

function MiniTrack({ values, activeIndex, label, color }: {
  values: number[];
  activeIndex: number;
  label: string;
  color: string;
}) {
  const max = Math.max(1, ...values);
  const points = values.map((value, index) => ({
    x: 4 + index * (292 / Math.max(1, values.length - 1)),
    y: 53 - value / max * 44,
  }));
  const active = points[activeIndex];
  return (
    <div className="usage-mini-track" style={{ "--track-color": color } as CSSProperties}>
      <div className="usage-mini-track-head">
        <span>{label}</span>
        <strong>{formatTokens(values[activeIndex] ?? 0)}</strong>
      </div>
      <svg viewBox="0 0 300 60" preserveAspectRatio="none" aria-hidden="true">
        <line x1="4" y1="53" x2="296" y2="53" stroke="currentColor" className="text-border/80" />
        <path d={smoothPath(points)} fill="none" stroke={color} strokeWidth="2.3" strokeLinecap="round" />
        {active && <circle cx={active.x} cy={active.y} r="4.2" fill={color} stroke="hsl(var(--card))" strokeWidth="2" />}
      </svg>
    </div>
  );
}

export function DailyTrendPanel({ daily }: { daily: DailyUsage[] }) {
  const { t } = useTranslation();
  const [activeIndex, setActiveIndex] = useState<number | null>(null);
  const selectedIndex = activeIndex ?? Math.max(0, daily.length - 1);
  const chart = useMemo(() => {
    const peak = Math.max(1, ...daily.map(day => day.totals.totalTokens));
    const maximum = peak * 1.12;
    const points = daily.map((day, index) => ({
      x: LEFT + index * ((WIDTH - LEFT - RIGHT) / Math.max(1, daily.length - 1)),
      y: BASELINE - day.totals.totalTokens / maximum * (BASELINE - TOP),
    }));
    const line = smoothPath(points);
    const area = points.length ? `${line} L ${points[points.length - 1].x} ${BASELINE} L ${points[0].x} ${BASELINE} Z` : "";
    return { maximum, points, line, area };
  }, [daily]);

  if (!daily.length) return null;
  const day = daily[selectedIndex];
  const activePoint = chart.points[selectedIndex];
  const groupedModels = new Map<string, number>();
  for (const model of day.byModel ?? []) {
    const name = model.model.split("::").pop() || model.model;
    groupedModels.set(name, (groupedModels.get(name) ?? 0) + model.totals.totalTokens);
  }
  const models = [...groupedModels.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6);
  const selectFromPointer = (event: MouseEvent<SVGSVGElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const x = (event.clientX - rect.left) / rect.width * WIDTH;
    const index = Math.round((x - LEFT) / (WIDTH - LEFT - RIGHT) * Math.max(1, daily.length - 1));
    setActiveIndex(Math.min(daily.length - 1, Math.max(0, index)));
  };

  return (
    <section className="usage-trend-card" aria-label={t("accounts.dailyTrend")}>
      <div className="usage-panel-heading">
        <div className="flex items-center gap-3">
          <span className="usage-heading-icon"><BarChart3 className="h-4 w-4" /></span>
          <div>
            <h4>{t("accounts.dailyTrend")}</h4>
            <p>{t("accounts.dailyAggregate")}</p>
          </div>
        </div>
        <div className="usage-trend-selection" aria-live="polite">
          <span>{day.date}</span>
          <strong>{formatTokens(day.totals.totalTokens)} Token</strong>
        </div>
      </div>

      <div className="usage-chart-surface">
        <svg
          viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
          className="usage-trend-svg"
          role="img"
          aria-label={t("accounts.dailyTrendChartLabel")}
          onMouseMove={selectFromPointer}
          onMouseLeave={() => setActiveIndex(null)}
        >
          <defs>
            <linearGradient id="usage-total-area" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="hsl(var(--primary))" stopOpacity="0.22" />
              <stop offset="100%" stopColor="hsl(var(--primary))" stopOpacity="0" />
            </linearGradient>
          </defs>
          {[0, 0.25, 0.5, 0.75, 1].map(ratio => {
            const y = BASELINE - ratio * (BASELINE - TOP);
            return <g key={ratio}>
              <line x1={LEFT} x2={WIDTH - RIGHT} y1={y} y2={y} className="usage-chart-gridline" />
              <text x={LEFT - 12} y={y + 4} textAnchor="end" className="usage-chart-axis">{formatTokens(chart.maximum * ratio)}</text>
            </g>;
          })}
          <path d={chart.area} fill="url(#usage-total-area)" className="usage-area-enter" />
          <path d={chart.line} className="usage-trend-line" />
          <line x1={activePoint.x} x2={activePoint.x} y1={TOP} y2={BASELINE} className="usage-chart-crosshair" />
          <circle cx={activePoint.x} cy={activePoint.y} r="7" className="usage-chart-active-point" />
          {daily.map((item, index) => (
            <g key={item.date}>
              {(index % 2 === 0 || index === daily.length - 1) &&
                <text x={chart.points[index].x} y={HEIGHT - 13} textAnchor="middle" className="usage-chart-axis">
                  {index === daily.length - 1 ? t("accounts.todayShort") : item.date.slice(5)}
                </text>}
              <circle
                cx={chart.points[index].x} cy={chart.points[index].y} r="13" fill="transparent"
                tabIndex={0} aria-label={`${item.date} ${formatTokens(item.totals.totalTokens)} Token`}
                onFocus={() => setActiveIndex(index)} onBlur={() => setActiveIndex(null)}
                onClick={() => setActiveIndex(index)}
              />
            </g>
          ))}
        </svg>
      </div>

      <div className="usage-mini-tracks">
        <MiniTrack values={daily.map(item => item.totals.inputTokens)} activeIndex={selectedIndex} label={t("accounts.inputShort")} color="#54a7d9" />
        <MiniTrack values={daily.map(item => item.totals.outputTokens + item.totals.reasoningTokens)} activeIndex={selectedIndex} label={t("accounts.outputShort")} color="#ed7c35" />
        <MiniTrack values={daily.map(item => item.totals.cacheReadTokens + item.totals.cacheWriteTokens)} activeIndex={selectedIndex} label={t("accounts.cacheShort")} color="#54b98b" />
      </div>

      <div className="usage-chart-models" aria-live="polite">
        <span className="usage-chart-models-label">{t("accounts.modelComposition")} · {day.date}</span>
        {models.length ? models.map(([name, value], index) => (
          <span key={name} className="usage-model-chip" style={{ "--model-color": MODEL_COLORS[index] } as CSSProperties} title={`${name} · ${formatTokens(value)} Token`}>
            <i /> <span>{name}</span> <strong>{formatTokens(value)}</strong>
          </span>
        )) : <span className="text-xs text-muted-foreground">{t("accounts.localUsageEmpty")}</span>}
      </div>
    </section>
  );
}
