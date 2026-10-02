/** Estimated cost formatting shared by the usage views. */
export function formatCost(micros: number) {
  const usd = micros / 1_000_000;
  if (!Number.isFinite(usd) || usd <= 0) return "$0";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  if (usd < 1) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

/** Keep the recorded subtotal visible together with its pricing coverage. */
export function costLabel(
  micros: number,
  pricedCount: number,
  unpricedCount: number,
  unpricedLabel: (count: number) => string,
  unknown: string,
  coverage?: {
    partialCount: number;
    coverageUnknownCount: number;
    partialLabel: (count: number) => string;
    coverageUnknownLabel: (count: number) => string;
  },
) {
  if (pricedCount === 0) return unknown;
  const labels = [formatCost(micros)];
  if (unpricedCount > 0) labels.push(unpricedLabel(unpricedCount));
  if (coverage?.partialCount) labels.push(coverage.partialLabel(coverage.partialCount));
  if (coverage?.coverageUnknownCount) labels.push(coverage.coverageUnknownLabel(coverage.coverageUnknownCount));
  return labels.join(" · ");
}
