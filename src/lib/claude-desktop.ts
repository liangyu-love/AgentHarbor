import type {
  ClaudeDesktopActionRequest,
  ClaudeDesktopApplied,
  ClaudeDesktopProbeOutcome,
  ClaudeDesktopRestartResult,
  ClaudeDesktopCandidates,
  ClaudeDesktopError,
  ClaudeDesktopExitRequest,
  ClaudeDesktopPathsChoice,
  ClaudeDesktopPreview,
  ClaudeDesktopProbeRequest,
  ClaudeDesktopStatus,
  DesktopAction,
} from "@/types/claude-desktop";
import { errorMessage } from "@/lib/invoke-error";
import type { QueryClient } from "@tanstack/react-query";
import { queryKeys } from "@/lib/query-keys";

/** 供应商与能力变更只刷新投影，不自动重写 Desktop 文件。 */
export async function invalidateClaudeDesktop(queryClient: QueryClient) {
  await Promise.all([
    queryClient.invalidateQueries({ queryKey: queryKeys.claudeDesktopStatus }),
    queryClient.invalidateQueries({ queryKey: queryKeys.claudeDesktopCandidates }),
  ]);
}

/**
 * 后端返回结构化错误时会给出 JSON 文本；其余情况保持原样。
 *
 * 结构化错误里没有原始响应体或密钥，可以安全用于 UI 与日志。
 */
export function desktopErrorOf(cause: unknown): ClaudeDesktopError | null {
  const message = errorMessage(cause);
  if (!message.startsWith("{")) return null;
  try {
    const parsed = JSON.parse(message) as Partial<ClaudeDesktopError>;
    if (typeof parsed?.code !== "string" || typeof parsed?.message !== "string") return null;
    return {
      code: parsed.code,
      message: parsed.message,
      fileRole: parsed.fileRole ?? null,
      modelId: parsed.modelId ?? null,
      retryable: parsed.retryable ?? false,
      suggestedAction: parsed.suggestedAction ?? null,
      params: parsed.params ?? [],
    };
  } catch {
    return null;
  }
}

/**
 * 结构化错误转为界面可渲染的本地化 key。
 *
 * 非结构化错误回落到既有的 i18n key 判断。
 */
export function desktopErrorKey(cause: unknown, fallback: string): string {
  const structured = desktopErrorOf(cause);
  if (structured) return structured.message;
  const message = errorMessage(cause);
  return /^[a-zA-Z][\w-]*(\.[\w-]+)+$/.test(message) ? message : fallback;
}

export function desktopPathsChoice(choice: ClaudeDesktopPathsChoice): ClaudeDesktopPathsChoice {
  return {
    normalRoot: choice.normalRoot ?? null,
    thirdPartyRoot: choice.thirdPartyRoot ?? null,
  };
}

/**
 * Desktop 状态、候选与操作。
 *
 * 路由预览准备共享 listener；所有模式都不修改供应商的
 * enabled / 默认模型 / 协议。
 */
export const claudeDesktopApi = {
  status: () => invokeDesktop<ClaudeDesktopStatus>("get_claude_desktop_status"),
  candidates: (paths?: ClaudeDesktopPathsChoice) =>
    invokeDesktop<ClaudeDesktopCandidates>("list_claude_desktop_candidates", {
      pathsChoice: paths ? desktopPathsChoice(paths) : null,
    }),
  preview: (action: DesktopAction, request: ClaudeDesktopActionRequest) =>
    invokeDesktop<ClaudeDesktopPreview>("preview_claude_desktop_change", { action, request }),
  cancelPreview: (revision: string) => invokeDesktop<void>("cancel_claude_desktop_preview", { revision }),
  commit: (action: DesktopAction, revision: string, request: ClaudeDesktopActionRequest) =>
    invokeDesktop<ClaudeDesktopApplied>("commit_claude_desktop_change", {
      action,
      revision,
      request,
    }),
  probe: (request: ClaudeDesktopProbeRequest) =>
    invokeDesktop<ClaudeDesktopProbeOutcome>("probe_claude_desktop_provider", { request }),
  /** 先完全退出 Claude Desktop 再启动，让刚写入的配置立即生效。 */
  restart: () => invokeDesktop<ClaudeDesktopRestartResult>("restart_claude_desktop"),
  exitRequest: () => invokeDesktop<ClaudeDesktopExitRequest | null>("get_claude_desktop_exit_request"),
  resolveExit: (decision: "background" | "stopAndExit" | "leaveAndExit") =>
    invokeDesktop<void>("resolve_claude_desktop_exit", { decision }),
};

async function invokeDesktop<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}
