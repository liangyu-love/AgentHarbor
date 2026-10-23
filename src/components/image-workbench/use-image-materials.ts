import { useCallback, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, type ImageMaterial, type ImageMaterialGroup, type ImageMaterialLibrary } from "@/lib/api";
import { toast } from "@/hooks/use-toast";
import { errorMessage, formatInvokeError } from "@/lib/invoke-error";
import { queryKeys } from "@/lib/query-keys";
import { isTauriRuntime } from "@/lib/tauri-runtime";
import { fileToDataUrl } from "./workbench-types";

/** Mirrors the limits `image_materials.rs` enforces. */
export const IMAGE_MATERIAL_LIMITS = {
  groups: 100,
  items: 1000,
  add: 50,
  name: 80,
  note: 200,
} as const;

const KEY = queryKeys.imageMaterials;

const moved = <T,>(items: T[], from: number, to: number) => {
  const next = [...items];
  const [item] = next.splice(from, 1);
  next.splice(to, 0, item);
  return next;
};

/** A file's name without its extension names the picture it becomes. */
const baseName = (file: File) => file.name.replace(/\.[^.]+$/, "").trim();

// Saves and adds run one after another: an add between two saves must not be written over.
let chain: Promise<unknown> = Promise.resolve();

/** The user's image materials: groups of pictures kept in the app data folder or one the user picked. */
export function useImageMaterials() {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: KEY,
    queryFn: () => api.image.loadImageMaterials(),
    enabled: isTauriRuntime(),
    // The UI holds the newest state; a background refetch would undo unsaved edits.
    staleTime: Infinity,
  });
  const groups = useMemo(() => query.data?.groups ?? [], [query.data]);
  const itemCount = useMemo(() => groups.reduce((sum, group) => sum + group.items.length, 0), [groups]);

  /** Apply a change now and persist it; resolves false when nothing was saved. */
  const commit = useCallback((change: (groups: ImageMaterialGroup[]) => ImageMaterialGroup[]) => {
    const current = queryClient.getQueryData<ImageMaterialLibrary>(KEY);
    // Never write over a library that failed to load.
    if (!current) return Promise.resolve(false);
    const next: ImageMaterialLibrary = { ...current, groups: change(current.groups) };
    queryClient.setQueryData(KEY, next);
    const save = chain.then(() => api.image.saveImageMaterials(next));
    chain = save.catch(() => undefined);
    return save.then(
      saved => {
        // A newer change is already showing and its own save will follow.
        if (queryClient.getQueryData(KEY) === next) queryClient.setQueryData(KEY, saved);
        return true;
      },
      cause => {
        toast({ title: "图片素材保存失败", description: formatInvokeError(cause), variant: "destructive" });
        void queryClient.invalidateQueries({ queryKey: KEY });
        return false;
      },
    );
  }, [queryClient]);

  const addGroup = useCallback(async (name: string) => {
    const id = crypto.randomUUID();
    const saved = await commit(groups => [...groups, { id, name: name.trim(), items: [] }]);
    return saved ? id : null;
  }, [commit]);

  const renameGroup = useCallback((id: string, name: string) =>
    commit(groups => groups.map(group => (group.id === id ? { ...group, name: name.trim() } : group))), [commit]);

  const deleteGroup = useCallback((id: string) =>
    commit(groups => groups.filter(group => group.id !== id)), [commit]);

  const moveGroup = useCallback((from: number, to: number) =>
    commit(groups => moved(groups, from, to)), [commit]);

  const updateItem = useCallback((id: string, fields: Pick<ImageMaterial, "name" | "note">) =>
    commit(groups => groups.map(group => ({
      ...group,
      items: group.items.map(item => (item.id === id
        ? { ...item, name: fields.name.trim(), note: fields.note.trim() }
        : item)),
    }))), [commit]);

  const deleteItem = useCallback((id: string) =>
    commit(groups => groups.map(group => ({ ...group, items: group.items.filter(item => item.id !== id) }))), [commit]);

  const moveItem = useCallback((groupId: string, from: number, to: number) =>
    commit(groups => groups.map(group => (group.id === groupId ? { ...group, items: moved(group.items, from, to) } : group))), [commit]);

  /** Copy the chosen pictures into a group; resolves how many were added. */
  const addImages = useCallback(async (groupId: string, files: File[]) => {
    if (files.length === 0) return 0;
    if (files.length > IMAGE_MATERIAL_LIMITS.add) {
      toast({ title: `一次最多添加 ${IMAGE_MATERIAL_LIMITS.add} 张图片`, variant: "destructive" });
      return 0;
    }
    try {
      const images = await Promise.all(files.map(async file => ({ name: baseName(file), dataUrl: await fileToDataUrl(file) })));
      const add = chain.then(() => api.image.addImageMaterials(groupId, images));
      chain = add.catch(() => undefined);
      queryClient.setQueryData(KEY, await add);
      toast({ title: `已添加 ${images.length} 张图片素材`, variant: "success" });
      return images.length;
    } catch (cause) {
      toast({ title: "添加图片失败", description: formatInvokeError(cause), variant: "destructive" });
      return 0;
    }
  }, [queryClient]);

  /** Ask the user for a folder; null when they cancel. */
  const pickDirectory = useCallback(async () => {
    if (!isTauriRuntime() || !queryClient.getQueryData(KEY)) return null;
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({ directory: true, multiple: false });
      return typeof selected === "string" ? selected : null;
    } catch (cause) {
      toast({ title: "无法打开文件夹选择", description: formatInvokeError(cause), variant: "destructive" });
      return null;
    }
  }, [queryClient]);

  /**
   * Keep the materials in `directory`; they move there unless it already holds some.
   * Resolves "shared" when it is the gallery's folder and `allowShared` is not set.
   */
  const moveTo = useCallback(async (directory: string, allowShared = false) => {
    try {
      const relocate = chain.then(() => api.image.setImageMaterialsDirectory(directory, allowShared));
      chain = relocate.catch(() => undefined);
      const library = await relocate;
      queryClient.setQueryData(KEY, library);
      toast({ title: "图片素材位置已更换", description: library.directory, variant: "success" });
      return "moved" as const;
    } catch (cause) {
      if (errorMessage(cause) === "image.errors.imageMaterialsShared") return "shared" as const;
      toast({ title: "更换图片素材位置失败", description: formatInvokeError(cause), variant: "destructive" });
      return "failed" as const;
    }
  }, [queryClient]);

  return {
    groups,
    itemCount,
    directory: query.data?.directory ?? "",
    loading: query.isPending && query.fetchStatus !== "idle",
    loadError: query.isError ? formatInvokeError(query.error) : null,
    ready: query.isSuccess,
    reload: () => void query.refetch(),
    addGroup,
    renameGroup,
    deleteGroup,
    moveGroup,
    updateItem,
    deleteItem,
    moveItem,
    addImages,
    pickDirectory,
    moveTo,
  };
}
