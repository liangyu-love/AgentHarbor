import i18n from "@/lib/i18n";

/**
 * Error contract between Rust commands and the UI.
 *
 * A command rejects with a string (Tauri serializes `Result<_, String>` errors as
 * the bare string, not an `Error`). That string is either an i18n key such as
 * `newapi.errors.network`, or raw text from the OS, SQLite, or an upstream,
 * which is kept as-is because it is what users and bug reports need. Every UI
 * path should go through these helpers instead of `String(error)`, which prints
 * `Error: ` prefixes and untranslated keys.
 */

const I18N_KEY = /^[a-zA-Z][\w-]*(\.[\w-]+)+$/;

/** The message carried by a rejected invoke or thrown value, trimmed. */
export function errorMessage(error: unknown): string {
  if (error == null) return "";
  if (typeof error === "string") return error.trim();
  if (error instanceof Error) return error.message.trim();
  if (typeof error === "object") {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message.trim();
    try {
      const text = String(error);
      if (text !== "[object Object]") return text.trim();
    } catch {
      // not stringifiable
    }
  }
  return "";
}

/**
 * The i18n key an error carries when it starts with one of `prefixes`, otherwise
 * `fallback`. For forms that store the key and render `t(key)` later.
 */
export function errorKey(error: unknown, prefixes: string[], fallback: string): string {
  const message = errorMessage(error);
  return prefixes.some(prefix => message.startsWith(`${prefix}.`)) && I18N_KEY.test(message)
    ? message
    : fallback;
}

/**
 * Readable text for any error: a known i18n key is translated, raw text is shown
 * unchanged, and an empty error or an unknown key falls back to `fallback`.
 */
export function formatInvokeError(error: unknown, fallback?: string): string {
  const message = errorMessage(error);
  const generic = fallback ?? i18n.t("common.toastErrorGenericDesc");
  if (!message) return generic;
  if (I18N_KEY.test(message)) {
    return i18n.exists(message) ? i18n.t(message) : generic;
  }
  return message;
}
