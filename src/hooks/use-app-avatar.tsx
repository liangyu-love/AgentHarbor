import { createContext, useContext, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "@/hooks/use-toast";

export const DEFAULT_AVATAR = new URL("../../assets/app-icon.png", import.meta.url).href;
const STORAGE_KEY = "aimami_avatar";
const MAX_FILE_BYTES = 5 * 1024 * 1024;
const AVATAR_SIZE = 256;

function loadAvatar(): string | null {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    return saved?.startsWith("data:image/png;base64,") && saved.length < 512 * 1024 ? saved : null;
  } catch {
    return null;
  }
}

// Store a small, self-contained image so moving/deleting the original file is safe.
async function prepareAvatar(file: File): Promise<string> {
  if (file.size > MAX_FILE_BYTES) throw new Error("avatar.tooLarge");
  if (!["image/png", "image/jpeg", "image/webp"].includes(file.type)) {
    throw new Error("avatar.invalidType");
  }
  const url = URL.createObjectURL(file);
  try {
    const image = new Image();
    image.src = url;
    await image.decode();
    const { naturalWidth: width, naturalHeight: height } = image;
    if (!width || !height || width > 8192 || height > 8192) {
      throw new Error("avatar.invalidDimensions");
    }
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = AVATAR_SIZE;
    const context = canvas.getContext("2d");
    if (!context) throw new Error("avatar.readFailed");
    const side = Math.min(width, height);
    context.drawImage(image, (width - side) / 2, (height - side) / 2, side, side, 0, 0, AVATAR_SIZE, AVATAR_SIZE);
    return canvas.toDataURL("image/png");
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("avatar.")) throw error;
    throw new Error("avatar.readFailed");
  } finally {
    URL.revokeObjectURL(url);
  }
}

interface AppAvatarState {
  src: string;
  isCustom: boolean;
  busy: boolean;
  errorKey: string | null;
  chooseImage: () => void;
  reset: () => void;
}

const AppAvatarContext = createContext<AppAvatarState | null>(null);

export function AppAvatarProvider({ children }: { children: ReactNode }) {
  const { t } = useTranslation();
  const [avatar, setAvatar] = useState(loadAvatar);
  const [busy, setBusy] = useState(false);
  const [errorKey, setErrorKey] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const savingRef = useRef(false);

  const reportError = (key: string) => {
    setErrorKey(key);
    toast({ title: t("avatar.failed"), description: t(key), variant: "destructive" });
  };

  const save = (next: string | null) => {
    try {
      if (next) localStorage.setItem(STORAGE_KEY, next);
      else localStorage.removeItem(STORAGE_KEY);
    } catch {
      reportError("avatar.saveFailed");
      return;
    }
    setAvatar(next);
    setErrorKey(null);
    toast({ title: t(next ? "avatar.saved" : "avatar.restored"), variant: "success" });
  };

  const upload = async (file: File) => {
    if (savingRef.current) return;
    savingRef.current = true;
    setBusy(true);
    setErrorKey(null);
    try {
      save(await prepareAvatar(file));
    } catch (error) {
      reportError(error instanceof Error ? error.message : "avatar.readFailed");
    } finally {
      savingRef.current = false;
      setBusy(false);
    }
  };

  return (
    <AppAvatarContext.Provider value={{
      src: avatar ?? DEFAULT_AVATAR,
      isCustom: avatar !== null,
      busy,
      errorKey,
      chooseImage: () => { if (!savingRef.current) inputRef.current?.click(); },
      reset: () => { if (!savingRef.current) save(null); },
    }}>
      {children}
      <input
        ref={inputRef}
        type="file"
        accept="image/png,image/jpeg,image/webp"
        hidden
        aria-label={t("avatar.choose")}
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = "";
          if (file) void upload(file);
        }}
      />
    </AppAvatarContext.Provider>
  );
}

export function useAppAvatar() {
  const context = useContext(AppAvatarContext);
  if (!context) throw new Error("useAppAvatar requires AppAvatarProvider");
  return context;
}

export function AppAvatarImage({ className }: { className?: string }) {
  const { src } = useAppAvatar();
  const { t } = useTranslation();
  return <img src={src} alt={t("avatar.title")} className={className} draggable={false} onError={(event) => {
    if (event.currentTarget.src !== DEFAULT_AVATAR) event.currentTarget.src = DEFAULT_AVATAR;
  }} />;
}
