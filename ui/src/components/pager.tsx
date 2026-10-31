// The footer of a paged list: how much of it is shown ("1–50 of 1,234 studies"), and the
// previous and next pages. Fifty rows a page; past that, narrowing the list beats paging it.

import { ChevronLeft, ChevronRight, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";

export const PAGE_SIZE = 50;

export function Pager({ total, pageIndex, noun, loading, onPage }: { total: number; pageIndex: number; noun: [string, string]; loading: boolean; onPage: (i: number) => void }) {
  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const first = pageIndex * PAGE_SIZE + 1;
  const last = Math.min(total, (pageIndex + 1) * PAGE_SIZE);
  return (
    <div className="flex h-12 shrink-0 items-center gap-3 border-t bg-background px-5 text-sm text-muted-foreground">
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
      {pages > 1 && (
        <nav aria-label="Pages" className="ml-auto flex shrink-0">
          <Button variant="outline" size="sm" className="h-8 rounded-r-none bg-card pl-2" disabled={pageIndex === 0} onClick={() => onPage(pageIndex - 1)}>
            <ChevronLeft /> Previous
          </Button>
          <Button variant="outline" size="sm" className="-ml-px h-8 rounded-l-none bg-card pr-2" disabled={pageIndex >= pages - 1} onClick={() => onPage(pageIndex + 1)}>
            Next <ChevronRight />
          </Button>
        </nav>
      )}
    </div>
  );
}
