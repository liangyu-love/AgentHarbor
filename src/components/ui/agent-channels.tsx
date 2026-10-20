import type { ReactNode } from "react";
import { Layers } from "lucide-react";
import { ClientGlyph, type ShortcutTarget } from "@/components/newapi/configuration-shortcuts";
import { CountUp } from "@/components/ui/count-up";
import { cn } from "@/lib/utils";

export interface AgentChannel<T extends string> {
  id: T;
  label: string;
  /** The agent's mark; none for "all". */
  glyph?: ShortcutTarget;
  count: number;
  /** A second line under the count, e.g. what is not managed yet. */
  note?: ReactNode;
  /** The agent is not installed: shown dimmed, still selectable. */
  muted?: boolean;
  title?: string;
}

/**
 * One tile per agent with what it holds; picking a tile shows only that
 * agent's entries. It is the page's filter and its tally at once.
 */
export function AgentChannels<T extends string>({ channels, value, onChange, label }: {
  channels: AgentChannel<T>[];
  value: T;
  onChange: (value: T) => void;
  label: string;
}) {
  return (
    <div className="agent-channels" role="group" aria-label={label} data-agent-channels>
      {channels.map(channel => {
        const active = channel.id === value;
        return (
          <button
            key={channel.id}
            type="button"
            aria-pressed={active}
            title={channel.title}
            data-agent-channel={channel.id}
            className={cn("agent-channel", channel.muted && "agent-channel-muted")}
            onClick={() => onChange(channel.id)}
          >
            <span className="agent-channel-glyph">
              {channel.glyph ? <ClientGlyph client={channel.glyph} className="h-4 w-4" /> : <Layers className="h-4 w-4" />}
            </span>
            <span className="min-w-0">
              <span className="block truncate text-[12px] font-medium text-muted-foreground">{channel.label}</span>
              <span className="flex items-baseline gap-1.5">
                <strong className="poster-number text-xl font-semibold leading-tight text-foreground">
                  <CountUp value={channel.count} />
                </strong>
                {channel.note && <span className="truncate text-[11px] text-muted-foreground">{channel.note}</span>}
              </span>
            </span>
          </button>
        );
      })}
    </div>
  );
}
