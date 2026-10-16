import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ChevronDown, KeyRound, LogIn } from "lucide-react";

import type { useAccountSources } from "@/components/accounts/use-account-sources";
import { Button, type ButtonProps } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import type { AccountSource, AccountSourceMethod } from "@/types/account-sources";

type Sources = ReturnType<typeof useAccountSources>;

/** A method's name: the source's own wording, else the method's. */
export function useMethodLabel() {
  const { t } = useTranslation();
  return (source: AccountSource, method: AccountSourceMethod) =>
    t(`accountSources.method.${source.kind}.${method.id}`, { defaultValue: t(`accountSources.method.${method.id}`, { defaultValue: method.id }) });
}

/**
 * Sign in to a source: one button for a single way, a menu of its ways
 * (browser, the vendor app's own sign-in, an API key…) otherwise.
 */
export function SourceSignInButton({ source, sources, className, size }: {
  source: AccountSource;
  sources: Sources;
  className?: string;
  size?: ButtonProps["size"];
}) {
  const { t } = useTranslation();
  const label = useMethodLabel();
  const busy = sources.pending !== null;
  const title = t("accountSources.signIn", { name: source.name });
  if (source.methods.length <= 1) {
    return (
      <Button className={className} size={size} disabled={busy} onClick={() => sources.signIn(source.kind)}>
        <LogIn />{title}
      </Button>
    );
  }
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button className={className} size={size} disabled={busy} data-sign-in-methods={source.kind}>
          <LogIn />{title}<ChevronDown className="opacity-70" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="min-w-[14rem]">
        {source.methods.map(method => (
          <DropdownMenuItem key={method.id} data-sign-in-method={method.id} onSelect={() => sources.signIn(source.kind, method.id)}>
            {method.input ? <KeyRound className="h-4 w-4 opacity-70" /> : <LogIn className="h-4 w-4 opacity-70" />}
            {label(source, method)}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** What a sign-in method asks the user to type first: an API key, a token, an instance. */
export function SignInInputDialog({ sources }: { sources: Sources }) {
  const { t } = useTranslation();
  const label = useMethodLabel();
  const ask = sources.inputFor;
  const source = ask ? sources.sources.find(item => item.kind === ask.kind) : undefined;
  const [value, setValue] = useState("");
  useEffect(() => { if (ask) setValue(""); }, [ask]);
  const key = ask ? `accountSources.input.${ask.kind}.${ask.method.id}` : "";
  return (
    <Dialog open={Boolean(ask && source)} onOpenChange={open => { if (!open) sources.dismissInput(); }}>
      {ask && source && (
        <DialogContent className="sm:max-w-md" data-sign-in-input={ask.method.id}>
          <form className="space-y-4" onSubmit={event => { event.preventDefault(); sources.submitInput(value); }}>
            <DialogHeader>
              <DialogTitle>{label(source, ask.method)}</DialogTitle>
              <DialogDescription>{t(`${key}.hint`, { defaultValue: t("accountSources.input.hint") })}</DialogDescription>
            </DialogHeader>
            <Input autoFocus value={value} onChange={event => setValue(event.target.value)}
              type={ask.method.input === "secret" ? "password" : "text"} autoComplete="off" spellCheck={false}
              aria-label={label(source, ask.method)} className="font-mono"
              placeholder={t(`${key}.placeholder`, { defaultValue: "" })} />
            <DialogFooter>
              <Button type="button" variant="outline" onClick={sources.dismissInput}>{t("common.cancel")}</Button>
              <Button type="submit" disabled={!value.trim()}><LogIn />{t("accountSources.input.submit")}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      )}
    </Dialog>
  );
}
