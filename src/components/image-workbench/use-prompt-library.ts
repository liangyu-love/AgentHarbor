import { useCallback, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type PromptGroup, type PromptLibrary, type PromptLibraryKind, type PromptTemplate } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { storageKeys } from "@/lib/storage-keys";
import { isTauriRuntime } from "@/lib/tauri-runtime";

/** What each book of the 画谱 is called, and where it is kept in the query cache. */
export const LIBRARY_BOOKS = {
  prompts: {
    key: queryKeys.imagePromptLibrary, name: "画谱", item: "模版", unit: "个", group: "分组",
    note: "备注", content: "提示词", file: "agentharbor-prompts.json", presets: "预置模版",
    notePlaceholder: "用途、适合的模型、要避开的坑…",
    contentPlaceholder: "描述画面主体、构图、光线和风格…",
  },
  materials: {
    key: queryKeys.imagePromptMaterials, name: "素材", item: "素材", unit: "条", group: "分类",
    note: "简介", content: "内容", file: "agentharbor-materials.json", presets: "预置素材",
    notePlaceholder: "适合什么画面，AI 挑选时会参考",
    contentPlaceholder: "写进提示词的描述片段，如：藏青色水手服上衣，白色水手领，深色百褶短裙",
  },
} as const;

/** The 画谱's books: the two of text, and the pictures kept as image materials. */
export type LibraryBook = PromptLibraryKind | "images";

/** The book the 画谱 last showed. */
export function rememberedBook(): LibraryBook {
  try {
    const book = localStorage.getItem(storageKeys.imagePromptBook);
    return book === "materials" || book === "images" ? book : "prompts";
  } catch {
    return "prompts";
  }
}

export function rememberBook(kind: LibraryBook) {
  try {
    localStorage.setItem(storageKeys.imagePromptBook, kind);
  } catch {
    // Storage unavailable; the 画谱 opens on the templates next time.
  }
}

/** Mirrors the limits `image_prompts.rs` enforces. */
export const PROMPT_LIMITS = {
  groups: 100,
  templates: 2000,
  groupName: 40,
  title: 80,
  note: 200,
  prompt: 32_000,
} as const;

export type TemplateDraft = Pick<PromptTemplate, "title" | "note" | "prompt">;

/** A template saved without a title is named after its prompt's opening words. */
export const titleFromPrompt = (prompt: string) =>
  Array.from((prompt.trim().split("\n")[0] ?? "").trim()).slice(0, 16).join("").trim();

// The retired built-in library kept its favorites here.
try {
  localStorage.removeItem("image-prompt-favorites");
} catch {
  // Storage unavailable; nothing to clean up.
}

/** Pinned templates lead the group, each part keeping its order. */
const pinnedFirst = (templates: PromptTemplate[]) => [
  ...templates.filter(template => template.pinned),
  ...templates.filter(template => !template.pinned),
];

const moved = <T,>(items: T[], from: number, to: number) => {
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
};

const cleanDraft = (draft: TemplateDraft): TemplateDraft => {
  const prompt = draft.prompt.trim();
  return { prompt, note: draft.note.trim(), title: draft.title.trim() || titleFromPrompt(prompt) };
};

// Each book's saves run one after another so an older write never lands after a newer one.
const saveChains: Record<PromptLibraryKind, Promise<unknown>> = {
  prompts: Promise.resolve(),
  materials: Promise.resolve(),
};

/** Resolves once every save of the book made so far has settled. */
export const librarySaved = (kind: PromptLibraryKind) => saveChains[kind];

/** The user's prompt groups and templates (or material categories and materials), saved to the app data folder on every change. */
export function usePromptLibrary(kind: PromptLibraryKind = "prompts") {
  const queryClient = useQueryClient();
  const book = LIBRARY_BOOKS[kind];
  const libraryKey = book.key;
  const query = useQuery({
    queryKey: libraryKey,
    queryFn: () => api.image.loadPromptLibrary(kind),
    enabled: isTauriRuntime(),
    // The UI holds the newest state; a background refetch would undo unsaved edits.
    staleTime: Infinity,
  });
  const groups = useMemo(() => query.data?.groups ?? [], [query.data]);
  const templateCount = useMemo(
    () => groups.reduce((sum, group) => sum + group.templates.length, 0),
    [groups],
  );

  /** Apply a change now and persist it; resolves false when nothing was saved. */
  const commit = useCallback((change: (groups: PromptGroup[]) => PromptGroup[]) => {
    const current = queryClient.getQueryData<PromptLibrary>(libraryKey);
    // Never write over a library that failed to load.
    if (!current) return Promise.resolve(false);
    const next: PromptLibrary = { groups: change(current.groups) };
    queryClient.setQueryData(libraryKey, next);
    const save = saveChains[kind].then(() => api.image.savePromptLibrary(next, kind));
    saveChains[kind] = save.catch(() => undefined);
    return save.then(
      saved => {
        // A newer change is already showing and its own save will follow.
        if (queryClient.getQueryData(libraryKey) === next) queryClient.setQueryData(libraryKey, saved);
        return true;
      },
      cause => {
        toast({ title: `${book.name}保存失败`, description: formatInvokeError(cause), variant: "destructive" });
        void queryClient.invalidateQueries({ queryKey: libraryKey });
        return false;
      },
    );
  }, [queryClient, kind]);

  const addGroup = useCallback(async (name: string) => {
    const id = crypto.randomUUID();
    const saved = await commit(groups => [...groups, { id, name: name.trim(), templates: [] }]);
    return saved ? id : null;
  }, [commit]);

  const renameGroup = useCallback((id: string, name: string) =>
    commit(groups => groups.map(group => (group.id === id ? { ...group, name: name.trim() } : group))), [commit]);

  const deleteGroup = useCallback((id: string) =>
    commit(groups => groups.filter(group => group.id !== id)), [commit]);

  const moveGroup = useCallback((from: number, to: number) =>
    commit(groups => moved(groups, from, to)), [commit]);

  /** Create a template, or update one and move it into `groupId` if it lives elsewhere. */
  const saveTemplate = useCallback(async (groupId: string, draft: TemplateDraft, templateId?: string) => {
    const id = templateId ?? crypto.randomUUID();
    const fields = cleanDraft(draft);
    const saved = await commit(groups => {
      const existing = groups.flatMap(group => group.templates).find(template => template.id === id);
      const template: PromptTemplate = { id, pinned: existing?.pinned ?? false, ...fields };
      return groups.map(group => {
        const inGroup = group.templates.some(item => item.id === id);
        if (group.id === groupId) {
          return {
            ...group,
            templates: inGroup
              ? group.templates.map(item => (item.id === id ? template : item))
              : pinnedFirst([...group.templates, template]),
          };
        }
        return inGroup ? { ...group, templates: group.templates.filter(item => item.id !== id) } : group;
      });
    });
    return saved ? id : null;
  }, [commit]);

  const deleteTemplate = useCallback((id: string) =>
    commit(groups => groups.map(group => ({
      ...group,
      templates: group.templates.filter(template => template.id !== id),
    }))), [commit]);

  /** Reorder within a group; a template stays inside its pinned or unpinned part. */
  const moveTemplate = useCallback((groupId: string, from: number, to: number) =>
    commit(groups => groups.map(group => {
      if (group.id !== groupId) return group;
      const pinnedCount = group.templates.filter(template => template.pinned).length;
      const [low, high] = group.templates[from]?.pinned
        ? [0, pinnedCount - 1]
        : [pinnedCount, group.templates.length - 1];
      return { ...group, templates: moved(group.templates, from, Math.min(high, Math.max(low, to))) };
    })), [commit]);

  /** Pinning moves a template to the end of the pinned part, unpinning to the top of the rest. */
  const togglePin = useCallback((id: string) =>
    commit(groups => groups.map(group => (
      group.templates.some(template => template.id === id)
        ? {
          ...group,
          templates: pinnedFirst(group.templates.map(template => (
            template.id === id ? { ...template, pinned: !template.pinned } : template
          ))),
        }
        : group
    ))), [commit]);

  const importLibrary = useCallback(async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({ multiple: false, directory: false, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      // Earlier edits must be on disk before the import merges into the file.
      await saveChains[kind];
      const result = await api.image.importPromptLibrary(path, kind);
      queryClient.setQueryData(libraryKey, result.library);
      const extra = [
        result.groups > 0 && `新增 ${result.groups} 个${book.group}`,
        result.skipped > 0 && `跳过 ${result.skipped} 条重复或无效内容`,
      ].filter(Boolean).join(" · ");
      toast({ title: `已导入 ${result.templates} ${book.unit}${book.item}`, description: extra || undefined, variant: "success" });
    } catch (cause) {
      toast({ title: "导入失败", description: formatInvokeError(cause), variant: "destructive" });
    }
  }, [queryClient, kind]);

  /** Bring in what AgentHarbor ships for this book; what is already there is skipped. */
  const addPresets = useCallback(async () => {
    try {
      await saveChains[kind];
      const result = await api.image.addPromptPresets(kind);
      queryClient.setQueryData(libraryKey, result.library);
      toast(result.templates > 0
        ? { title: `已添加 ${result.templates} ${book.unit}${book.presets}`, description: result.groups > 0 ? `新增 ${result.groups} 个${book.group}` : undefined, variant: "success" }
        : { title: `${book.presets}都已在${book.name}里了`, variant: "success" });
    } catch (cause) {
      toast({ title: `添加${book.presets}失败`, description: formatInvokeError(cause), variant: "destructive" });
    }
  }, [queryClient, kind]);

  const exportLibrary = useCallback(async () => {
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({ defaultPath: book.file, filters: [{ name: "JSON", extensions: ["json"] }] });
      if (typeof path !== "string") return;
      await saveChains[kind];
      await api.image.exportPromptLibrary(path, kind);
      toast({ title: `${book.name}已导出`, description: path, variant: "success" });
    } catch (cause) {
      toast({ title: "导出失败", description: formatInvokeError(cause), variant: "destructive" });
    }
  }, [kind]);

  return {
    kind,
    book,
    groups,
    templateCount,
    loading: query.isPending && query.fetchStatus !== "idle",
    loadError: query.isError ? formatInvokeError(query.error) : null,
    ready: query.isSuccess,
    reload: () => void query.refetch(),
    addGroup,
    renameGroup,
    deleteGroup,
    moveGroup,
    saveTemplate,
    deleteTemplate,
    moveTemplate,
    togglePin,
    importLibrary,
    exportLibrary,
    addPresets,
  };
}

export type PromptLibraryState = ReturnType<typeof usePromptLibrary>;
