/** Estimated cost formatting shared by the usage views. */
export function formatCost(micros: number) {
  const usd = micros / 1_000_000;
  if (!Number.isFinite(usd) || usd <= 0) return "$0";
  if (usd < 0.01) return `$${usd.toFixed(4)}`;
  if (usd < 1) return `$${usd.toFixed(3)}`;
  return `$${usd.toFixed(2)}`;
}

/** `$x · N unpriced` when part of the usage has no price, else just `$x`. */
export function costLabel(
  micros: number,
  pricedCount: number,
  unpricedCount: number,
  unpricedLabel: (count: number) => string,
  unknown: string,
) {
  if (pricedCount === 0) return unknown;
  const label = formatCost(micros);
  return unpricedCount > 0 ? `${label} · ${unpricedLabel(unpricedCount)}` : label;
}
