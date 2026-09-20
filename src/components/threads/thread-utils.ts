import type { ThreadSummary } from "@/types/threads";

export type StatusFilter = "all" | "active" | "archived";

export interface FolderGroup {
  key: string;
  cwd: string;
  folderName: string;
  items: ThreadSummary[];
  latestAt: number;
}

export function filterThreads(
  threads: ThreadSummary[],
  status: StatusFilter,
  query: string,
): ThreadSummary[] {
  const keyword = query.trim().toLowerCase();
  return threads.filter((thread) => {
    if (status === "active" && thread.archived) return false;
    if (status === "archived" && !thread.archived) return false;
    if (!keyword) return true;
    return (
      thread.title.toLowerCase().includes(keyword) ||
      thread.cwd.toLowerCase().includes(keyword) ||
      thread.folderName.toLowerCase().includes(keyword)
    );
  });
}

export function groupThreads(threads: ThreadSummary[], unknownFolder: string): FolderGroup[] {
  const groups = new Map<string, FolderGroup>();
  for (const thread of threads) {
    const key = thread.cwd || unknownFolder;
    const existing = groups.get(key);
    if (existing) {
      existing.items.push(thread);
      existing.latestAt = Math.max(existing.latestAt, thread.updatedAt);
      continue;
    }
    groups.set(key, {
      key,
      cwd: thread.cwd,
      folderName: thread.folderName || key,
      items: [thread],
      latestAt: thread.updatedAt,
    });
  }
  return [...groups.values()].sort((a, b) => b.latestAt - a.latestAt);
}
