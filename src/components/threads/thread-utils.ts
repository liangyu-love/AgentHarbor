import type { ThreadSummary } from "@/types/threads";

export type StatusFilter = "all" | "active" | "archived";
export type SortOption = "updatedAt" | "messages" | "fileSize" | "createdAt";

export interface FolderGroup {
  key: string;
  cwd: string;
  folderName: string;
  items: ThreadSummary[];
  latestAt: number;
  totalMessages: number;
  totalSize: number;
  userMessages: number;
  agentMessages: number;
}

export function formatSize(bytes: number): string {
  if (!bytes || bytes <= 0) return "0 B";
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${bytes} B`;
}

export function formatTokenCount(tokens: number): string {
  if (!tokens || tokens <= 0) return "0";
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(tokens >= 100_000 ? 0 : 1)}K`;
  return String(tokens);
}

export function filterAndSortThreads(
  threads: ThreadSummary[],
  status: StatusFilter,
  query: string,
  sortBy: SortOption = "updatedAt",
): ThreadSummary[] {
  const keyword = query.trim().toLowerCase();
  const filtered = threads.filter((thread) => {
    if (status === "active" && thread.archived) return false;
    if (status === "archived" && !thread.archived) return false;
    if (!keyword) return true;
    return (
      (thread.title || "").toLowerCase().includes(keyword) ||
      (thread.cwd || "").toLowerCase().includes(keyword) ||
      (thread.folderName || "").toLowerCase().includes(keyword) ||
      (thread.id || "").toLowerCase().includes(keyword) ||
      (thread.modelProvider || "").toLowerCase().includes(keyword) ||
      (thread.model || "").toLowerCase().includes(keyword)
    );
  });

  return filtered.sort((a, b) => {
    switch (sortBy) {
      case "messages":
        return (b.userMessages + b.agentMessages) - (a.userMessages + a.agentMessages);
      case "fileSize":
        return b.fileSize - a.fileSize;
      case "createdAt":
        return b.createdAt - a.createdAt;
      case "updatedAt":
      default:
        return b.updatedAt - a.updatedAt;
    }
  });
}

// Retain backwards compatibility for filterThreads
export function filterThreads(
  threads: ThreadSummary[],
  status: StatusFilter,
  query: string,
): ThreadSummary[] {
  return filterAndSortThreads(threads, status, query, "updatedAt");
}

export const DIRECT_CHATS_KEY = "virtual:direct-chats";

// Normalize identity only; keep the original cwd on threads for file operations.
export function normalizeProjectPath(cwd: string): string {
  let path = cwd.replace(/\\/g, "/");
  if (/^\/\/\?\/UNC\//i.test(path)) path = "//" + path.slice(8);
  else if (path.startsWith("//?/")) path = path.slice(4);
  path = path.replace(/\/+$/, "");
  if (/^[a-z]:/i.test(path) || path.startsWith("//")) path = path.toLowerCase();
  return path || (cwd.startsWith("/") ? "/" : "");
}

export function threadProjectKey(thread: Pick<ThreadSummary, "cwd">): string {
  const path = normalizeProjectPath(thread.cwd);
  // Codex's dated scratch workspaces are direct chats, including renamed ones.
  // A real project named new-chat outside this location remains a project.
  if (/(?:^|\/)documents\/codex\/\d{4}-\d{2}-\d{2}\/[^/]+(?:\/.*)?$/i.test(path)) {
    return DIRECT_CHATS_KEY;
  }
  return path || "virtual:unknown-workspace";
}

export function groupThreads(threads: ThreadSummary[], unknownFolder: string, directChats = "Direct chats"): FolderGroup[] {
  const groups = new Map<string, FolderGroup>();
  for (const thread of threads) {
    const key = threadProjectKey(thread);
    const existing = groups.get(key);
    const msgCount = (thread.userMessages || 0) + (thread.agentMessages || 0);
    if (existing) {
      existing.items.push(thread);
      existing.latestAt = Math.max(existing.latestAt, thread.updatedAt);
      existing.totalMessages += msgCount;
      existing.userMessages += thread.userMessages || 0;
      existing.agentMessages += thread.agentMessages || 0;
      existing.totalSize += thread.fileSize || 0;
      continue;
    }
    groups.set(key, {
      key,
      cwd: key === DIRECT_CHATS_KEY ? "" : thread.cwd,
      folderName: key === DIRECT_CHATS_KEY ? directChats : thread.folderName || thread.cwd || unknownFolder,
      items: [thread],
      latestAt: thread.updatedAt,
      totalMessages: msgCount,
      totalSize: thread.fileSize || 0,
      userMessages: thread.userMessages || 0,
      agentMessages: thread.agentMessages || 0,
    });
  }
  return [...groups.values()].sort((a, b) => b.latestAt - a.latestAt);
}
