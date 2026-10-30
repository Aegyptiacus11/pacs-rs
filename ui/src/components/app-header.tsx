// The header: the name, the two lists, search (⌘K), upload and whether the server answers.

import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileStack, Search, Upload, Users, WifiOff } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { CommandMenu } from "@/components/command-menu";
import { useUpload } from "@/components/upload";
import { ROOT } from "@/lib/api";
import { href, type Route } from "@/lib/route";
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
        <span className="flex h-8 items-center gap-1.5 rounded-md border border-destructive/30 bg-destructive/10 px-2.5 text-xs font-medium text-destructive">
          <WifiOff className="size-3.5" />
          Server unreachable
        </span>
      </TooltipTrigger>
      <TooltipContent>pacs-rs does not answer; retrying every 15 seconds</TooltipContent>
    </Tooltip>
  );
}

export function AppHeader({ route }: { route: Route }) {
  const upload = useUpload();
  const [search, setSearch] = useState(false);
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if ((e.key === "k" && (e.metaKey || e.ctrlKey)) || (e.key === "/" && !(e.target as HTMLElement).closest("input, textarea, [contenteditable]"))) {
        e.preventDefault();
        setSearch((s) => !s);
      }
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, []);
  const tab = (to: string, active: boolean, Icon: typeof Users, label: string) => (
    <a
      href={to}
      className={cn(
        "flex h-8 items-center gap-2 rounded-md px-3 text-sm transition-colors",
        active ? "bg-secondary font-medium text-foreground" : "text-muted-foreground hover:text-foreground",
      )}
    >
      <Icon className="size-4" strokeWidth={1.75} />
      {label}
    </a>
  );
  return (
    <header className="sticky top-0 z-30 border-b bg-background/90 backdrop-blur supports-[backdrop-filter]:bg-background/75">
      <div className="mx-auto flex h-14 max-w-[1440px] items-center gap-6 px-6">
        <a href={href.studies()} className="flex items-center gap-2 font-semibold tracking-tight">
          <Logo />
          pacs-rs
        </a>
        <nav className="flex items-center gap-1">
          {tab(href.studies(), route.page === "studies", FileStack, "Studies")}
          {tab(href.patients(), route.page === "patients", Users, "Patients")}
        </nav>
        <div className="ml-auto flex items-center gap-2">
          <Button variant="outline" className="h-8 w-56 justify-start gap-2 px-3 font-normal text-muted-foreground" onClick={() => setSearch(true)}>
            <Search className="size-4" />
            Search
            <Kbd className="ml-auto">⌘K</Kbd>
          </Button>
          <Button size="sm" onClick={upload.open}>
            <Upload /> Upload
          </Button>
          <ServerStatus />
        </div>
      </div>
      <CommandMenu open={search} onOpenChange={setSearch} />
    </header>
  );
}
