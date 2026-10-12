/** Asks the app shell to show Settings → Terminal proxy. */
export const OPEN_TERMINAL_PROXY_EVENT = "agentharbor:open-terminal-proxy";

export function openTerminalProxySettings() {
  window.dispatchEvent(new CustomEvent(OPEN_TERMINAL_PROXY_EVENT));
}
