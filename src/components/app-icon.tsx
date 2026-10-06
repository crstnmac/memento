import { useEffect, useRef, useState } from "react";
import { AppWindowIcon } from "lucide-react";
import { api } from "@/lib/api";
import { cn } from "@/lib/utils";

const nativeRuntime = "__TAURI_INTERNALS__" in window;

/**
 * Real app icons (rendered natively by the backend and cached on disk) keyed by
 * bundle id. Icons arrive after the list does, so rows render immediately with
 * a placeholder and fill in. Already-fetched ids are never requested again.
 */
export function useAppIcons(bundleIds: string[]): Record<string, string> {
  const [icons, setIcons] = useState<Record<string, string>>({});
  const requested = useRef(new Set<string>());
  const signature = bundleIds.join("\n");

  useEffect(() => {
    if (!nativeRuntime) return;
    const missing = bundleIds.filter((id) => !requested.current.has(id));
    if (missing.length === 0) return;
    missing.forEach((id) => requested.current.add(id));
    api
      .getAppIcons(missing)
      .then((found) => setIcons((prev) => ({ ...prev, ...found })))
      .catch(() => { /* placeholders are fine */ });
    // `signature` is the stable identity of `bundleIds`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [signature]);

  return icons;
}

export function AppIcon({ src, fallback: Fallback = AppWindowIcon, className }: {
  src?: string;
  fallback?: React.ComponentType<{ className?: string }>;
  className?: string;
}) {
  return (
    <span aria-hidden="true" className={cn("grid size-7 shrink-0 place-items-center overflow-hidden rounded-md", !src && "border bg-background text-muted-foreground", className)}>
      {src ? <img src={src} alt="" width={28} height={28} className="size-7 object-contain" draggable={false} /> : <Fallback className="size-4" />}
    </span>
  );
}
