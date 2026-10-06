import {
  SearchIcon,
  SparklesIcon,
  CheckCircle2Icon,
  MicIcon,
  PauseIcon,
  PlayIcon,
  SettingsIcon,
} from "lucide-react";

import { cn } from "@/lib/utils";
import { ScrollArea } from "@/components/ui/scroll-area";

export type View = "today" | "memory" | "actions" | "meetings" | "settings";

const items: { view: View; label: string; icon: typeof SparklesIcon }[] = [
  { view: "today", label: "Today", icon: SparklesIcon },
  { view: "memory", label: "Search", icon: SearchIcon },
  { view: "actions", label: "Follow-ups", icon: CheckCircle2Icon },
];

interface SidebarProps {
  view: View;
  onChange: (view: View) => void;
  actionCount: number;
  paused: boolean;
  onPauseToggle: () => void;
  onRecord: () => void;
}

export function Sidebar({ view, onChange, actionCount, paused, onPauseToggle, onRecord }: SidebarProps) {
  return (
    <nav aria-label="Main navigation" className="app-sidebar">
      <ScrollArea className="min-h-0 flex-1">
      <div className="flex min-h-full flex-col pr-1">
      <div className="flex flex-col gap-0.5">
        {items.map((item) => {
          const Icon = item.icon;
          const active = view === item.view;
          return (
            <button
              key={item.view}
              type="button"
              aria-current={active ? "page" : undefined}
              className={cn("nav-item", active && "nav-item-active")}
              onClick={() => onChange(item.view)}
            >
              <Icon />
              <span>{item.label}</span>
              {item.view === "actions" && actionCount > 0 ? <span className="nav-count">{actionCount}</span> : null}
            </button>
          );
        })}
      </div>

      <div className="mt-auto border-t pt-4">
        <button type="button" className="nav-item" onClick={onRecord}>
          <MicIcon /> <span>Record</span>
        </button>
        <button className={cn("nav-item", view === "settings" && "nav-item-active")} onClick={() => onChange("settings")} aria-current={view === "settings" ? "page" : undefined}>
          <SettingsIcon /> <span>Settings</span>
        </button>
        <button className="capture-control" onClick={onPauseToggle} aria-label={paused ? "Capture is paused. Resume capture" : "Memento is remembering. Pause capture"} title={paused ? "Resume capture" : "Pause capture"}>
          <span className={cn("capture-status-dot", paused ? "bg-amber-500" : "capture-pulse bg-emerald-500")} />
          <strong className="min-w-0 flex-1 text-left">{paused ? "Paused" : "Remembering"}</strong>
          {paused ? <PlayIcon /> : <PauseIcon />}
        </button>
      </div>
      </div>
      </ScrollArea>
    </nav>
  );
}
