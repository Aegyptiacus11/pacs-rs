// The studies: a worklist on the left, the chosen study on the right. A click on a row shows
// the study in the pane (its patient, series, details, the patient's other studies and the
// actions, once); a double click or Enter opens it in the viewer; ↑ ↓ move through the list;
// checkboxes act on several studies at once; a right click has the actions too. The server
// filters, sorts and pages (/ui/api/studies): the browser holds one page at a time.

import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  flexRender,
  getCoreRowModel,
  useReactTable,
  type ColumnDef,
  type RowSelectionState,
  type SortingState,
} from "@tanstack/react-table";
import { ArrowDown, ArrowUp, CalendarRange, ChevronDown, ChevronLeft, ChevronRight, ChevronsLeft, ChevronsRight, Copy, Download, Ellipsis, Eye, FileStack, Loader2, MousePointerClick, Search, Trash2, Upload, User, X } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Calendar } from "@/components/ui/calendar";
import { Separator } from "@/components/ui/separator";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { FacetedFilter } from "@/components/faceted-filter";
import { ConfirmDelete, type PendingDelete } from "@/components/confirm-delete";
import { SeriesThumb } from "@/components/series-thumb";
import { useUpload } from "@/components/upload";
import { Modalities } from "@/components/modalities";
import { PAGE_SIZE, Pager } from "@/components/pager";
import { IMAGING_MODALITIES } from "@/lib/modalities";
import { bytes, formatDate, formatTime, initials, isoDate, parseDate, personName, plural, sexAge, url } from "@/lib/api";
import { go, href } from "@/lib/route";
import { setSearch, useSearch } from "@/lib/search";
import {
  deleteStudies,
  usePatientStudies,
  useRefreshStudies,
  useStudy,
  useStudyFacets,
  useStudyPage,
  useStudySeries,
  useStudySummary,
  type Study,
  type StudyFilters,
  type StudyQuery,
  type StudySummary,
} from "@/lib/studies";
import { cn } from "@/lib/utils";

const RANGES = [
  ["any", "All", null],
  ["today", "Today", 0],
  ["7d", "7 days", 7],
  ["30d", "30 days", 30],
  ["1y", "1 year", 365],
] as const;
type Range = (typeof RANGES)[number][0];
/** A custom range: ISO dates (yyyy-mm-dd) from the date fields, either end open. */
type Span = { from: string; to: string };
const key = (iso: string) => iso.replaceAll("-", "");
function inSpan(date: string, { from, to }: Span) {
  if (from && !(date && date >= key(from))) return false;
  if (to && !(date && date <= key(to))) return false;
  return true;
}
function spanLabel({ from, to }: Span) {
  if (from && to) return from === to ? formatDate(key(from)) : `${formatDate(key(from))} – ${formatDate(key(to))}`;
  return from ? `From ${formatDate(key(from))}` : `Until ${formatDate(key(to))}`;
}
const RANGE_NAMES: Record<Range, string> = { any: "All time", today: "Today", "7d": "Last 7 days", "30d": "Last 30 days", "1y": "Last 12 months" };

const SEXES: Record<string, string> = { F: "Female", M: "Male", O: "Other" };

function sinceFor(range: Range) {
  const days = RANGES.find(([k]) => k === range)?.[2];
  if (days == null) return null;
  const since = new Date();
  since.setHours(0, 0, 0, 0);
  since.setDate(since.getDate() - days);
  return since;
}

// a study opens as its hanging protocol lays it out; a series, on its own
function openStudy(id: string, seriesId?: string) {
  go(href.view(id, seriesId));
}

function copy(text: string, what: string) {
  navigator.clipboard?.writeText(text);
  toast(`${what} copied`);
}

function SortHeader({ column, children, className }: { column: any; children: ReactNode; className?: string }) {
  const sorted = column.getIsSorted();
  return (
    <button
      className={cn("-mx-2 inline-flex h-7 items-center gap-1 rounded px-2 hover:bg-muted hover:text-foreground", sorted && "text-foreground", className)}
      onClick={() => column.toggleSorting(sorted === "asc")}
    >
      {children}
      {sorted === "asc" ? <ArrowUp className="size-3.5" /> : sorted === "desc" ? <ArrowDown className="size-3.5" /> : null}
    </button>
  );
}

// the table's columns, as the server sorts them
const SORT_KEYS: Record<string, StudyQuery["sort"]> = { name: "patient", description: "description", modality: "modality", date: "date", series: "series", instances: "images" };
const ymd = (d: Date) => `${d.getFullYear()}${String(d.getMonth() + 1).padStart(2, "0")}${String(d.getDate()).padStart(2, "0")}`;

/** The dates a range keeps, `YYYYMMDD`, either end open. */
function rangeDates(range: Range | "custom", span: Span): { from: string | null; to: string | null } {
  if (range === "custom") return { from: span.from ? key(span.from) : null, to: span.to ? key(span.to) : null };
  const since = sinceFor(range);
  return { from: since ? ymd(since) : null, to: null };
}

export function StudiesPage({ patient, open }: { patient: string | null; open: string | null }) {
  const refresh = useRefreshStudies();
  const upload = useUpload();
  // the top bar's search
  const typed = useSearch("studies");
  const [q, setQ] = useState(typed);
  const [modalities, setModalities] = useState<string[]>([]);
  const [range, setRange] = useState<Range | "custom">("any");
  const [span, setSpan] = useState<Span>({ from: "", to: "" });
  const [sexes, setSexes] = useState<string[]>([]);
  const [sorting, setSorting] = useState<SortingState>([{ id: "date", desc: true }]);
  const [selection, setSelection] = useState<RowSelectionState>({});
  const [pagination, setPagination] = useState({ pageIndex: 0, pageSize: PAGE_SIZE });
  const [current, setCurrent] = useState<string | null>(open);
  const [pending, setPending] = useState<PendingDelete | null>(null);
  // after ↑ or ↓ past the page's end: the row to choose once the next page is in
  const [edge, setEdge] = useState<"first" | "last" | null>(null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setCurrent(open), [open]);
  // Esc lets the chosen one go, unless it is closing something else or being typed into
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      if ((e.target as HTMLElement).closest("input, textarea, [role=dialog], [role=menu], [role=listbox]")) return;
      if (document.querySelector("[role=dialog], [role=menu]")) return;
      setCurrent(null);
    };
    addEventListener("keydown", key);
    return () => removeEventListener("keydown", key);
  }, []);
  // the URL names the study shown, so a reload or a link comes back to it
  useEffect(() => {
    const to = href.studies(patient, current);
    if (location.hash !== to) history.replaceState(null, "", to);
  }, [patient, current]);
  // the server is asked once typing pauses
  useEffect(() => {
    const t = setTimeout(() => setQ(typed), 250);
    return () => clearTimeout(t);
  }, [typed]);

  const filters: StudyFilters = { q, ...rangeDates(range, span), modalities, sexes, patient };
  const filterKey = JSON.stringify(filters);
  const sort = sorting[0] ?? { id: "date", desc: true };
  // other filters or another order: back to the first page
  useEffect(() => setPagination((p) => (p.pageIndex ? { ...p, pageIndex: 0 } : p)), [filterKey, sort.id, sort.desc]);
  const { pageIndex, pageSize } = pagination;
  const page = useStudyPage({ ...filters, sort: SORT_KEYS[sort.id] ?? "date", desc: sort.desc, offset: pageIndex * pageSize, limit: pageSize });
  const summary = useStudySummary(filters);
  const choices = useStudyFacets(patient);
  const theirs = usePatientStudies(patient);
  const rows = page.data?.studies ?? [];
  const total = page.data?.total ?? 0;

  // every study seen on a page, for the selection and the pane
  const known = useRef(new Map<string, Study>()).current;
  for (const s of [...rows, ...(summary.data?.recent ?? []), ...(theirs.data ?? [])]) known.set(s.id, s);
  const one = useStudy(current, current ? known.get(current) : undefined);
  const shown = (current && (known.get(current) ?? one.data)) || null;
  const owner = patient ? (theirs.data?.[0] ?? null) : null;

  // what each filter can pick, with how many studies have it (over every study, or the patient's)
  const facets = useMemo(() => {
    const f = choices.data ?? { modalities: {}, sexes: {}, dates: {} };
    const dates = Object.entries(f.dates);
    const all = Object.values(f.sexes).reduce((n, c) => n + c, 0);
    const extra = Object.keys(f.modalities).filter((m) => !IMAGING_MODALITIES.some(([c]) => c === m)).sort();
    return {
      all,
      // every imaging modality, then whatever else the archive holds (SEG, SR, ...)
      modalities: [...IMAGING_MODALITIES, ...extra.map((m) => [m, ""] as [string, string])].map(([m, name]) => ({ value: m, label: m, hint: name, count: f.modalities[m] ?? 0 })),
      ranges: Object.fromEntries(
        RANGES.map(([value]) => {
          const since = sinceFor(value);
          return [value, since ? dates.filter(([d]) => d >= ymd(since)).reduce((n, [, c]) => n + c, 0) : all];
        }),
      ) as Record<Range, number>,
      sexes: Object.entries(f.sexes).map(([x, n]) => ({ value: x, label: SEXES[x] ?? x, count: n })),
      dates: dates.map(([d]) => d),
      count: (s: Span) => dates.filter(([d]) => inSpan(d, s)).reduce((n, [, c]) => n + c, 0),
    };
  }, [choices.data]);

  const columns = useMemo<ColumnDef<Study>[]>(
    () => [
      {
        id: "select",
        header: ({ table }) => (
          <Checkbox
            aria-label="Select all shown"
            checked={table.getIsAllRowsSelected() || (table.getIsSomeRowsSelected() && "indeterminate")}
            onCheckedChange={(v) => table.toggleAllRowsSelected(!!v)}
          />
        ),
        cell: ({ row }) => (
          <Checkbox aria-label="Select study" checked={row.getIsSelected()} onCheckedChange={(v) => row.toggleSelected(!!v)} onClick={(e) => e.stopPropagation()} />
        ),
        enableSorting: false,
      },
      {
        id: "date",
        accessorFn: (s) => s.date + s.time,
        header: ({ column }) => <SortHeader column={column}>Date</SortHeader>,
        cell: ({ row: { original: s } }) => (
          <div className="tabular leading-tight whitespace-nowrap">
            <div>{formatDate(s.date)}</div>
            <div className="text-xs text-muted-foreground">{formatTime(s.time) || " "}</div>
          </div>
        ),
      },
      {
        id: "name",
        accessorFn: (s) => personName(s.name),
        header: ({ column }) => <SortHeader column={column}>Patient</SortHeader>,
        cell: ({ row: { original: s } }) => (
          <div className="min-w-0">
            <div className="truncate font-medium">{personName(s.name)}</div>
            <div className="truncate text-xs text-muted-foreground">{[s.patientId, sexAge(s.sex, s.birth, s.date)].filter(Boolean).join(" · ")}</div>
          </div>
        ),
      },
      {
        id: "description",
        accessorFn: (s) => s.description,
        header: ({ column }) => <SortHeader column={column}>Study</SortHeader>,
        cell: ({ row: { original: s } }) => (
          <div className="min-w-0">
            <div className="truncate">{s.description || <span className="text-muted-foreground">No description</span>}</div>
            <div className="truncate text-xs text-muted-foreground">{[s.accession && `Acc. ${s.accession}`, s.referring && `Ref. ${personName(s.referring)}`].filter(Boolean).join(" · ") || " "}</div>
          </div>
        ),
      },
      {
        id: "modality",
        accessorFn: (s) => s.modalities.join(" "),
        header: ({ column }) => <SortHeader column={column}>Modality</SortHeader>,
        cell: ({ row: { original: s } }) => <Modalities list={s.modalities} className="flex-nowrap overflow-hidden" />,
      },
      {
        id: "series",
        accessorFn: (s) => s.series,
        header: ({ column }) => (
          <div className="text-right">
            <SortHeader column={column}>Series</SortHeader>
          </div>
        ),
        cell: ({ row: { original: s } }) => <div className="tabular text-right">{s.series}</div>,
      },
      {
        id: "instances",
        accessorFn: (s) => s.instances,
        header: ({ column }) => (
          <div className="text-right">
            <SortHeader column={column}>Images</SortHeader>
          </div>
        ),
        cell: ({ row: { original: s } }) => <div className="tabular text-right">{s.instances.toLocaleString()}</div>,
      },
    ],
    [],
  );

  // the server sorts and pages; the table only lays the page out
  const table = useReactTable({
    data: rows,
    columns,
    getRowId: (s) => s.id,
    state: { sorting, rowSelection: selection, pagination },
    onSortingChange: setSorting,
    onRowSelectionChange: setSelection,
    onPaginationChange: setPagination,
    getCoreRowModel: getCoreRowModel(),
    manualSorting: true,
    manualPagination: true,
    rowCount: total,
  });

  const selected = Object.keys(selection).filter((id) => selection[id]).map((id) => known.get(id)).filter((x): x is Study => !!x);
  const filtered = !!q || modalities.length > 0 || sexes.length > 0 || range !== "any";
  const visible = table.getRowModel().rows;
  const pages = Math.max(1, Math.ceil(total / pageSize));
  // what the summary covers: the date range, and whether other filters narrow it
  const scope = (range === "custom" ? spanLabel(span) : RANGE_NAMES[range]) + (q || modalities.length || sexes.length ? ", filtered" : "");

  // a page reached by ↑ or ↓: its first or last row
  useEffect(() => {
    if (!edge || page.isPlaceholderData || !rows.length) return;
    const row = edge === "first" ? rows[0] : rows[rows.length - 1];
    setCurrent(row.id);
    setEdge(null);
    requestAnimationFrame(() => list.current?.querySelector(`[data-row="${row.id}"]`)?.scrollIntoView({ block: "nearest" }));
  }, [edge, page.data, page.isPlaceholderData]);

  // ↑ ↓ through the list, onto the next or previous page at its ends; Enter opens, space ticks
  function onListKey(e: React.KeyboardEvent) {
    if ((e.target as HTMLElement).closest("button, input, [role=checkbox]")) return;
    const at = visible.findIndex((r) => r.id === current);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const down = e.key === "ArrowDown";
      if (at < 0 && visible.length) return setCurrent(visible[down ? 0 : visible.length - 1].id);
      const to = at + (down ? 1 : -1);
      if (to >= visible.length && pageIndex < pages - 1) {
        setPagination((p) => ({ ...p, pageIndex: p.pageIndex + 1 }));
        setEdge("first");
      } else if (to < 0 && pageIndex > 0) {
        setPagination((p) => ({ ...p, pageIndex: p.pageIndex - 1 }));
        setEdge("last");
      } else if (visible[to]) {
        setCurrent(visible[to].id);
        list.current?.querySelector(`[data-row="${visible[to].id}"]`)?.scrollIntoView({ block: "nearest" });
      }
    } else if (e.key === "Enter" && current) openStudy(current);
    else if (e.key === " " && at >= 0) {
      e.preventDefault();
      visible[at].toggleSelected();
    } else if (e.key === "Escape") setCurrent(null);
  }

  async function confirmDelete() {
    if (!pending) return;
    const failed = await deleteStudies(pending.ids);
    setPending(null);
    setSelection((sel) => Object.fromEntries(Object.entries(sel).filter(([id]) => !pending.ids.includes(id))));
    if (current && pending.ids.includes(current)) setCurrent(null);
    if (failed.length) toast.error(`${pending.ids.length - failed.length} deleted, ${failed.length} not`, { description: failed[0] });
    else toast.success(pending.ids.length === 1 ? "Study deleted" : `${pending.ids.length} studies deleted`);
    await refresh();
  }

  return (
    <div className="flex h-full">
      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex items-center gap-4 px-6 pt-5 pb-3">
          <div className="mr-auto min-w-0">
            <h1 className="truncate text-lg font-semibold tracking-tight">{owner ? personName(owner.name) : "Studies"}</h1>
            <p className="text-sm text-muted-foreground">
              {page.isPending
                ? "Loading…"
                : owner
                  ? [owner.patientId, sexAge(owner.sex, owner.birth), plural(total, "study", "studies")].filter(Boolean).join(" · ")
                  : `${plural(total, "study", "studies")}${summary.data ? ` from ${plural(summary.data.patients, "patient")}` : ""}${filtered ? " match" : ""}`}
            </p>
          </div>
          <Button onClick={upload.open} title="Or drop DICOM files or folders anywhere on the page">
            <Upload /> Upload
          </Button>
        </div>

        {/* the filters, right above what they filter: when, then what and who */}
        <div className="flex flex-wrap items-center gap-2 px-6 pb-3">
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            value={range === "custom" ? "" : range}
            onValueChange={(v) => v && setRange(v as Range)}
            aria-label="Study date"
            className="bg-card"
          >
            {RANGES.map(([value, label]) => (
              <ToggleGroupItem key={value} value={value} className="h-8 gap-1.5 px-2.5 text-xs data-[state=on]:bg-selection-soft data-[state=on]:text-selection">
                {label}
                <span className="tabular text-[11px] opacity-60">{facets.ranges[value]}</span>
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
          <CustomRange
            active={range === "custom"}
            span={span}
            dates={facets.dates}
            count={facets.count}
            onApply={(s) => {
              setSpan(s);
              setRange(s.from || s.to ? "custom" : "any");
            }}
          />
          <Separator orientation="vertical" className="mx-1 !h-5" />
          <FacetedFilter title="Modality" options={facets.modalities} selected={modalities} onChange={setModalities} />
          <FacetedFilter title="Sex" options={facets.sexes} selected={sexes} onChange={setSexes} searchable={false} />
          {owner && (
            <Badge variant="secondary" className="h-8 gap-1.5 rounded-md pr-1 pl-2.5 text-sm font-normal">
              <User className="size-3.5" />
              {personName(owner.name)}
              <Button variant="ghost" size="icon-xs" aria-label="Show every patient" onClick={() => go(href.studies())}>
                <X />
              </Button>
            </Badge>
          )}
          {filtered && (
            <Button
              variant="ghost"
              size="sm"
              className="h-8 px-2 text-muted-foreground"
              title="Clear the search and every filter"
              onClick={() => {
                setSearch("studies", "");
                setQ("");
                setModalities([]);
                setSexes([]);
                setRange("any");
                setSpan({ from: "", to: "" });
              }}
            >
              Reset <X />
            </Button>
          )}
        </div>

        <div ref={list} tabIndex={0} onKeyDown={onListKey} aria-label="Studies" className="min-h-0 flex-1 overflow-auto border-t outline-none">
          <Table className="table-fixed">
            <colgroup>
              <col className="w-11" />
              <col className="w-32" />
              <col className="w-[26%]" />
              <col />
              <col className="w-24" />
              <col className="w-16" />
              <col className="w-20" />
            </colgroup>
            <TableHeader className="sticky top-0 z-10 bg-background shadow-[inset_0_-1px_0_var(--border)]">
              {table.getHeaderGroups().map((group) => (
                <TableRow key={group.id} className="border-0 hover:bg-transparent">
                  {group.headers.map((header) => (
                    <TableHead key={header.id} className="h-9 text-xs font-medium text-muted-foreground first:pl-6 last:pr-6">
                      {header.isPlaceholder ? null : flexRender(header.column.columnDef.header, header.getContext())}
                    </TableHead>
                  ))}
                </TableRow>
              ))}
            </TableHeader>
            <TableBody>
              {page.isPending &&
                Array.from({ length: 8 }, (_, i) => (
                  <TableRow key={i} className="h-12 hover:bg-transparent">
                    <TableCell colSpan={7} className="px-6">
                      <div className="flex items-center gap-6">
                        <Skeleton className="h-3 w-40" />
                        <Skeleton className="h-3 w-56" />
                        <Skeleton className="ml-auto h-3 w-20" />
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              {visible.map((row) => (
                <ContextMenu key={row.id}>
                  <ContextMenuTrigger asChild>
                    <TableRow
                      data-row={row.id}
                      data-state={row.getIsSelected() ? "selected" : undefined}
                      aria-current={row.id === current ? "true" : undefined}
                      className={cn(
                        "h-12 cursor-pointer",
                        row.id === current && "bg-selection-soft shadow-[inset_3px_0_0_var(--selection)] hover:bg-selection-soft data-[state=selected]:bg-selection-soft",
                      )}
                      // a second click on the chosen row lets it go
                      onClick={() => setCurrent((c) => (c === row.id ? null : row.id))}
                      onDoubleClick={() => openStudy(row.id)}
                    >
                      {row.getVisibleCells().map((cell) => (
                        <TableCell key={cell.id} className="py-1.5 first:pl-6 last:pr-6">
                          {flexRender(cell.column.columnDef.cell, cell.getContext())}
                        </TableCell>
                      ))}
                    </TableRow>
                  </ContextMenuTrigger>
                  <ContextMenuContent className="w-56">
                    <StudyMenu study={row.original} kind="context" onDelete={setPending} />
                  </ContextMenuContent>
                </ContextMenu>
              ))}
            </TableBody>
          </Table>

          {!page.isPending && !page.isPlaceholderData && rows.length === 0 && (
            <div className="flex flex-col items-center px-6 py-20 text-center">
              <div className="mb-4 grid size-11 place-items-center rounded-full border bg-card">
                <FileStack className="size-5 text-muted-foreground" strokeWidth={1.75} />
              </div>
              {facets.all === 0 && !patient ? (
                <>
                  <div className="font-medium">No studies yet</div>
                  <p className="mt-1 max-w-sm text-sm text-muted-foreground">Upload DICOM files or folders, or send them with STOW-RS or POST /instances.</p>
                  <Button className="mt-5" onClick={upload.open}>
                    <Upload /> Upload studies
                  </Button>
                </>
              ) : (
                <>
                  <div className="font-medium">No study matches</div>
                  <p className="mt-1 text-sm text-muted-foreground">Try another search, or reset the filters.</p>
                </>
              )}
            </div>
          )}
        </div>

        {total > 0 && (
          <Pager
            total={total}
            pageIndex={pageIndex}
            noun={["study", "studies"]}
            loading={page.isPlaceholderData}
            onPage={(i) => {
              setPagination((p) => ({ ...p, pageIndex: i }));
              list.current?.scrollTo({ top: 0 });
            }}
          />
        )}
      </section>

      <aside aria-label="The chosen study" className="hidden w-[400px] shrink-0 overflow-y-auto border-l bg-card lg:block">
        {shown ? (
          <StudyPane key={shown.id} study={shown} onPick={setCurrent} onClose={() => setCurrent(null)} onDelete={setPending} />
        ) : (
          <ArchiveSummary summary={summary.data} scope={scope} total={facets.all} onPick={setCurrent} />
        )}
      </aside>

      {selected.length > 0 && (
        <div className="fixed inset-x-0 bottom-6 z-40 flex justify-center px-4">
          <div className="flex items-center gap-1 rounded-xl border bg-popover p-1.5 pl-4 shadow-lg animate-in fade-in slide-in-from-bottom-2">
            <span className="text-sm font-medium">{plural(selected.length, "study", "studies")} selected</span>
            <span className="mr-2 ml-2 text-sm text-muted-foreground">{selected.reduce((n, s) => n + s.instances, 0).toLocaleString()} images</span>
            <Button variant="ghost" size="sm" asChild>
              <a href={url(`tools/create-archive?resources=${selected.map((s) => s.id).join(",")}`)} download>
                <Download /> Download ZIP
              </a>
            </Button>
            <Button
              variant="ghost"
              size="sm"
              className="text-destructive hover:text-destructive"
              onClick={() =>
                setPending({
                  ids: selected.map((s) => s.id),
                  title: `Delete ${plural(selected.length, "study", "studies")}?`,
                  description: `${selected.reduce((n, s) => n + s.instances, 0).toLocaleString()} images from ${plural(new Set(selected.map((s) => s.patient)).size, "patient")} will be removed from storage. This can't be undone.`,
                })
              }
            >
              <Trash2 /> Delete
            </Button>
            <Button variant="ghost" size="icon-sm" aria-label="Clear selection" onClick={() => setSelection({})}>
              <X />
            </Button>
          </div>
        </div>
      )}

      <ConfirmDelete pending={pending} onCancel={() => setPending(null)} onConfirm={confirmDelete} />
    </div>
  );
}

// a date range of one's own: shortcuts, two months to click the first and last day on (days
// with studies dotted), and both ends typed; how many studies it takes, as it is picked
const iso = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const fromIso = (s: string) => (s ? new Date(Number(s.slice(0, 4)), Number(s.slice(5, 7)) - 1, Number(s.slice(8, 10))) : undefined);
function shortcuts(): [string, Span][] {
  const t = new Date();
  const day = (y: number, m: number, d: number) => iso(new Date(y, m, d));
  const [y, m, d] = [t.getFullYear(), t.getMonth(), t.getDate()];
  const monday = d - ((t.getDay() + 6) % 7);
  return [
    ["Yesterday", { from: day(y, m, d - 1), to: day(y, m, d - 1) }],
    ["This week", { from: day(y, m, monday), to: iso(t) }],
    ["Last week", { from: day(y, m, monday - 7), to: day(y, m, monday - 1) }],
    ["This month", { from: day(y, m, 1), to: iso(t) }],
    ["Last month", { from: day(y, m - 1, 1), to: day(y, m, 0) }],
    ["Last 3 months", { from: day(y, m - 3, d), to: iso(t) }],
    ["This year", { from: day(y, 0, 1), to: iso(t) }],
    ["Last year", { from: day(y - 1, 0, 1), to: day(y - 1, 11, 31) }],
  ];
}

function CustomRange({ active, span, dates, count, onApply }: { active: boolean; span: Span; dates: string[]; count: (s: Span) => number; onApply: (s: Span) => void }) {
  const [open, setOpen] = useState(false);
  const [draft, setDraft] = useState(span);
  const today = new Date();
  // the ends in order, whichever was typed first
  const ordered = draft.from && draft.to && draft.from > draft.to ? { from: draft.to, to: draft.from } : draft;
  const studyDays = useMemo(() => dates.map((d) => parseDate(d)).filter((d): d is Date => !!d), [dates]);
  const first = studyDays.reduce<Date | null>((a, d) => (!a || d < a ? d : a), null);
  const apply = (s: Span) => {
    onApply(s);
    setOpen(false);
  };
  const picked = shortcuts().find(([, s]) => s.from === ordered.from && s.to === ordered.to)?.[0];
  return (
    <Popover
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (o) setDraft(span);
      }}
    >
      <PopoverTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          aria-label="Custom date range"
          className={cn("h-8 gap-1.5 bg-card px-3 text-xs font-medium", active && "border-selection/40 bg-selection-soft text-selection hover:bg-selection-soft hover:text-selection")}
        >
          <CalendarRange className="size-3.5" />
          {active ? spanLabel(span) : "Custom…"}
          {active && (
            <span
              role="button"
              aria-label="Clear the date range"
              className="-mr-1.5 ml-0.5 grid size-5 place-items-center rounded hover:bg-selection/15"
              onClick={(e) => {
                e.stopPropagation();
                onApply({ from: "", to: "" });
              }}
            >
              <X className="size-3" />
            </span>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-auto p-0">
        <form
          onSubmit={(e) => {
            e.preventDefault();
            apply(ordered);
          }}
        >
          <div className="flex">
            <div className="flex w-36 flex-col gap-0.5 border-r p-2">
              {shortcuts().map(([label, s]) => (
                <button
                  key={label}
                  type="button"
                  onClick={() => apply(s)}
                  className={cn("rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent", picked === label && "bg-selection-soft text-selection")}
                >
                  {label}
                </button>
              ))}
            </div>
            <Calendar
              mode="range"
              numberOfMonths={2}
              captionLayout="dropdown"
              showOutsideDays={false}
              defaultMonth={fromIso(ordered.from) ?? new Date(today.getFullYear(), today.getMonth() - 1, 1)}
              startMonth={new Date(Math.min(first?.getFullYear() ?? today.getFullYear(), today.getFullYear() - 10), 0, 1)}
              endMonth={today}
              disabled={{ after: today }}
              selected={{ from: fromIso(ordered.from), to: fromIso(ordered.to) }}
              onSelect={(r) => setDraft({ from: r?.from ? iso(r.from) : "", to: r?.to ? iso(r.to) : "" })}
              modifiers={{ studies: studyDays }}
              modifiersClassNames={{ studies: "has-studies" }}
              className="p-3"
            />
          </div>
          <div className="flex items-end gap-3 border-t p-3">
            {(["from", "to"] as const).map((end) => (
              <label key={end} className="grid gap-1 text-xs text-muted-foreground">
                {end === "from" ? "From" : "To"}
                <Input
                  type="date"
                  max={iso(today)}
                  value={draft[end]}
                  onChange={(e) => setDraft((d) => ({ ...d, [end]: e.target.value }))}
                  className="h-8 w-36 text-sm text-foreground"
                />
              </label>
            ))}
            <div className="mb-2 ml-1 min-w-0 flex-1 text-xs text-muted-foreground">
              {ordered.from || ordered.to ? (
                <>
                  <span className="font-medium text-foreground tabular">{plural(count(ordered), "study", "studies")}</span>
                  {ordered.from && ordered.to ? "" : ordered.from ? " since then" : " until then"}
                </>
              ) : (
                "Click the first day, then the last."
              )}
            </div>
            <Button type="button" variant="ghost" size="sm" onClick={() => apply({ from: "", to: "" })}>
              Clear
            </Button>
            <Button type="submit" size="sm" disabled={!ordered.from && !ordered.to}>
              Apply
            </Button>
          </div>
        </form>
      </PopoverContent>
    </Popover>
  );
}

function deleteOne(s: Study): PendingDelete {
  return {
    ids: [s.id],
    title: "Delete this study?",
    description: `${s.description || "The study"} of ${personName(s.name)} (${formatDate(s.date)}): ${plural(s.instances, "image")} will be removed from storage. This can't be undone.`,
  };
}

// the actions beyond Open and Download: in the pane's ⋯ menu and on a right click
function StudyMenu({ study: s, kind, onDelete }: { study: Study; kind: "dropdown" | "context"; onDelete: (p: PendingDelete) => void }) {
  // asked for when the menu opens
  const theirs = usePatientStudies(s.patient).data ?? [s];
  const [Item, Separator] = kind === "dropdown" ? [DropdownMenuItem, DropdownMenuSeparator] : [ContextMenuItem, ContextMenuSeparator];
  return (
    <>
      {kind === "context" && (
        <>
          <Item onSelect={() => openStudy(s.id)}>
            <Eye /> Open
          </Item>
          <Item asChild>
            <a href={url(`studies/${s.id}/archive`)} download>
              <Download /> Download ZIP
            </a>
          </Item>
        </>
      )}
      <Item onSelect={() => copy(s.uid, "Study Instance UID")}>
        <Copy /> Copy Study UID
      </Item>
      <Separator />
      <Item onSelect={() => go(href.studies(s.patient))}>
        <User /> All studies of this patient
      </Item>
      <Item asChild>
        <a href={url(`patients/${s.patient}/archive`)} download>
          <Download /> Download all of this patient
        </a>
      </Item>
      <Separator />
      <Item variant="destructive" onSelect={() => onDelete(deleteOne(s))}>
        <Trash2 /> Delete study…
      </Item>
      <Item
        variant="destructive"
        onSelect={() =>
          // a patient is their studies: deleting them all removes the patient
          onDelete({
            ids: theirs.map((x) => x.id),
            title: "Delete this patient?",
            description: `${personName(s.name)} (${s.patientId || "no ID"}) and all ${plural(theirs.length, "study", "studies")}, ${plural(theirs.reduce((n, x) => n + x.instances, 0), "image")}, will be removed from storage. This can't be undone.`,
          })
        }
      >
        <Trash2 /> Delete patient…
      </Item>
    </>
  );
}

function Section({ title, aside, children }: { title: string; aside?: ReactNode; children: ReactNode }) {
  return (
    <section className="border-t px-5 py-4">
      <div className="mb-2.5 flex items-baseline justify-between">
        <h3 className="text-xs font-medium text-muted-foreground">{title}</h3>
        {aside && <span className="text-xs text-muted-foreground">{aside}</span>}
      </div>
      {children}
    </section>
  );
}

// the chosen study: who, what, its series, the patient's other studies, and what can be done
function StudyPane({ study: s, onPick, onClose, onDelete }: { study: Study; onPick: (id: string) => void; onClose: () => void; onDelete: (p: PendingDelete) => void }) {
  const details = useStudySeries(s.id);
  const others = (usePatientStudies(s.patient).data ?? []).filter((x) => x.id !== s.id);
  const facts: [string, ReactNode][] = [
    ["Accession", s.accession || "—"],
    ["Study ID", s.studyId || "—"],
    ["Referring", s.referring ? personName(s.referring) : "—"],
    ["Stored", details.data ? bytes(details.data.stats.DiskSize) : "…"],
  ];
  return (
    <div className="flex min-h-full flex-col">
      <div className="flex items-center gap-3 px-5 pt-5 pb-4">
        <span className="grid size-10 shrink-0 place-items-center rounded-full bg-secondary text-sm font-medium text-muted-foreground ring-1 ring-border">{initials(s.name)}</span>
        <div className="min-w-0">
          <button className="block max-w-full truncate text-left font-semibold hover:underline" onClick={() => go(href.studies(s.patient))} title="All studies of this patient">
            {personName(s.name)}
          </button>
          <div className="truncate text-xs text-muted-foreground">
            {[s.patientId, sexAge(s.sex, s.birth, s.date), isoDate(s.birth) && `born ${formatDate(s.birth)}`].filter(Boolean).join(" · ")}
          </div>
        </div>
        <Button variant="ghost" size="icon-sm" aria-label="Close the study" title="Close (Esc)" className="ml-auto self-start text-muted-foreground" onClick={onClose}>
          <X />
        </Button>
      </div>

      <div className="border-t px-5 py-4">
        <div className="text-base leading-snug font-medium">{s.description || <span className="text-muted-foreground">No description</span>}</div>
        <div className="mt-1 flex items-center gap-2 text-sm text-muted-foreground">
          <Modalities list={s.modalities} />
          <span className="tabular">
            {formatDate(s.date)} {formatTime(s.time)}
          </span>
        </div>
        <div className="mt-4 flex gap-2">
          <Button className="flex-1" onClick={() => openStudy(s.id)}>
            <Eye /> Open
          </Button>
          <Button variant="outline" asChild>
            <a href={url(`studies/${s.id}/archive`)} download>
              <Download /> Download
            </a>
          </Button>
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="outline" size="icon" aria-label="More actions">
                <Ellipsis />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end" className="w-56">
              <StudyMenu study={s} kind="dropdown" onDelete={onDelete} />
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </div>

      <Section title="Series" aside={details.data ? `${plural(details.data.series.length, "series", "series")} · ${plural(s.instances, "image")}` : null}>
        <div className="grid grid-cols-3 gap-2.5">
          {details.isPending && Array.from({ length: Math.min(s.series || 3, 6) }, (_, i) => <Skeleton key={i} className="aspect-square rounded-md" />)}
          {details.data?.series.map((x: any) => (
            <button key={x.ID} className="group/thumb min-w-0 text-left" onClick={() => openStudy(s.id, x.ID)} title="Open this series">
              <SeriesThumb series={x} studyUid={s.uid} className="ring-selection ring-offset-2 ring-offset-card transition group-hover/thumb:ring-2" />
              <div className="mt-1 truncate text-xs font-medium">{x.MainDicomTags.SeriesDescription || "No description"}</div>
              <div className="truncate text-[11px] text-muted-foreground">
                {x.MainDicomTags.Modality} {x.MainDicomTags.SeriesNumber ? `#${x.MainDicomTags.SeriesNumber}` : ""} · {x.Instances.length}
              </div>
            </button>
          ))}
        </div>
      </Section>

      <Section title={others.length ? "Other studies of this patient" : "No other study of this patient"}>
        {others.length > 0 && (
          <div className="-mx-2 flex flex-col">
            {others.map((o) => (
              <button key={o.id} className="flex items-center gap-3 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent" onClick={() => onPick(o.id)}>
                <span className="w-11 shrink-0 text-[11px] font-medium text-muted-foreground uppercase">{o.date < s.date ? "Prior" : "Later"}</span>
                <span className="min-w-0 flex-1 truncate">{o.description || "No description"}</span>
                <Modalities list={o.modalities} className="flex-nowrap" />
                <span className="tabular shrink-0 text-xs text-muted-foreground">{formatDate(o.date)}</span>
              </button>
            ))}
          </div>
        )}
      </Section>

      <Section title="Details">
        <dl className="grid grid-cols-[88px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-sm">
          {facts.map(([k, v]) => (
            <Fragment key={k}>
              <dt className="text-muted-foreground">{k}</dt>
              <dd className="tabular truncate">{v}</dd>
            </Fragment>
          ))}
          <dt className="text-muted-foreground">Study UID</dt>
          <dd className="min-w-0">
            <button className="flex max-w-full items-center gap-1.5 text-left" onClick={() => copy(s.uid, "Study Instance UID")} title="Copy">
              <span className="truncate font-mono text-xs text-muted-foreground">{s.uid}</span>
              <Copy className="size-3.5 shrink-0 text-muted-foreground" />
            </button>
          </dd>
        </dl>
      </Section>
    </div>
  );
}

// before a study is chosen: what the list holds, all of its pages, from the server
function ArchiveSummary({ summary, scope, total, onPick }: { summary: StudySummary | undefined; scope: string; total: number; onPick: (id: string) => void }) {
  if (!summary) return <div className="space-y-3 p-5">{Array.from({ length: 4 }, (_, i) => <Skeleton key={i} className="h-10" />)}</div>;
  const byModality = summary.modalities;
  const recent = summary.recent;
  const top = byModality[0]?.[1] ?? 1;
  const numbers: [string, number][] = [
    ["Patients", summary.patients],
    ["Studies", summary.studies],
    ["Series", summary.series],
    ["Images", summary.instances],
  ];
  return (
    <div className="flex min-h-full flex-col">
      <div className="px-5 pt-5 pb-4">
        <div className="font-semibold">{scope}</div>
        <div className="text-xs text-muted-foreground">
          {summary.studies === total ? `Every study in the archive` : `${plural(summary.studies, "study", "studies")} of ${total.toLocaleString()} in the archive`}
        </div>
      </div>
      <section className="grid grid-cols-2 gap-px border-t bg-border">
        {numbers.map(([label, n]) => (
          <div key={label} className="bg-card px-5 py-4">
            <div className="tabular text-2xl font-semibold tracking-tight">{n.toLocaleString()}</div>
            <div className="text-xs text-muted-foreground">{label}</div>
          </div>
        ))}
      </section>
      {byModality.length > 0 && (
        <Section title="Studies by modality">
          <div className="space-y-1.5">
            {byModality.map(([m, n]) => (
              <div key={m} className="flex items-center gap-3 text-sm">
                <span className="w-10 font-mono text-xs text-muted-foreground">{m}</span>
                <span className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                  <span className="block h-full rounded-full bg-selection/70" style={{ width: `${(100 * n) / top}%` }} />
                </span>
                <span className="tabular w-8 text-right text-xs">{n}</span>
              </div>
            ))}
          </div>
        </Section>
      )}
      {recent.length > 0 && (
        <Section title="Last received">
          <div className="-mx-2 flex flex-col">
            {recent.map((s) => (
              <button key={s.id} onClick={() => onPick(s.id)} className="flex items-center gap-3 rounded-md px-2 py-1.5 text-left text-sm hover:bg-accent">
                <span className="min-w-0 flex-1 truncate">
                  {personName(s.name)} <span className="text-muted-foreground">· {s.description || "No description"}</span>
                </span>
                <Modalities list={s.modalities} className="flex-nowrap" />
              </button>
            ))}
          </div>
        </Section>
      )}
      <div className="mt-auto flex items-center gap-2.5 border-t px-5 py-4 text-xs text-muted-foreground">
        <MousePointerClick className="size-4" strokeWidth={1.75} />
        Choose a study to see its series and details.
      </div>
    </div>
  );
}
