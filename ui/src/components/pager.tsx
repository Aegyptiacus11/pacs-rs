// The footer of a paged list: where we are, rows per page (kept in this browser), first,
// previous, numbered pages around this one, next, last, and a page typed in (Enter goes).

import { useEffect, useState } from "react";
import { ChevronDown, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { DropdownMenu, DropdownMenuContent, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";

// rows per page, kept in this browser
export const PAGE_SIZES = [25, 50, 100, 200];
export const PAGE_KEY = "pacs-rs-page-size";
export function savedPageSize() {
  try {
    const n = Number(localStorage.getItem(PAGE_KEY));
    return PAGE_SIZES.includes(n) ? n : 50;
  } catch {
    return 50;
  }
}

/** `compact`: for a narrow list, the page as "2 / 6" between the arrows instead of numbers. */
export function Pager({
  total,
  pageIndex,
  pageSize,
  loading,
  compact,
  onPage,
  onPageSize,
}: {
  total: number;
  pageIndex: number;
  pageSize: number;
  loading: boolean;
  compact?: boolean;
  onPage: (i: number) => void;
  onPageSize: (n: number) => void;
}) {
  const pages = Math.max(1, Math.ceil(total / pageSize));
  const near = [...new Set([0, pageIndex - 1, pageIndex, pageIndex + 1, pages - 1])].filter((i) => i >= 0 && i < pages).sort((a, b) => a - b);
  const items: (number | "gap")[] = [];
  for (const i of near) {
    const last = items[items.length - 1];
    if (typeof last === "number" && i - last > 1) items.push(i - last === 2 ? last + 1 : "gap");
    items.push(i);
  }
  return (
    <div className={cn("flex flex-wrap items-center gap-y-2 border-t bg-background py-2 text-sm text-muted-foreground", compact ? "gap-x-3 px-5" : "gap-x-6 px-6")}>
      <span className="tabular inline-flex items-center gap-2">
        {(pageIndex * pageSize + 1).toLocaleString()}–{Math.min(total, (pageIndex + 1) * pageSize).toLocaleString()} of {total.toLocaleString()}
        {loading && <Loader2 className="size-3.5 animate-spin" aria-label="Loading the page" />}
      </span>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="sm" className="h-7 gap-1 px-2 font-normal text-muted-foreground">
            <span className="tabular">{pageSize}</span> per page <ChevronDown className="size-3.5" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="w-36">
          <DropdownMenuRadioGroup value={String(pageSize)} onValueChange={(v) => onPageSize(Number(v))}>
            {PAGE_SIZES.map((n) => (
              <DropdownMenuRadioItem key={n} value={String(n)}>
                {n} per page
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      {pages > 1 && (
        <nav aria-label="Pages" className="ml-auto flex items-center gap-1">
          <Button variant="ghost" size="icon-sm" aria-label="First page" disabled={pageIndex === 0} onClick={() => onPage(0)}>
            <ChevronsLeft />
          </Button>
          <Button variant="ghost" size="icon-sm" aria-label="Previous page" disabled={pageIndex === 0} onClick={() => onPage(pageIndex - 1)}>
            <ChevronLeft />
          </Button>
          {compact ? (
            <span className="tabular flex items-center gap-1 px-1 text-xs">
              <GoTo pageIndex={pageIndex} pages={pages} onPage={onPage} /> / {pages}
            </span>
          ) : (
            items.map((i, k) =>
              i === "gap" ? (
                <span key={`gap${k}`} className="w-7 text-center">…</span>
              ) : (
                <Button
                  key={i}
                  variant="ghost"
                  size="sm"
                  aria-label={`Page ${i + 1}`}
                  aria-current={i === pageIndex ? "page" : undefined}
                  className={cn("tabular h-8 min-w-8 px-2 font-normal", i === pageIndex && "bg-selection-soft font-medium text-selection hover:bg-selection-soft hover:text-selection")}
                  onClick={() => onPage(i)}
                >
                  {i + 1}
                </Button>
              ),
            )
          )}
          <Button variant="ghost" size="icon-sm" aria-label="Next page" disabled={pageIndex >= pages - 1} onClick={() => onPage(pageIndex + 1)}>
            <ChevronRight />
          </Button>
          <Button variant="ghost" size="icon-sm" aria-label="Last page" disabled={pageIndex >= pages - 1} onClick={() => onPage(pages - 1)}>
            <ChevronsRight />
          </Button>
          {!compact && pages > 5 && (
            <label className="ml-2 flex items-center gap-1.5 text-xs">
              Page <GoTo pageIndex={pageIndex} pages={pages} onPage={onPage} /> of {pages.toLocaleString()}
            </label>
          )}
        </nav>
      )}
    </div>
  );
}

// a page number typed in: Enter (or leaving the field) goes there, kept within 1 and the last
function GoTo({ pageIndex, pages, onPage }: { pageIndex: number; pages: number; onPage: (i: number) => void }) {
  const [text, setText] = useState(String(pageIndex + 1));
  useEffect(() => setText(String(pageIndex + 1)), [pageIndex]);
  const go = () => {
    const n = Math.round(Number(text));
    if (!Number.isFinite(n) || n < 1) return setText(String(pageIndex + 1));
    const to = Math.min(pages, n) - 1;
    setText(String(to + 1));
    if (to !== pageIndex) onPage(to);
  };
  return (
    <Input
      inputMode="numeric"
      aria-label="Go to page"
      value={text}
      onChange={(e) => setText(e.target.value.replace(/\D/g, ""))}
      onKeyDown={(e) => e.key === "Enter" && go()}
      onBlur={go}
      onFocus={(e) => e.target.select()}
      className="tabular h-7 w-14 px-2 text-center text-xs"
    />
  );
}
