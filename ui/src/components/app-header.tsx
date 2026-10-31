// The bar along the top: the name, the two lists, the search (Ctrl K, ⌘K on a Mac, or /) of
// the list shown, light or dark, and a warning when the server doesn't answer.

import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileStack, Moon, Sun, Users, WifiOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { SearchBar } from "@/components/search-bar";
import { ROOT } from "@/lib/api";
import { go, href, type Route } from "@/lib/route";
import { setSearch, useSearch } from "@/lib/search";
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

function Tab({ to, active, Icon, children }: { to: string; active: boolean; Icon: typeof Users; children: string }) {
  return (
    <a
      href={to}
      aria-current={active ? "page" : undefined}
      className={cn(
        "relative flex h-14 items-center gap-2 px-3 text-sm transition-colors",
        active ? "font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
      )}
    >
      <Icon className="size-4" strokeWidth={1.75} />
      {children}
      {/* the page shown: a line along the bar's bottom edge */}
      {active && <span className="absolute inset-x-2 -bottom-px h-0.5 rounded-full bg-foreground" />}
    </a>
  );
}

export function AppHeader({ route }: { route: Route }) {
  const page = route.page === "patients" ? "patients" : "studies";
  const text = useSearch(page);
  // Ctrl K (⌘K) or /: the search
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.key === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && !(e.target as HTMLElement).closest("input, textarea, [contenteditable]"))) {
        const box = document.querySelector<HTMLInputElement>("[data-search]");
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
    <header className="flex h-14 shrink-0 items-center gap-6 border-b bg-card px-5">
      <a href={href.studies()} className="flex items-center gap-2 font-semibold tracking-tight">
        <Logo className="size-6" />
        pacs-rs
      </a>
      <nav aria-label="Lists" className="-mb-px flex self-stretch">
        <Tab to={href.studies()} active={page === "studies"} Icon={FileStack}>
          Studies
        </Tab>
        <Tab to={href.patients()} active={page === "patients"} Icon={Users}>
          Patients
        </Tab>
      </nav>
      <SearchBar
        key={page}
        value={text}
        onChange={(v) => setSearch(page, v)}
        placeholder={page === "studies" ? "Search studies and patients: name, ID, accession, description…" : "Search patients: name or patient ID…"}
        suggest={page === "studies"}
        onPatient={(p) => go(href.patients(p.id))}
        className="mx-auto w-full max-w-xl"
      />
      <div className="flex items-center gap-2">
        <ServerStatus />
        <ThemeToggle />
      </div>
    </header>
  );
}
