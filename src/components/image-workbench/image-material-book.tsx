import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, FolderInput, FolderOpen, FolderPlus, ImagePlus, Loader2, Pencil, Plus, RotateCw, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ConfirmDialog, type ConfirmRequest } from "@/components/ui/confirm-dialog";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { toast } from "@/hooks/use-toast";
import { api, type ImageMaterial, type ImageMaterialGroup } from "@/lib/api";
import { formatInvokeError } from "@/lib/invoke-error";
import { cn } from "@/lib/utils";
import { editorKeys } from "./prompt-template-dialog";
import { BookHeader, GroupNameInput } from "./library-parts";
import {
  DragGhost, MaterialMenu, MoveToButton, useMaterialDrag, useMaterialSelection, type MaterialMenuRequest,
} from "./material-selection";
import { IMAGE_MATERIAL_LIMITS, useImageMaterials } from "./use-image-materials";
import { usePointerSort } from "./use-pointer-sort";
import type { LibraryBook } from "./use-prompt-library";
import { MAX_REFERENCES, type ReferenceImage } from "./workbench-types";

/** Long edge of a grid thumbnail; a 2x screen still shows it sharp. */
const THUMB_EDGE = 320;

/** One picture's thumbnail, read once and kept for the session. */
function MaterialThumb({ item }: { item: ImageMaterial }) {
  const thumb = useQuery({
    queryKey: ["image-material-thumb", item.id],
    queryFn: () => api.image.loadImageMaterial(item.id, THUMB_EDGE),
    staleTime: Infinity,
  });
  if (thumb.data) {
    return <img src={thumb.data} alt={item.name} draggable={false} className="h-full w-full object-cover" />;
  }
  return (
    <span className="flex h-full w-full items-center justify-center text-[10px] text-muted-foreground">
      {thumb.isError ? "读取失败" : <Loader2 className="h-3.5 w-3.5 animate-spin" />}
    </span>
  );
}

/** The 画谱's image book: pictures the user keeps in groups, a click away from the references. */
export function ImageMaterialBook({ onKindChange, collapsed, onCollapsedChange, references, onAddReferences, onRemoveReference }: {
  onKindChange: (kind: LibraryBook) => void;
  collapsed: boolean;
  onCollapsedChange: (collapsed: boolean) => void;
  references: ReferenceImage[];
  onAddReferences: (references: Omit<ReferenceImage, "key">[]) => number;
  onRemoveReference: (key: string) => void;
}) {
  const library = useImageMaterials();
  const { groups } = library;
  const [activeId, setActiveId] = useState<string | null>(null);
  const [naming, setNaming] = useState<{ groupId: string | null } | null>(null);
  const [editing, setEditing] = useState<ImageMaterial | null>(null);
  const [confirm, setConfirm] = useState<ConfirmRequest | null>(null);
  const [picking, setPicking] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);
  const [menu, setMenu] = useState<MaterialMenuRequest | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const active = groups.find(group => group.id === activeId) ?? groups[0];
  const activeItems = useMemo(() => active?.items ?? [], [active]);
  const groupSort = usePointerSort({ onMove: (from, to) => void library.moveGroup(from, to), disabled: naming !== null });
  const selection = useMaterialSelection(activeItems, active?.id);

  /** Move pictures to another group, saying where they went. */
  const moveTo = async (ids: string[], groupId: string) => {
    const target = groups.find(group => group.id === groupId);
    if (ids.length === 0 || !target) return;
    if (await library.moveItems(ids, groupId)) {
      selection.clear();
      toast({ title: `已移动 ${ids.length} 张图片到「${target.name}」`, variant: "success" });
    }
  };
  const materialDrag = useMaterialDrag({ groupId: active?.id, onDrop: (ids, groupId) => void moveTo(ids, groupId) });
  const chosen = (id: string) => references.some(reference => reference.materialId === id);

  const nameTaken = (name: string, exceptId: string | null) =>
    groups.some(group => group.name === name && group.id !== exceptId);

  const submitGroupName = async (name: string) => {
    const target = naming;
    setNaming(null);
    if (!target) return;
    if (target.groupId) {
      await library.renameGroup(target.groupId, name);
      return;
    }
    const id = await library.addGroup(name);
    if (id) setActiveId(id);
  };

  /** A click puts the picture among the references, or takes it back out. */
  const toggle = async (item: ImageMaterial) => {
    const loaded = references.find(reference => reference.materialId === item.id);
    if (loaded) {
      onRemoveReference(loaded.key);
      return;
    }
    if (picking) return;
    setPicking(item.id);
    try {
      const src = await api.image.loadImageMaterial(item.id);
      onAddReferences([{ src, materialId: item.id, name: item.name, note: item.note }]);
    } catch (cause) {
      toast({ title: "未能载入图片", description: formatInvokeError(cause), variant: "destructive" });
    } finally {
      setPicking(null);
    }
  };

  const confirmDeleteGroup = (group: ImageMaterialGroup) => setConfirm({
    title: `删除分组「${group.name}」？`,
    description: group.items.length > 0
      ? `其中 ${group.items.length} 张图片会从素材文件夹一并删除，且无法恢复。`
      : "这个分组还没有图片。",
    confirmLabel: "删除分组",
    destructive: true,
    onConfirm: () => {
      setConfirm(null);
      void library.deleteGroup(group.id);
    },
  });

  const confirmDeleteItems = (ids: string[]) => {
    const items = activeItems.filter(item => ids.includes(item.id));
    if (items.length === 0) return;
    setConfirm({
      title: items.length === 1 ? `删除图片「${items[0].name}」？` : `删除选中的 ${items.length} 张图片？`,
      description: "图片会从素材文件夹删除，且无法恢复；已加入参考图的不受影响。",
      confirmLabel: items.length === 1 ? "删除图片" : `删除 ${items.length} 张`,
      destructive: true,
      onConfirm: () => {
        setConfirm(null);
        void library.deleteItems(items.map(item => item.id)).then(saved => saved && selection.clear());
      },
    });
  };

  const addFiles = async (files: File[]) => {
    if (!active || files.length === 0) return;
    setAdding(true);
    await library.addImages(active.id, files);
    setAdding(false);
  };

  /** Import a whole folder: its pictures into this group, its subfolders into their own. */
  const importFolder = async () => {
    if (!active) return;
    const directory = await library.pickDirectory();
    if (!directory) return;
    setAdding(true);
    await library.importFolder(active.id, directory);
    setAdding(false);
  };

  /** Move the materials to another folder; sharing the gallery's folder is the user's call. */
  const changeFolder = async () => {
    const selected = await library.pickDirectory();
    if (!selected || await library.moveTo(selected) !== "shared") return;
    setConfirm({
      title: "和历史画册用同一个文件夹？",
      description: "这个文件夹正被历史画册使用。合用后素材图片也会出现在画册里，在画册里删除一张，素材里的那张也会一起删掉。",
      confirmLabel: "仍然使用",
      onConfirm: () => {
        setConfirm(null);
        void library.moveTo(selected, true);
      },
    });
  };

  const openFolder = async () => {
    try {
      await api.openPath(library.directory);
    } catch (cause) {
      toast({ title: "无法打开素材文件夹", description: formatInvokeError(cause), variant: "destructive" });
    }
  };

  const body = () => {
    if (library.loadError) {
      return (
        <div role="alert" className="flex flex-col items-center gap-2 py-6 text-center">
          <p className="font-serif text-xs text-destructive">图片素材读取失败：{library.loadError}</p>
          <Button type="button" variant="outline" size="xs" className="gap-1 font-serif" onClick={library.reload}>
            <RotateCw className="h-3 w-3" /> 重新读取
          </Button>
        </div>
      );
    }
    if (!library.ready) {
      return (
        <p className="py-6 text-center font-serif text-xs text-muted-foreground">
          {library.loading ? "正在展开图片素材…" : "图片素材保存在本机，仅在桌面端可用。"}
        </p>
      );
    }
    return (
      <>
        <div role="tablist" aria-label="图片分组" className={cn(
          "flex flex-wrap gap-1 rounded-lg bg-background/45 p-1 ring-1 ring-border/45",
          groupSort.drag && "select-none",
        )}>
          {groups.map((group, index) => {
            const selected = group.id === active?.id;
            if (naming?.groupId === group.id) {
              return (
                <GroupNameInput key={group.id} label="分组" initial={group.name} taken={name => nameTaken(name, group.id)}
                  onSubmit={name => void submitGroupName(name)} onCancel={() => setNaming(null)} />
              );
            }
            const dragging = groupSort.drag?.from === index;
            const dropTarget = (groupSort.drag?.over === index && !dragging) || materialDrag.drag?.over === group.id;
            return (
              <button
                key={group.id}
                ref={groupSort.itemRef(index)}
                type="button"
                role="tab"
                aria-selected={selected}
                data-image-material-group={group.name}
                data-material-drop={group.id}
                title="单击切换 · 双击重命名 · 拖动或 Alt + ←→ 排序 · 把图片拖到这里可移入"
                onClick={() => setActiveId(group.id)}
                onDoubleClick={() => setNaming({ groupId: group.id })}
                {...groupSort.handleProps(index, groups.length)}
                className={cn(
                  "inline-flex h-7 max-w-[12rem] shrink-0 touch-none select-none items-center justify-center gap-1 whitespace-nowrap rounded-md px-2.5 font-serif text-[11px] outline-none transition-[background-color,color,box-shadow,opacity] focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 focus-visible:ring-offset-1",
                  selected
                    ? "bg-background font-bold text-[#b83b38] shadow-xs ring-1 ring-[#b83b38]/20 dark:bg-white/[0.06] dark:text-[#fca5a5] dark:ring-[#d94f4b]/25"
                    : "text-muted-foreground hover:bg-muted/70 hover:text-foreground",
                  dragging && "opacity-50",
                  dropTarget && "ring-2 ring-[#b83b38]/40",
                )}
              >
                <span className="truncate">{group.name}</span>
                <span className={cn("min-w-3.5 text-center font-sans text-[9px] tabular-nums opacity-60", selected && "opacity-75")}>
                  {group.items.length}
                </span>
              </button>
            );
          })}
          {naming?.groupId === null ? (
            <GroupNameInput label="分组" initial="" taken={name => nameTaken(name, null)}
              onSubmit={name => void submitGroupName(name)} onCancel={() => setNaming(null)} />
          ) : (
            <button
              type="button"
              onClick={() => setNaming({ groupId: null })}
              disabled={groups.length >= IMAGE_MATERIAL_LIMITS.groups}
              className="inline-flex h-7 shrink-0 items-center gap-1 rounded-md border border-dashed border-border/70 px-2 font-serif text-[11px] text-muted-foreground outline-none transition-colors hover:border-[#b83b38]/40 hover:text-foreground focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 disabled:opacity-50"
            >
              <Plus className="h-3 w-3" /> 新建分组
            </button>
          )}
        </div>
        {!active ? (
          <p className="mx-auto max-w-xs py-6 text-center font-serif text-xs leading-relaxed text-muted-foreground">
            把自己准备的参考图收在这里：人物、服装、姿势、背景……先建一个分组，再添加图片。点一下图片就会加入图生图的参考图。
          </p>
        ) : (
          <>
            {selection.ids.length > 0 ? (
              <div role="toolbar" aria-label="选中的图片" data-material-selection
                className="flex min-h-6 items-center justify-between gap-2 rounded-md bg-[#b83b38]/[0.07] px-2 py-0.5 dark:bg-[#d94f4b]/10">
                <p className="min-w-0 truncate font-serif text-[11px] text-[#b83b38] dark:text-[#fca5a5]">
                  已选 {selection.ids.length} 张 · 点图片继续选，Esc 退出
                </p>
                <div className="flex shrink-0 items-center gap-0.5">
                  <MoveToButton groups={groups} groupId={active.id} onMove={target => void moveTo(selection.ids, target)}>
                    <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 font-serif text-[11px]">
                      <FolderInput className="h-3.5 w-3.5" /> 移动到
                    </Button>
                  </MoveToButton>
                  <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 font-serif text-[11px] hover:text-destructive"
                    onClick={() => confirmDeleteItems(selection.ids)}>
                    <Trash2 className="h-3.5 w-3.5" /> 删除
                  </Button>
                  <Button type="button" variant="ghost" size="xs" className="h-6 px-1.5 font-serif text-[11px] text-muted-foreground"
                    onClick={selection.selectAll}>
                    全选
                  </Button>
                  <Button type="button" size="icon" variant="ghost" className="h-6 w-6 text-muted-foreground"
                    title="取消选择" aria-label="取消选择" onClick={selection.clear}>
                    <X className="h-3 w-3" />
                  </Button>
                </div>
              </div>
            ) : (
              <div className="flex items-center justify-between gap-2">
                <p className="min-w-0 truncate font-serif text-[11px] text-muted-foreground"
                  title="点图片加入或移出参考图 · Ctrl / Shift + 点击多选 · 右键或拖到分组上移动">
                  {active.items.length} 张图片{active.items.length > 0 && " · 点击加入参考图 · Ctrl / Shift 多选 · 右键或拖到分组可移动"}
                </p>
                <div className="flex shrink-0 items-center gap-0.5">
                  <Button type="button" variant="ghost" size="xs"
                    className="h-6 gap-1 px-1.5 font-serif text-[11px] text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]"
                    disabled={adding || library.itemCount >= IMAGE_MATERIAL_LIMITS.items}
                    onClick={() => inputRef.current?.click()}>
                    {adding ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <ImagePlus className="h-3.5 w-3.5" />} 添加图片
                  </Button>
                  <Button type="button" variant="ghost" size="xs"
                    className="h-6 gap-1 px-1.5 font-serif text-[11px] text-muted-foreground hover:text-[#b83b38] dark:hover:text-[#fca5a5]"
                    title="导入一个文件夹里的全部图片；子文件夹会放进同名分组"
                    disabled={adding || library.itemCount >= IMAGE_MATERIAL_LIMITS.items}
                    onClick={() => void importFolder()}>
                    <FolderPlus className="h-3.5 w-3.5" /> 导入文件夹
                  </Button>
                  <Button type="button" size="icon" variant="ghost" className="h-6 w-6 text-muted-foreground hover:text-foreground"
                    title="重命名分组" aria-label="重命名分组" onClick={() => setNaming({ groupId: active.id })}>
                    <Pencil className="h-3 w-3" />
                  </Button>
                  <Button type="button" size="icon" variant="ghost" className="h-6 w-6 text-muted-foreground hover:text-destructive"
                    title="删除分组" aria-label="删除分组" onClick={() => confirmDeleteGroup(active)}>
                    <Trash2 className="h-3 w-3" />
                  </Button>
                </div>
              </div>
            )}
            <input ref={inputRef} type="file" multiple accept="image/png,image/jpeg,image/webp,image/gif" className="hidden"
              aria-label="添加图片素材"
              onChange={event => {
                void addFiles(Array.from(event.target.files ?? []));
                event.target.value = "";
              }} />
            {active.items.length === 0 ? (
              <p className="py-6 text-center font-serif text-xs leading-relaxed text-muted-foreground">
                这个分组还没有图片。点「添加图片」选入 PNG / JPG / WebP，一次最多 {IMAGE_MATERIAL_LIMITS.add} 张；
                <br />或「导入文件夹」一次导入整个文件夹，子文件夹会放进同名分组。
                <br />文件名会成为图片的名字，可以再改名、写备注，AI 润色时模型会读到。
              </p>
            ) : (
              <div
                role="group"
                aria-label={`${active.name} 的图片`}
                className={cn(
                  "grid max-h-96 grid-cols-[repeat(auto-fill,minmax(6.5rem,1fr))] gap-2 overflow-y-auto overscroll-contain pr-1",
                  materialDrag.drag && "cursor-grabbing select-none",
                )}
                onKeyDown={event => {
                  if (event.key === "Escape" && selection.ids.length > 0) {
                    event.preventDefault();
                    selection.clear();
                  } else if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "a") {
                    event.preventDefault();
                    selection.selectAll();
                  }
                }}
              >
                {active.items.map(item => {
                  const index = references.findIndex(reference => reference.materialId === item.id);
                  const isChosen = index >= 0;
                  const picked = selection.has(item.id);
                  const carried = materialDrag.drag?.ids.includes(item.id) ?? false;
                  return (
                    <div key={item.id} data-image-material={item.name} className="group/card relative min-w-0">
                      <button
                        type="button"
                        aria-pressed={isChosen}
                        aria-label={(isChosen ? `${item.name}：已是图${index + 1}，点击移出参考图` : `${item.name}：加入参考图`)
                          + (picked ? "（已选中）" : "")}
                        data-picked={picked || undefined}
                        title={item.note || item.name}
                        disabled={(!isChosen && references.length >= MAX_REFERENCES && selection.ids.length === 0) || picking === item.id}
                        {...materialDrag.handleProps(() => selection.targetsOf(item))}
                        onClick={event => {
                          if (materialDrag.consumeClick() || selection.click(event, item)) return;
                          void toggle(item);
                        }}
                        onContextMenu={event => {
                          event.preventDefault();
                          if (!selection.has(item.id)) selection.clear();
                          setMenu({ x: event.clientX, y: event.clientY, ids: selection.targetsOf(item) });
                        }}
                        className={cn(
                          "block aspect-square w-full touch-none overflow-hidden rounded-lg border bg-card/60 outline-none transition-[border-color,box-shadow,opacity] focus-visible:ring-2 focus-visible:ring-[#b83b38]/35 disabled:cursor-not-allowed disabled:opacity-60",
                          isChosen ? "border-[#b83b38]/70 ring-2 ring-[#b83b38]/35" : "border-border/60 hover:border-[#b83b38]/40",
                          picked && "border-foreground/70 ring-2 ring-foreground/45 ring-offset-1 ring-offset-background",
                          carried && "opacity-45",
                        )}
                      >
                        <MaterialThumb item={item} />
                      </button>
                      {isChosen && (
                        <span className="pointer-events-none absolute left-1 top-1 inline-flex items-center gap-0.5 rounded bg-[#b83b38] px-1 font-sans text-[10px] font-semibold text-white">
                          <Check className="h-2.5 w-2.5" /> 图{index + 1}
                        </span>
                      )}
                      {picked && (
                        <span aria-hidden className="pointer-events-none absolute bottom-6 right-1 flex h-4 w-4 items-center justify-center rounded-full bg-foreground text-background shadow">
                          <Check className="h-2.5 w-2.5" />
                        </span>
                      )}
                      {picking === item.id && (
                        <Loader2 className="pointer-events-none absolute left-1/2 top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 animate-spin text-white drop-shadow" />
                      )}
                      {selection.ids.length === 0 && (
                        <div className="absolute right-1 top-1 flex gap-0.5 opacity-0 transition-opacity group-hover/card:opacity-100 group-focus-within/card:opacity-100">
                          <button type="button" aria-label={`编辑「${item.name}」`} title="改名 / 备注" onClick={() => setEditing(item)}
                            className="rounded bg-background/90 p-0.5 text-muted-foreground shadow-sm ring-1 ring-border/60 hover:text-foreground">
                            <Pencil className="h-3 w-3" />
                          </button>
                          <button type="button" aria-label={`删除「${item.name}」`} title="删除图片" onClick={() => confirmDeleteItems([item.id])}
                            className="rounded bg-background/90 p-0.5 text-muted-foreground shadow-sm ring-1 ring-border/60 hover:text-destructive">
                            <Trash2 className="h-3 w-3" />
                          </button>
                        </div>
                      )}
                      <p className="mt-0.5 truncate text-center font-serif text-[10px] text-muted-foreground">{item.name}</p>
                    </div>
                  );
                })}
              </div>
            )}
          </>
        )}
      </>
    );
  };

  return (
    <div data-prompt-library className="space-y-2.5 rounded-xl bg-black/[0.025] p-2.5 ring-1 ring-border/50 dark:bg-white/[0.02] sm:space-y-3 sm:p-3">
      <BookHeader kind="images" onKindChange={onKindChange} count={library.itemCount} countLabel="张图片"
        collapsed={collapsed} onCollapsedChange={onCollapsedChange}
        actions={library.ready && library.directory && (
          <>
            <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
              title="换一个文件夹存放图片素材，现有素材会一起搬过去" onClick={() => void changeFolder()}>
              <FolderInput className="h-3.5 w-3.5" /> 位置
            </Button>
            <Button type="button" variant="ghost" size="xs" className="h-6 gap-1 px-1.5 text-[11px] text-muted-foreground hover:text-foreground"
              title={`在资源管理器中打开：${library.directory}`} onClick={() => void openFolder()}>
              <FolderOpen className="h-3.5 w-3.5" /> 文件夹
            </Button>
          </>
        )} />
      {!collapsed && body()}
      <ImageMaterialDialog item={editing} onClose={() => setEditing(null)}
        onSave={async fields => {
          if (editing && await library.updateItem(editing.id, fields)) setEditing(null);
        }} />
      <MaterialMenu request={menu} onClose={() => setMenu(null)} groups={groups} groupId={active?.id} items={activeItems}
        chosen={chosen} onToggle={item => void toggle(item)} onEdit={setEditing}
        onMove={(ids, groupId) => void moveTo(ids, groupId)} onDelete={confirmDeleteItems} />
      <DragGhost drag={materialDrag.drag} groups={groups} />
      <ConfirmDialog request={confirm} onClose={() => setConfirm(null)} />
    </div>
  );
}

/** Rename a picture and write the note the polishing model reads beside it. */
function ImageMaterialDialog({ item, onClose, onSave }: {
  item: ImageMaterial | null;
  onClose: () => void;
  onSave: (fields: { name: string; note: string }) => Promise<void>;
}) {
  const [name, setName] = useState("");
  const [note, setNote] = useState("");
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (!item) return;
    setName(item.name);
    setNote(item.note);
  }, [item]);
  const save = async () => {
    if (!name.trim() || saving) return;
    setSaving(true);
    await onSave({ name, note });
    setSaving(false);
  };
  return (
    <Dialog open={item !== null} onOpenChange={open => !open && !saving && onClose()}>
      <DialogContent className="max-w-md gap-4" onKeyDown={editorKeys(() => void save())}>
        <DialogHeader>
          <DialogTitle className="font-serif">图片素材</DialogTitle>
          <DialogDescription className="font-serif text-xs">
            名字和备注会随图片一起发给润色模型，比如「白色水手服」「只参考姿势」。
          </DialogDescription>
        </DialogHeader>
        {item && (
          <div className="flex gap-3">
            <div className="h-24 w-24 shrink-0 overflow-hidden rounded-lg border bg-card/60">
              <MaterialThumb item={item} />
            </div>
            <div className="min-w-0 flex-1 space-y-2">
              <Input value={name} maxLength={IMAGE_MATERIAL_LIMITS.name} aria-label="图片名字" placeholder="名字"
                onChange={event => setName(event.target.value)} className="h-8 font-serif text-xs font-semibold" />
              <Textarea value={note} maxLength={IMAGE_MATERIAL_LIMITS.note} aria-label="图片备注"
                placeholder="备注（可选）：这张图要参考什么"
                onChange={event => setNote(event.target.value)} className="min-h-[4.5rem] resize-y font-serif text-xs" />
            </div>
          </div>
        )}
        <DialogFooter className="gap-2">
          <Button type="button" variant="outline" size="sm" onClick={onClose} disabled={saving}>取消</Button>
          <Button type="button" size="sm" className="ink-generate-button text-white" onClick={() => void save()} disabled={!name.trim() || saving}>
            {saving ? "保存中…" : "保存"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
