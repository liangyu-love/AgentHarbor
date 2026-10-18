import { type KeyboardEvent, type ReactNode, useEffect, useLayoutEffect, useRef, useState } from "react";

export interface InjectTab<T extends string> {
  id: T;
  icon: ReactNode;
  title: string;
  description: string;
  /** Live state shown under the title, readable from any tab. */
  summary?: ReactNode;
}

type IndicatorFrame = { x: number; width: number; instant: boolean };

/** The injection page's groups as one segmented bar. A highlight glides to
 *  the chosen tab (resizes follow at once), and each tab carries its summary.
 *  Panels are the caller's: `group-panel-{id}`, labelled by `group-tab-{id}`. */
export function InjectTabs<T extends string>({ tabs, active, onChange, label }: {
  tabs: InjectTab<T>[];
  active: T;
  onChange: (id: T) => void;
  label: string;
}) {
  const listRef = useRef<HTMLDivElement>(null);
  const [frame, setFrame] = useState<IndicatorFrame | null>(null);
  const measure = (instant: boolean) => {
    const tab = listRef.current?.querySelector<HTMLElement>(`[data-inject-tab="${active}"]`);
    setFrame(tab ? { x: tab.offsetLeft, width: tab.offsetWidth, instant } : null);
  };
  const measureRef = useRef(measure);
  measureRef.current = measure;
  const placed = useRef(false);
  useLayoutEffect(() => {
    measure(!placed.current);
    placed.current = true;
  }, [active]);
  useEffect(() => {
    const list = listRef.current;
    if (!list || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => measureRef.current(true));
    observer.observe(list);
    return () => observer.disconnect();
  }, []);

  const move = (event: KeyboardEvent<HTMLButtonElement>, index: number) => {
    const next = event.key === "ArrowRight" ? index + 1 : event.key === "ArrowLeft" ? index - 1
      : event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : null;
    if (next === null) return;
    event.preventDefault();
    const tab = tabs[(next + tabs.length) % tabs.length];
    onChange(tab.id);
    listRef.current?.querySelector<HTMLElement>(`[data-inject-tab="${tab.id}"]`)?.focus();
  };
  const current = tabs.find(tab => tab.id === active);

  return (
    <div>
      <div ref={listRef} role="tablist" aria-label={label} className="group-tabs">
        <span aria-hidden className="group-tabs-indicator" data-instant={frame?.instant ? "" : undefined}
          style={frame ? { width: frame.width, transform: `translateX(${frame.x}px)` } : { opacity: 0 }} />
        {tabs.map((tab, index) => {
          const selected = tab.id === active;
          return (
            <button key={tab.id} type="button" role="tab" id={`group-tab-${tab.id}`} data-inject-tab={tab.id}
              aria-selected={selected} aria-controls={`group-panel-${tab.id}`} tabIndex={selected ? 0 : -1}
              className="group-tab" onClick={() => onChange(tab.id)} onKeyDown={event => move(event, index)}>
              <span className="group-tab-title">
                <span aria-hidden className="group-tab-icon">{tab.icon}</span>
                <span className="truncate">{tab.title}</span>
              </span>
              <span className="group-tab-summary" data-section-summary>{tab.summary}</span>
            </button>
          );
        })}
      </div>
      {current && <p key={current.id} className="group-tabs-description wallpaper-plate">{current.description}</p>}
    </div>
  );
}
