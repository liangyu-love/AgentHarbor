import { useCallback, useState } from "react";
import { storageKeys } from "@/lib/storage-keys";

type FoldMap = Record<string, "open" | "closed">;

const readFolds = (): FoldMap => {
  try {
    const parsed: unknown = JSON.parse(localStorage.getItem(storageKeys.foldedSections) ?? "null");
    return parsed && typeof parsed === "object" && !Array.isArray(parsed) ? (parsed as FoldMap) : {};
  } catch {
    return {};
  }
};

// Remembers each section's last state; sections without a saved state use `defaultOpen`.
export function useFold(id: string, defaultOpen = true) {
  const [open, setOpen] = useState(() => {
    const saved = readFolds()[id];
    return saved === undefined ? defaultOpen : saved === "open";
  });

  const toggle = useCallback(() => {
    const next = !open;
    setOpen(next);
    try {
      localStorage.setItem(
        storageKeys.foldedSections,
        JSON.stringify({ ...readFolds(), [id]: next ? "open" : "closed" }),
      );
    } catch {
      // Storage unavailable: the fold only lasts for this session.
    }
  }, [id, open]);

  return { open, toggle };
}
