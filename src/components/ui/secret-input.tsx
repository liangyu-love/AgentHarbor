import { type ComponentProps, useState } from "react";
import { Eye, EyeOff } from "lucide-react";

import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";

type SecretInputProps = Omit<ComponentProps<typeof Input>, "type" | "value" | "onChange"> & {
  value: string;
  onChange: (value: string) => void;
  /** A saved secret the field leaves blank; with onReveal the eye fetches it. */
  hasStoredSecret?: boolean;
  onReveal?: () => Promise<boolean>;
};

/** A password field with its own show/hide eye. WebView2 adds a reveal
 *  button of its own to password inputs; it is hidden so only one shows. */
export function SecretInput({ value, onChange, hasStoredSecret, onReveal, disabled, className, ...props }: SecretInputProps) {
  const [visible, setVisible] = useState(false);
  const [revealing, setRevealing] = useState(false);
  const canToggle = Boolean(value) || Boolean(hasStoredSecret && onReveal);
  const toggle = async () => {
    if (!value && hasStoredSecret && onReveal) {
      setRevealing(true);
      try {
        if (await onReveal()) setVisible(true);
      } finally {
        setRevealing(false);
      }
      return;
    }
    setVisible(current => !current);
  };
  return (
    <div className="relative">
      <Input
        autoComplete="off"
        spellCheck={false}
        {...props}
        type={visible ? "text" : "password"}
        className={cn("pr-9 [&::-ms-reveal]:hidden", className)}
        value={value}
        disabled={disabled || revealing}
        onChange={event => onChange(event.target.value)}
      />
      <button
        type="button"
        onClick={() => void toggle()}
        disabled={disabled || !canToggle || revealing}
        aria-label={visible ? "hide" : "show"}
        className="absolute right-2.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
      >
        {visible ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
      </button>
    </div>
  );
}
