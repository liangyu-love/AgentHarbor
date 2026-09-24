export function isTauriRuntime(): boolean {
  return !!(window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
}
