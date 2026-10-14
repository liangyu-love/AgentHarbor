import { useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { NewApiRouterView } from "@/types/newapi";
import { cn } from "@/lib/utils";
import { ClientGlyph } from "./configuration-shortcuts";
import { CLIENTS, linkTone, useRoutingLinks, type Client } from "./use-routing-links";

const STORAGE_KEY = "agentharbor.inject-client.v1";
/** Shortcuts elsewhere on the page ask the console to show a client. */
export const SELECT_CLIENT_EVENT = "agentharbor:select-client";

const readStored = (): Client | null => {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    return (CLIENTS as string[]).includes(value ?? "") ? (value as Client) : null;
  } catch {
    return null;
  }
};

/** The fixed header of one client's panel: what it is, its state and its actions. */
export function ConsoleHeader({ titleId, title, leading, meta, actions }: {
  titleId: string;
  title: ReactNode;
  leading?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <div className="flex min-w-0 flex-1 basis-56 flex-wrap items-center gap-2.5">
        {leading}
        {/* Shortcuts move focus here when the panel has no control to land on. */}
        <h2 id={titleId} tabIndex={-1} className="min-w-0 font-display text-xl font-semibold tracking-tight text-foreground focus:outline-none">
          {title}
        </h2>
        {meta}
      </div>
      {actions && <div className="flex shrink-0 flex-wrap items-center gap-2">{actions}</div>}
    </div>
  );
}

// One client at a time: a rail of the clients with how each is connected,
// and the chosen client's whole configuration beside it. Every panel stays
// mounted so unsaved edits survive switching.
export function ClientConsole({ router, panels }: {
  router: NewApiRouterView | undefined;
  panels: Record<Client, ReactNode>;
}) {
  const { t } = useTranslation();
  const { links, uses } = useRoutingLinks(router);
  const [selected, setSelected] = useState<Client>(() => readStored() ?? "codex");
  const tabs = useRef<Record<string, HTMLButtonElement | null>>({});

  const select = (client: Client) => {
    setSelected(client);
    try {
      localStorage.setItem(STORAGE_KEY, client);
    } catch {
      // Storage unavailable: the choice lasts for this visit.
    }
  };

  useEffect(() => {
    const show = (event: Event) => {
      const client = (event as CustomEvent<string>).detail;
      if ((CLIENTS as string[]).includes(client)) select(client as Client);
    };
    window.addEventListener(SELECT_CLIENT_EVENT, show);
    return () => window.removeEventListener(SELECT_CLIENT_EVENT, show);
  }, []);

  const move = (event: KeyboardEvent<HTMLButtonElement>, client: Client) => {
    const index = CLIENTS.indexOf(client);
    const next = event.key === "ArrowDown" || event.key === "ArrowRight" ? CLIENTS[(index + 1) % CLIENTS.length]
      : event.key === "ArrowUp" || event.key === "ArrowLeft" ? CLIENTS[(index - 1 + CLIENTS.length) % CLIENTS.length]
      : event.key === "Home" ? CLIENTS[0]
      : event.key === "End" ? CLIENTS[CLIENTS.length - 1]
      : null;
    if (!next) return;
    event.preventDefault();
    select(next);
    tabs.current[next]?.focus();
  };

  return (
    <section className="inject-console harbor-card !p-0" aria-label={t("newapi.groups.clients")} data-client-console>
      <div role="tablist" aria-orientation="vertical" aria-label={t("newapi.groups.clients")} className="inject-rail">
        <p className="inject-rail-kicker poster-kicker text-[10px] text-muted-foreground">{t("newapi.console.kicker")}</p>
        {CLIENTS.map((client) => {
          const link = links[client];
          const current = selected === client;
          return (
            <button
              key={client}
              ref={(node) => { tabs.current[client] = node; }}
              type="button"
              role="tab"
              id={`client-tab-${client}`}
              aria-selected={current}
              aria-controls={`client-panel-${client}`}
              tabIndex={current ? 0 : -1}
              data-client-tab={client}
              data-link={link}
              onClick={() => select(client)}
              onKeyDown={(event) => move(event, client)}
              className="inject-tab group"
            >
              <span className="inject-tab-glyph">
                <ClientGlyph client={client} className="h-[18px] w-[18px]" />
              </span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-display text-[14px] font-semibold tracking-tight text-foreground">
                  {t(`newapi.topology.client.${client}`)}
                </span>
                <span className={cn("block truncate text-[11px]",
                  link === "routed" ? "text-primary" : link === "direct" ? "text-emerald-600 dark:text-emerald-400" : "text-muted-foreground")}>
                  {link === "routed"
                    ? t("newapi.topology.routedTo", { targets: uses[client].map((destination) => t(`newapi.topology.dest.${destination}`)).join(" + ") })
                    : t(`newapi.topology.link.${link}`)}
                </span>
              </span>
              <span aria-hidden className={cn("h-2 w-2 shrink-0 rounded-full", linkTone[link])} />
            </button>
          );
        })}
      </div>
      {CLIENTS.map((client) => (
        <div
          key={client}
          role="tabpanel"
          id={`client-panel-${client}`}
          aria-labelledby={`client-tab-${client}`}
          hidden={selected !== client}
          className="inject-panel min-w-0"
        >
          {panels[client]}
        </div>
      ))}
    </section>
  );
}
