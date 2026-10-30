// The rail down the left side: the name, the two lists, search (⌘K) and upload; at the bottom
// light or dark, and a warning when the server doesn't answer. Icons over their names, like
// the viewer's toolbar.

import { useEffect, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { FileStack, Moon, Search, Sun, Upload, Users, WifiOff } from "lucide-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Kbd } from "@/components/ui/kbd";
import { CommandMenu } from "@/components/command-menu";
import { useUpload } from "@/components/upload";
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

const itemClass = (active?: boolean) =>
  cn(
    "relative flex h-14 w-14 flex-col items-center justify-center gap-1 rounded-lg text-muted-foreground transition-colors hover:bg-accent hover:text-foreground",
    active && "bg-selection-soft text-selection hover:bg-selection-soft hover:text-selection",
  );

function RailItem({ label, tip, shortcut, Icon, active, href: to, onClick, className }: {
  label: string;
  tip?: string;
  shortcut?: string;
  Icon: typeof Users;
  active?: boolean;
  href?: string;
  onClick?: () => void;
  className?: string;
}) {
  const body: ReactNode = (
    <>
      <Icon className="size-5" strokeWidth={1.6} />
      <span className="text-[10.5px] leading-none">{label}</span>
    </>
  );
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {to ? (
          <a href={to} aria-current={active ? "page" : undefined} className={cn(itemClass(active), className)}>
            {body}
          </a>
        ) : (
          <button type="button" onClick={onClick} className={cn(itemClass(active), className)}>
            {body}
          </button>
        )}
      </TooltipTrigger>
      <TooltipContent side="right" className="flex items-center gap-2">
        {tip ?? label}
        {shortcut && <Kbd>{shortcut}</Kbd>}
      </TooltipContent>
    </Tooltip>
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
        <span role="status" className="flex h-14 w-14 flex-col items-center justify-center gap-1 rounded-lg bg-destructive/10 text-destructive">
          <WifiOff className="size-5" strokeWidth={1.6} />
          <span className="text-[10.5px] leading-none">Offline</span>
        </span>
      </TooltipTrigger>
      <TooltipContent side="right">pacs-rs does not answer; retrying every 15 seconds</TooltipContent>
    </Tooltip>
  );
}

// one click: light to dark and back
function ThemeToggle() {
  const [theme, setTheme] = useTheme();
  const dark = theme === "dark" || (theme === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  return <RailItem label={dark ? "Light" : "Dark"} tip={dark ? "Switch to light" : "Switch to dark"} Icon={dark ? Sun : Moon} onClick={() => setTheme(dark ? "light" : "dark")} />;
}

export function AppRail({ route }: { route: Route }) {
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
  return (
    <nav aria-label="pacs-rs" className="flex w-[72px] shrink-0 flex-col items-center gap-1 border-r bg-card py-3">
      <a href={href.studies()} aria-label="pacs-rs: the studies" className="mb-3 grid size-10 place-items-center rounded-lg hover:bg-accent">
        <Logo className="size-6" />
      </a>
      <RailItem label="Studies" Icon={FileStack} href={href.studies()} active={route.page === "studies"} />
      <RailItem label="Patients" Icon={Users} href={href.patients()} active={route.page === "patients"} />
      <RailItem label="Search" tip="Search patients, studies and actions" shortcut="⌘K" Icon={Search} onClick={() => setSearch(true)} />
      <RailItem label="Upload" tip="Upload DICOM files or folders" Icon={Upload} onClick={upload.open} />
      <div className="mt-auto flex flex-col items-center gap-1">
        <ServerStatus />
        <ThemeToggle />
      </div>
      <CommandMenu open={search} onOpenChange={setSearch} />
    </nav>
  );
}
