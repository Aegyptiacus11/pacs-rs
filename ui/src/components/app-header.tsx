// The frame around the pages. Down the left, the rail: the name and the two lists, icons over
// their names like the viewer's toolbar. Along the top, the bar: the search that finds a
// patient or a study anywhere (Ctrl K, ⌘K on a Mac), light or dark, and a warning when the
// server doesn't answer. / goes to the list's own filter.

import { useEffect, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileStack, Moon, Sun, Users, WifiOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { GlobalSearch } from "@/components/search-bar";
import { ROOT } from "@/lib/api";
import { href, type Route } from "@/lib/route";
import { useTheme } from "@/lib/theme";
import { cn } from "@/lib/utils";

export function Logo({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 20 20" className={cn("size-5", className)} aria-hidden="true">
      <rect x="1" y="1" width="18" height="18" rx="5" fill="currentColor" />
      <path d="M7 14V6h3.6a2.6 2.6 0 0 1 0 5.2H7" fill="none" stroke="var(--background)" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

// nothing while the server answers; a clear warning when it doesn't
function ServerStatus() {
  const health = useQuery({
    queryKey: ["health"],
    queryFn: async () => (await fetch(new URL("healthz", ROOT), { cache: "no-store" })).ok,
    refetchInterval: 15_000,
    retry: false,
  });
  if (health.isPending || health.data === true) return null;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span role="status" className="flex h-8 items-center gap-1.5 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 text-xs font-medium text-destructive">
          <WifiOff className="size-3.5" />
          Offline
        </span>
      </TooltipTrigger>
      <TooltipContent>pacs-rs does not answer; retrying every 15 seconds</TooltipContent>
    </Tooltip>
  );
}

// one click: light to dark and back
function ThemeToggle() {
  const [theme, setTheme] = useTheme();
  const dark = theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  const label = dark ? "Switch to light" : "Switch to dark";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon" aria-label={label} className="text-muted-foreground hover:text-foreground" onClick={() => setTheme(dark ? "light" : "dark")}>
          {dark ? <Sun /> : <Moon />}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

function RailItem({ label, Icon, active, to }: { label: string; Icon: typeof Users; active: boolean; to: string }) {
  const body: ReactNode = (
    <>
      <Icon className="size-5" strokeWidth={1.6} />
      <span className="text-[10.5px] leading-none">{label}</span>
    </>
  );
  return (
    <a
      href={to}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-14 w-14 flex-col items-center justify-center gap-1 rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
        active && "bg-selection-soft text-selection hover:bg-selection-soft hover:text-selection",
      )}
    >
      {body}
    </a>
  );
}

export function AppRail({ route }: { route: Route }) {
  return (
    <nav aria-label="pacs-rs" className="flex w-[72px] shrink-0 flex-col items-center gap-1 border-r bg-card pb-3">
      {/* as tall as the top bar, its line carried across */}
      <div className="mb-2 grid h-14 w-full shrink-0 place-items-center border-b">
        <a href={href.studies()} aria-label="pacs-rs: the studies" className="grid size-10 place-items-center rounded-lg hover:bg-accent">
          <Logo className="size-6" />
        </a>
      </div>
      <RailItem label="Studies" Icon={FileStack} to={href.studies()} active={route.page === "studies"} />
      <RailItem label="Patients" Icon={Users} to={href.patients()} active={route.page === "patients"} />
    </nav>
  );
}

export function AppHeader({ route }: { route: Route }) {
  // Ctrl K (⌘K): the bar's search; /: the list's filter
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      const k = e.key === "k" && (e.metaKey || e.ctrlKey);
      const slash = e.key === "/" && !(e.target as HTMLElement).closest("input, textarea, [contenteditable]");
      if (k || slash) {
        const box = document.querySelector<HTMLInputElement>(k ? "[data-search]" : "[data-filter]") ?? document.querySelector<HTMLInputElement>("[data-search]");
        if (!box) return;
        e.preventDefault();
        box.focus();
        box.select();
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, []);
  return (
    <header className="flex h-14 shrink-0 items-center gap-4 border-b bg-card px-5">
      <span className="w-24 shrink-0 font-semibold tracking-tight">pacs-rs</span>
      <GlobalSearch page={route.page === "patients" ? "patients" : "studies"} className="mx-auto w-full max-w-xl" />
      <div className="flex w-24 shrink-0 items-center justify-end gap-2">
        <ServerStatus />
        <ThemeToggle />
      </div>
    </header>
  );
}
