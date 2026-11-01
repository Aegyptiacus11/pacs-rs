// The footer of a paged list: how much of it is shown ("51–100 of 1,234 studies"), how many
// rows a page, and the pages - first, previous, the numbers around this one, next, last - with
// a box to type one in. Page buttons are icons named on hover, as everywhere in the family.

import { useState } from "react";
import { ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Loader2, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

export const PAGE_SIZE = 50;
/** The rows-a-page choices; the server serves up to 1,000 a request. */
export const PAGE_SIZES = [25, 50, 100, 200];

/** The page numbers to show (1-based): the first, the last, and the current one with a
 * neighbour either side; `null` where pages are skipped. */
export function pageStrip(page: number, pages: number): (number | null)[] {
  const wanted = new Set([1, pages, page - 1, page, page + 1]);
  const out: (number | null)[] = [];
  for (let p = 1; p <= pages; p += 1) {
    if (wanted.has(p)) out.push(p);
    else if (out.at(-1) !== null) out.push(null);
  }
  return out;
}

function Step({ label, icon: Icon, disabled, onClick }: { label: string; icon: LucideIcon; disabled: boolean; onClick: () => void }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button variant="ghost" size="icon-sm" aria-label={label} disabled={disabled} onClick={onClick}>
          <Icon />
        </Button>
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  );
}

export function Pager({
  total,
  pageIndex,
  pageSize,
  noun,
  loading,
  onPage,
  onPageSize,
}: {
  total: number;
  pageIndex: number;
  pageSize: number;
  noun: [string, string];
  loading: boolean;
  onPage: (i: number) => void;
  onPageSize: (size: number) => void;
}) {
  const [typed, setTyped] = useState("");
  const pages = Math.max(1, Math.ceil(total / pageSize));
  const first = pageIndex * pageSize + 1;
  const last = Math.min(total, (pageIndex + 1) * pageSize);
  const go = (i: number) => {
    const to = Math.min(pages - 1, Math.max(0, i));
    if (to !== pageIndex) onPage(to);
  };
  return (
    <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-x-4 gap-y-2 border-t bg-background px-5 py-2 text-sm text-muted-foreground">
      <span className="tabular min-w-0 truncate">
        {pages > 1 && (
          <>
            <span className="font-medium text-foreground">
              {first.toLocaleString()}–{last.toLocaleString()}
            </span>{" "}
            of{" "}
          </>
        )}
        <span className="font-medium text-foreground">{total.toLocaleString()}</span> {total === 1 ? noun[0] : noun[1]}
      </span>
      {loading && <Loader2 className="size-3.5 shrink-0 animate-spin" aria-label="Loading the page" />}
      <div className="ml-auto flex flex-wrap items-center gap-x-4 gap-y-2">
        {total > PAGE_SIZES[0] && (
          <div className="flex items-center gap-2">
            <span>Rows</span>
            <ToggleGroup
              type="single"
              variant="outline"
              size="sm"
              value={String(pageSize)}
              onValueChange={(v) => v && onPageSize(Number(v))}
              aria-label="Rows a page"
            >
              {PAGE_SIZES.map((n) => (
                <ToggleGroupItem key={n} value={String(n)} className="tabular h-8 px-2.5">
                  {n}
                </ToggleGroupItem>
              ))}
            </ToggleGroup>
          </div>
        )}
        {pages > 1 && (
          <nav aria-label="Pages" className="flex items-center gap-1">
            <Step label="First page" icon={ChevronsLeft} disabled={pageIndex === 0} onClick={() => go(0)} />
            <Step label="Previous page" icon={ChevronLeft} disabled={pageIndex === 0} onClick={() => go(pageIndex - 1)} />
            {pageStrip(pageIndex + 1, pages).map((p, i) =>
              p === null ? (
                <span key={`gap-${i}`} className="px-1" aria-hidden>
                  …
                </span>
              ) : (
                <Button
                  key={p}
                  variant={p === pageIndex + 1 ? "outline" : "ghost"}
                  size="sm"
                  aria-current={p === pageIndex + 1 ? "page" : undefined}
                  aria-label={`Page ${p}`}
                  className={cn("tabular h-8 min-w-8 px-2", p === pageIndex + 1 && "border-selection/40 bg-selection-soft text-selection hover:bg-selection-soft dark:border-selection/40 dark:bg-selection-soft")}
                  onClick={() => go(p - 1)}
                >
                  {p}
                </Button>
              ),
            )}
            <Step label="Next page" icon={ChevronRight} disabled={pageIndex >= pages - 1} onClick={() => go(pageIndex + 1)} />
            <Step label="Last page" icon={ChevronsRight} disabled={pageIndex >= pages - 1} onClick={() => go(pages - 1)} />
            {pages > 5 && (
              <form
                className="ml-1"
                onSubmit={(e) => {
                  e.preventDefault();
                  const n = parseInt(typed, 10);
                  if (Number.isFinite(n)) go(n - 1);
                  setTyped("");
                }}
              >
                <Input
                  value={typed}
                  onChange={(e) => setTyped(e.target.value.replace(/\D/g, ""))}
                  inputMode="numeric"
                  placeholder="Page"
                  aria-label={`Go to page (1 to ${pages})`}
                  title={`Go to page (1 to ${pages}), then Enter`}
                  className="tabular h-8 w-16 bg-card text-center"
                />
              </form>
            )}
          </nav>
        )}
      </div>
    </div>
  );
}
