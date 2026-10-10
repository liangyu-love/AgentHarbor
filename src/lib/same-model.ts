// A model's name as vendors agree on it, so the same model served by several
// relays can be found however each spells it. Ported from magpie's sameModel
// (MIT, Copyright (c) 2026 yetone): lowercase, without the vendor's prefix
// ("anthropic/claude-sonnet-5" is claude-sonnet-5), with a version's dot as
// Anthropic writes it ("claude-opus-5.5" is claude-opus-5-5, and so is
// Fireworks' "p" for the dot), "_" as "-", and without a snapshot date
// ("-20260801", Vertex's "@20260801", Ark's "-260910"). Variants such as
// ":batch", "-thinking" and four-digit releases ("-2507") stay distinct.
export function sameModel(id: string): string {
  let key = id.trim().toLowerCase().replaceAll("_", "-");
  const slash = key.lastIndexOf("/");
  if (slash >= 0) key = key.slice(slash + 1);
  const chars = [...key];
  for (let i = 1; i + 1 < chars.length; i++) {
    if ((chars[i] === "." || chars[i] === "p") && isDigit(chars[i - 1]) && isDigit(chars[i + 1])) chars[i] = "-";
  }
  key = chars.join("");
  const cut = Math.max(key.lastIndexOf("-"), key.lastIndexOf("@"));
  if (cut > 0 && snapshotDate(key.slice(cut + 1))) key = key.slice(0, cut);
  return key;
}

const isDigit = (char: string) => char >= "0" && char <= "9";

// YYYYMMDD from 2000 on, or Ark's YYMMDD from 2023 on with a real month and day.
function snapshotDate(text: string): boolean {
  if (!/^\d+$/.test(text)) return false;
  if (text.length === 8) return text.startsWith("20");
  if (text.length !== 6) return false;
  const [year, month, day] = [text.slice(0, 2), text.slice(2, 4), text.slice(4, 6)].map(Number);
  return year >= 23 && year <= 39 && month >= 1 && month <= 12 && day >= 1 && day <= 31;
}
