import type { KeyboardEvent } from "react";

/** Focus ring matching Button, for non-button elements made keyboard-operable. */
export const focusRing = "focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring";

/**
 * Enter/Space handler for an element given a button-like role (`button`,
 * `checkbox`), so it responds to the keyboard the way a native control does.
 */
export function onActivateKey(action: () => void) {
  return (event: KeyboardEvent) => {
    if (event.target !== event.currentTarget) return;
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      action();
    }
  };
}

/** Props that make a clickable non-button element reachable and operable by keyboard. */
export function pressable(action: () => void) {
  return { role: "button" as const, tabIndex: 0, onClick: action, onKeyDown: onActivateKey(action) };
}
