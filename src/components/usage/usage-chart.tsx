import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent, type PointerEvent, type RefObject } from "react";
import { useTranslation } from "react-i18next";

import { storageKeys } from "@/lib/storage-keys";
import type { UsageBucket, UsageChannel, UsagePart, UsagePayload } from "@/types";
import { channelLabel, clientLabel, formatMetric, niceScale, seriesColor, type UsageMetric } from "./usage-format";
import { UsageSegments } from "./usage-segments";

type Grouping = "client" | "channel" | "model";
type Form = "bar" | "line";
const FORMS = ["bar", "line"] as const;

function useForm() {
  const [form, setForm] = useState<Form>(() => {
    try {
      const stored = localStorage.getItem(storageKeys.usageChartForm);
      return FORMS.includes(stored as Form) ? stored as Form : "bar";
    } catch {
      return "bar";
    }
  });
  const choose = (next: Form) => {
    setForm(next);
    try { localStorage.setItem(storageKeys.usageChartForm, next); } catch { /* storage unavailable */ }
  };
  return [form, choose] as const;
}

const HEIGHT = 232;
const TOP = 8;
const BOTTOM = 24;
const LEFT = 56;
const RIGHT = 4;

function useWidth(ref: RefObject<HTMLElement | null>) {
  const [width, setWidth] = useState(0);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    setWidth(element.getBoundingClientRect().width);
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width));
    observer.observe(element);
    return () => observer.disconnect();
  }, [ref]);
  return width;
}

/** Stacked columns or one line per series, per hour or day, drawn at the container's real width so labels stay legible. */
export function UsageChart({ data, metric }: { data: UsagePayload; metric: UsageMetric }) {
  const { t } = useTranslation();
  const [grouping, setGrouping] = useState<Grouping>("client");
  const [form, setForm] = useForm();
  const [active, setActive] = useState<number | null>(null);
  const box = useRef<HTMLDivElement>(null);
  const width = useWidth(box);
  const { buckets } = data;
  const partsOf = (bucket: UsageBucket): UsagePart[] =>
    grouping === "client" ? bucket.byClient : grouping === "channel" ? bucket.byChannel : bucket.byModel;
  const amount = (part: UsagePart) => metric === "cost" ? part.costMicros : part.newTokens;

  // The largest series sits at the base; the folded rest goes on top. Colour
  // follows the range's own ranking, so switching cost and tokens never repaints.
  const series = useMemo(() => {
    const totals = new Map<string, number>();
    for (const bucket of buckets) {
      for (const part of partsOf(bucket)) totals.set(part.key, (totals.get(part.key) ?? 0) + amount(part));
    }
    const ranked = (grouping === "client" ? data.byClient : grouping === "channel" ? data.byChannel : data.byModel).map(group => group.key);
    const seen = [...totals.keys()];
    const rank = (key: string) => ranked.includes(key) ? ranked.indexOf(key) : ranked.length + seen.indexOf(key);
    const colored = seen.filter(key => key !== "other").sort((a, b) => rank(a) - rank(b));
    const keys = [...colored].sort((a, b) => totals.get(b)! - totals.get(a)!);
    if (totals.has("other")) keys.push("other");
    return keys.map(key => ({ key, total: totals.get(key) ?? 0, color: seriesColor(colored.indexOf(key), key) }));
    // partsOf and amount follow grouping and metric.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [buckets, grouping, metric, data.byClient, data.byChannel, data.byModel]);
  const modelNames = useMemo(() => new Map(data.byModel.map(group => [group.key, group.label])), [data.byModel]);
  const seriesName = (key: string) => key === "other" ? t("usage.chart.other")
    : grouping === "client" ? clientLabel(key, t)
    : grouping === "channel" ? channelLabel(key as UsageChannel, null, t)
    : modelNames.get(key) ?? key;
  const valueOf = (bucket: UsageBucket, key: string) =>
    amount(partsOf(bucket).find(part => part.key === key) ?? { key, newTokens: 0, costMicros: 0 });
  const bucketTotal = (bucket: UsageBucket) => partsOf(bucket).reduce((sum, part) => sum + amount(part), 0);
  // Lines compare series against each other, so their scale is the largest single value.
  const max = Math.max(0, ...buckets.map(bucket => form === "bar" ? bucketTotal(bucket)
    : Math.max(0, ...series.map(item => valueOf(bucket, item.key)))));
  const { top, step } = niceScale(max);
  const innerWidth = Math.max(0, width - LEFT - RIGHT);
  const innerHeight = HEIGHT - TOP - BOTTOM;
  const slot = buckets.length ? innerWidth / buckets.length : 0;
  const bar = Math.max(2, Math.min(slot * 0.62, 44));
  const y = (value: number) => TOP + innerHeight - value / top * innerHeight;
  const ticks = Array.from({ length: Math.round(top / step) + 1 }, (_, index) => index * step);
  const labelEvery = buckets.length > 24 ? 5 : buckets.length > 12 ? 3 : 1;
  const bucketName = (bucket: UsageBucket, index: number) => data.range === "today"
    ? t("usage.chart.hour", { hour: bucket.key })
    : index === buckets.length - 1 ? t("accounts.todayShort") : bucket.key.slice(5);

  // The readout doubles as the legend: range totals at rest, one period while pointing.
  const selected = active != null ? buckets[active] : null;
  const readoutParts = series.map(item => ({ ...item, value: selected ? valueOf(selected, item.key) : item.total }));
  const center = (index: number) => LEFT + index * slot + slot / 2;
  const readoutTotal = readoutParts.reduce((sum, item) => sum + item.value, 0);
  const axis = (value: number) => metric === "tokens" ? formatMetric(value, metric)
    : `$${Number((value / 1_000_000).toPrecision(value >= 1_000_000 ? 4 : 2))}`;

  const pick = (event: PointerEvent<SVGSVGElement>) => {
    if (!slot) return;
    const x = event.clientX - event.currentTarget.getBoundingClientRect().left - LEFT;
    setActive(Math.min(buckets.length - 1, Math.max(0, Math.floor(x / slot))));
  };
  const onKey = (event: KeyboardEvent<HTMLDivElement>) => {
    const last = buckets.length - 1;
    const next = event.key === "ArrowLeft" ? Math.max(0, (active ?? last + 1) - 1)
      : event.key === "ArrowRight" ? Math.min(last, (active ?? -1) + 1)
      : event.key === "Home" ? 0 : event.key === "End" ? last : event.key === "Escape" ? null : undefined;
    if (next === undefined) return;
    event.preventDefault();
    setActive(next);
  };

  return <section className="harbor-card route-rise" aria-labelledby="usage-chart-title" data-usage-chart>
    <div className="usage-card-head">
      <div className="usage-card-title">
        <span className="poster-kicker text-[9.5px] text-primary">Trend</span>
        <h3 id="usage-chart-title" className="font-display">{t("usage.chart.title")}</h3>
        <p>{t(`usage.chart.subtitle.${data.range}`)}</p>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <UsageSegments label={t("usage.chart.formLabel")} value={form} onChange={setForm}
          items={FORMS.map(value => ({ value, label: t(`usage.chart.form.${value}`) }))} />
        <UsageSegments label={t("usage.chart.groupLabel")} value={grouping} onChange={setGrouping}
          items={(["client", "channel", "model"] as const).map(value => ({ value, label: t(`usage.chart.group.${value}`) }))} />
      </div>
    </div>
    <div className="usage-readout" aria-live="polite" data-usage-readout>
      <span className="usage-readout-when">
        {selected ? (data.range === "today" ? t("usage.chart.hour", { hour: selected.key }) : selected.key) : t("usage.chart.total")}
      </span>
      <strong className="usage-readout-value poster-number">{formatMetric(readoutTotal, metric)}</strong>
      <span className="usage-readout-parts">
        {readoutParts.map(item => <span key={item.key} data-usage-series={item.key}>
          <i className="usage-swatch" style={{ "--swatch": item.color } as CSSProperties} />
          {seriesName(item.key)}<strong>{formatMetric(item.value, metric)}</strong>
        </span>)}
      </span>
    </div>
    <div ref={box} className="usage-chart" tabIndex={0} role="group" aria-label={t("usage.chart.label")}
      data-active={active != null || undefined} onKeyDown={onKey} onBlur={() => setActive(null)}>
      {max === 0 ? <div className="usage-chart-empty">{t("usage.chart.empty")}</div> : width > 0 &&
        <svg width={width} height={HEIGHT} viewBox={`0 0 ${width} ${HEIGHT}`} aria-hidden
          onPointerMove={pick} onPointerLeave={() => setActive(null)}>
          {ticks.map(value => <g key={value}>
            <line x1={LEFT} x2={width - RIGHT} y1={y(value)} y2={y(value)} className="usage-chart-grid" />
            <text x={LEFT - 8} y={y(value) + 3.5} textAnchor="end" className="usage-chart-axis">{axis(value)}</text>
          </g>)}
          {buckets.map((bucket, index) => {
            const x = LEFT + index * slot + (slot - bar) / 2;
            let base = 0;
            return <g key={bucket.key} className="usage-chart-col" data-active={index === active || undefined}>
              {form === "bar" && series.map(item => {
                const value = valueOf(bucket, item.key);
                if (value <= 0) return null;
                const height = value / top * innerHeight;
                // A 2px gap keeps neighbouring segments apart where hues sit close.
                const rect = <rect key={item.key} x={x} width={bar} y={y(base + value)}
                  height={Math.max(1, height - (height > 4 ? 2 : 0))} rx={Math.min(3, bar / 4)} fill={item.color} />;
                base += value;
                return rect;
              })}
              {(index % labelEvery === 0 || index === buckets.length - 1) &&
                <text x={center(index)} y={HEIGHT - 6} textAnchor="middle" className="usage-chart-axis">
                  {bucketName(bucket, index)}
                </text>}
            </g>;
          })}
          {form === "line" && <>
            {active != null && <line x1={center(active)} x2={center(active)} y1={TOP} y2={TOP + innerHeight} className="usage-chart-crosshair" />}
            {/* The largest series is drawn last, on top. */}
            {[...series].reverse().map(item => <g key={item.key} className="usage-chart-line" style={{ "--series": item.color } as CSSProperties}>
              <polyline points={buckets.map((bucket, index) => `${center(index)},${y(valueOf(bucket, item.key))}`).join(" ")} />
              {active != null && <circle cx={center(active)} cy={y(valueOf(buckets[active], item.key))} r={4} />}
            </g>)}
          </>}
        </svg>}
    </div>
  </section>;
}
