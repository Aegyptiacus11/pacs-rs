// The studies list: search and filters, a sortable table, a row opens the viewer, the chevron
// shows the series and details inline, ⋯ or a right click has the actions, checkboxes act on
// several studies at once.

import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";
import {
  flexRender,
  getCoreRowModel,
  getPaginationRowModel,
  getSortedRowModel,
  useReactTable,
  type ColumnDef,
  type RowSelectionState,
  type SortingState,
} from "@tanstack/react-table";
import {
  ArrowDown,
  ArrowUp,
  CalendarDays,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  Copy,
  Download,
  Eye,
  FileStack,
  Filter,
  Search,
  Trash2,
  Upload,
  User,
  X,
} from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuShortcut, ContextMenuTrigger } from "@/components/ui/context-menu";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { FacetedFilter } from "@/components/faceted-filter";
import { IMAGING_MODALITIES } from "@/lib/modalities";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ConfirmDelete, type PendingDelete } from "@/components/confirm-delete";
import { SeriesThumb } from "@/components/series-thumb";
import { useUpload } from "@/components/upload";
import { api, bytes, formatDate, formatTime, initials, isoDate, parseDate, personName, plural, sexAge, url } from "@/lib/api";
import { go, href } from "@/lib/route";
import { deleteStudies, sortSeries, useRefreshStudies, useStudies, useStudySeries, type Study } from "@/lib/studies";
import { cn } from "@/lib/utils";

const RANGES = [
  ["any", "Any time", null],
  ["today", "Today", 0],
  ["7d", "Last 7 days", 7],
  ["30d", "Last 30 days", 30],
  ["1y", "Last 12 months", 365],
] as const;
type Range = (typeof RANGES)[number][0];
const SEXES: Record<string, string> = { F: "Female", M: "Male", O: "Other" };

function sinceFor(range: Range) {
  const days = RANGES.find(([k]) => k === range)?.[2];
  if (days == null) return null;
  const since = new Date();
  since.setHours(0, 0, 0, 0);
  since.setDate(since.getDate() - days);
  return since;
}

async function openStudy(id: string, seriesId?: string) {
  if (seriesId) return go(href.view(id, seriesId));
  // the series with the most images: the one to read, rather than a localizer
  const series = sortSeries(await api<any[]>(`studies/${id}/series`));
  const main = series.reduce((best, s) => (s.Instances.length > (best?.Instances.length ?? -1) ? s : best), null as any);
  if (main) go(href.view(id, main.ID));
  else toast("This study has no series");
}

function copy(text: string, what: string) {
  navigator.clipboard?.writeText(text);
  toast(`${what} copied`);
}

function Avatar({ name }: { name: string }) {
  return (
    <span className="grid size-8 shrink-0 place-items-center rounded-full bg-secondary text-xs font-medium text-muted-foreground ring-1 ring-border">
      {initials(name)}
    </span>
  );
}

function Modalities({ list }: { list: string[] }) {
  return (
    <div className="flex flex-wrap gap-1">
      {list.map((m) => (
        <Badge key={m} variant="outline" className="h-5 rounded px-1.5 font-mono text-[11px] font-medium text-muted-foreground">
          {m}
        </Badge>
      ))}
    </div>
  );
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

export function StudiesPage({ patient, open }: { patient: string | null; open: string | null }) {
  const studies = useStudies();
  const refresh = useRefreshStudies();
  const upload = useUpload();
  const [query, setQuery] = useState("");
  const [modalities, setModalities] = useState<string[]>([]);
  const [range, setRange] = useState<Range>("any");
  const [sexes, setSexes] = useState<string[]>([]);
  const [sorting, setSorting] = useState<SortingState>([{ id: "date", desc: true }]);
  const [selection, setSelection] = useState<RowSelectionState>({});
  const [expanded, setExpanded] = useState<string | null>(open);
  const [pending, setPending] = useState<PendingDelete | null>(null);
  useEffect(() => setExpanded(open), [open]);

  const all = studies.data ?? [];
  const owner = patient ? all.find((s) => s.patient === patient) : null;

  const rows = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    const since = sinceFor(range);
    return all.filter((s) => {
      if (patient && s.patient !== patient) return false;
      if (modalities.length && !s.modalities.some((m) => modalities.includes(m))) return false;
      if (sexes.length && !sexes.includes(s.sex || "O")) return false;
      if (since && !(parseDate(s.date) && parseDate(s.date)! >= since)) return false;
      if (!words.length) return true;
      const text = `${s.name} ${personName(s.name)} ${s.patientId} ${s.accession} ${s.description} ${s.modalities.join(" ")} ${s.date}`.toLowerCase();
      return words.every((w) => text.includes(w));
    });
  }, [all, query, modalities, sexes, range, patient]);

  // what each filter can pick, with how many studies have it
  const facets = useMemo(() => {
    const scope = patient ? all.filter((s) => s.patient === patient) : all;
    const count = (pred: (s: Study) => boolean) => scope.filter(pred).length;
    const modalityList = [...new Set(scope.flatMap((s) => s.modalities))].sort();
    const sexList = [...new Set(scope.map((s) => s.sex || "O"))].sort();
    return {
      // every imaging modality, then whatever else the archive holds (SEG, SR, ...)
      modalities: [...IMAGING_MODALITIES, ...modalityList.filter((m) => !IMAGING_MODALITIES.some(([c]) => c === m)).map((m) => [m, ""] as [string, string])].map(([m, name]) => ({
        value: m,
        label: m,
        hint: name,
        count: count((s) => s.modalities.includes(m)),
      })),
      ranges: RANGES.slice(1).map(([value, label]) => {
        const since = sinceFor(value)!;
        return { value, label, count: count((s) => !!parseDate(s.date) && parseDate(s.date)! >= since) };
      }),
      sexes: sexList.map((x) => ({ value: x, label: SEXES[x] ?? x, count: count((s) => (s.sex || "O") === x) })),
    };
  }, [all, patient]);

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
        id: "expand",
        header: () => null,
        cell: ({ row }) => (
          <Button
            variant="ghost"
            size="icon-xs"
            aria-label={expanded === row.id ? "Hide details" : "Show details"}
            aria-expanded={expanded === row.id}
            onClick={(e) => {
              e.stopPropagation();
              setExpanded((x) => (x === row.id ? null : row.id));
            }}
          >
            <ChevronRight className={cn("transition-transform", expanded === row.id && "rotate-90")} />
          </Button>
        ),
      },
      {
        id: "name",
        accessorFn: (s) => personName(s.name),
        header: ({ column }) => <SortHeader column={column}>Patient</SortHeader>,
        cell: ({ row: { original: s } }) => (
          <div className="flex min-w-0 items-center gap-3">
            <Avatar name={s.name} />
            <div className="min-w-0">
              <div className="truncate font-medium">{personName(s.name)}</div>
              <div className="truncate text-xs text-muted-foreground">
                {[s.patientId, sexAge(s.sex, s.birth, s.date)].filter(Boolean).join(" · ")}
              </div>
            </div>
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
            <div className="mt-0.5 flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
              <Modalities list={s.modalities} />
              {s.accession && <span className="truncate">Accession {s.accession}</span>}
            </div>
          </div>
        ),
      },
      {
        id: "date",
        accessorFn: (s) => s.date + s.time,
        header: ({ column }) => <SortHeader column={column}>Date</SortHeader>,
        cell: ({ row: { original: s } }) => (
          <div className="tabular whitespace-nowrap">
            {formatDate(s.date)}
            <span className="ml-2 text-muted-foreground">{formatTime(s.time)}</span>
          </div>
        ),
      },
      {
        id: "instances",
        accessorFn: (s) => s.instances,
        header: ({ column }) => (
          <div className="text-right">
            <SortHeader column={column}>Images</SortHeader>
          </div>
        ),
        cell: ({ row: { original: s } }) => (
          <div className="tabular text-right whitespace-nowrap">
            {s.instances.toLocaleString()}
            <span className="ml-1.5 text-xs text-muted-foreground">{plural(s.series, "series", "series")}</span>
          </div>
        ),
      },
      {
        id: "actions",
        header: () => <span className="sr-only">Actions</span>,
        // the actions in plain sight: view, download, delete
        cell: ({ row: { original: s } }) => (
          <div className="flex items-center justify-end gap-0.5" onClick={(e) => e.stopPropagation()}>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="ghost" size="icon-sm" aria-label="View" className="text-muted-foreground hover:text-foreground" onClick={() => openStudy(s.id)}>
                  <Eye />
                </Button>
              </TooltipTrigger>
              <TooltipContent>View</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button variant="ghost" size="icon-sm" asChild className="text-muted-foreground hover:text-foreground">
                  <a href={url(`studies/${s.id}/archive`)} download aria-label="Download ZIP">
                    <Download />
                  </a>
                </Button>
              </TooltipTrigger>
              <TooltipContent>Download ZIP</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon-sm"
                  aria-label="Delete study"
                  className="text-muted-foreground hover:bg-destructive/10 hover:text-destructive"
                  onClick={() => setPending(deleteOne(s))}
                >
                  <Trash2 />
                </Button>
              </TooltipTrigger>
              <TooltipContent>Delete</TooltipContent>
            </Tooltip>
          </div>
        ),
      },
    ],
    [expanded],
  );

  const table = useReactTable({
    data: rows,
    columns,
    getRowId: (s) => s.id,
    state: { sorting, rowSelection: selection },
    onSortingChange: setSorting,
    onRowSelectionChange: setSelection,
    getCoreRowModel: getCoreRowModel(),
    getSortedRowModel: getSortedRowModel(),
    getPaginationRowModel: getPaginationRowModel(),
    initialState: { pagination: { pageSize: 50 } },
    autoResetPageIndex: true,
  });

  const selected = all.filter((s) => selection[s.id]);
  const filtered = !!query || modalities.length > 0 || sexes.length > 0 || range !== "any";
  const patients = new Set(rows.map((s) => s.patient)).size;
  const { pageIndex, pageSize } = table.getState().pagination;

  async function confirmDelete() {
    if (!pending) return;
    const failed = await deleteStudies(pending.ids);
    setPending(null);
    setSelection((sel) => Object.fromEntries(Object.entries(sel).filter(([id]) => !pending.ids.includes(id))));
    if (failed.length) toast.error(`${pending.ids.length - failed.length} deleted, ${failed.length} not`, { description: failed[0] });
    else toast.success(pending.ids.length === 1 ? "Study deleted" : `${pending.ids.length} studies deleted`);
    await refresh();
  }

  return (
    <div className="mx-auto max-w-[1440px] px-6 py-8">
      <div className="mb-6 flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">{owner ? personName(owner.name) : "Studies"}</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            {studies.isPending
              ? "Loading…"
              : owner
                ? [owner.patientId, sexAge(owner.sex, owner.birth), plural(rows.length, "study", "studies")].filter(Boolean).join(" · ")
                : `${plural(rows.length, "study", "studies")} from ${plural(patients, "patient")}${filtered ? " match the filters" : ""}`}
          </p>
        </div>
      </div>

      <div className="mb-4 flex flex-wrap items-center gap-2">
        <div className="relative w-full sm:w-72">
          <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search studies…" className="h-8 bg-card pl-8" />
        </div>
        <FacetedFilter title="Modality" options={facets.modalities} selected={modalities} onChange={setModalities} />
        <FacetedFilter title="Date" options={facets.ranges} selected={range === "any" ? [] : [range]} onChange={(v) => setRange((v[0] as Range) ?? "any")} single searchable={false} />
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
            className="h-8 px-2 lg:px-3"
            onClick={() => {
              setQuery("");
              setModalities([]);
              setSexes([]);
              setRange("any");
            }}
          >
            Reset <X />
          </Button>
        )}
      </div>

      <div className="overflow-hidden rounded-lg border bg-card">
        <Table className="table-fixed">
          <colgroup>
            <col className="w-10" />
            <col className="w-10" />
            <col className="w-[24%]" />
            <col />
            <col className="w-36" />
            <col className="w-28" />
            <col className="w-32" />
          </colgroup>
          <TableHeader>
            {table.getHeaderGroups().map((group) => (
              <TableRow key={group.id} className="hover:bg-transparent">
                {group.headers.map((header) => (
                  <TableHead key={header.id} className="h-10 text-xs font-medium text-muted-foreground">
                    {header.isPlaceholder ? null : flexRender(header.column.columnDef.header, header.getContext())}
                  </TableHead>
                ))}
              </TableRow>
            ))}
          </TableHeader>
          <TableBody>
            {studies.isPending &&
              Array.from({ length: 6 }, (_, i) => (
                <TableRow key={i} className="h-14 hover:bg-transparent">
                  <TableCell colSpan={7}>
                    <div className="flex items-center gap-3">
                      <Skeleton className="size-8 rounded-full" />
                      <Skeleton className="h-3 w-48" />
                      <Skeleton className="ml-auto h-3 w-24" />
                    </div>
                  </TableCell>
                </TableRow>
              ))}
            {table.getRowModel().rows.map((row) => (
              <Fragment key={row.id}>
                <ContextMenu>
                  <ContextMenuTrigger asChild>
                    <TableRow
                      data-state={row.getIsSelected() ? "selected" : undefined}
                      className={cn("group h-14 cursor-pointer", expanded === row.id && "border-b-0 bg-muted/40")}
                      tabIndex={0}
                      aria-expanded={expanded === row.id}
                      onClick={() => setExpanded((x) => (x === row.id ? null : row.id))}
                      onKeyDown={(e) => {
                        if (e.target !== e.currentTarget) return;
                        if (e.key === "Enter") setExpanded((x) => (x === row.id ? null : row.id));
                        if (e.key === " ") {
                          e.preventDefault();
                          row.toggleSelected();
                        }
                        if (e.key === "ArrowRight") setExpanded(row.id);
                        if (e.key === "ArrowLeft") setExpanded(null);
                      }}
                    >
                      {row.getVisibleCells().map((cell) => (
                        <TableCell key={cell.id} className="py-2">
                          {flexRender(cell.column.columnDef.cell, cell.getContext())}
                        </TableCell>
                      ))}
                    </TableRow>
                  </ContextMenuTrigger>
                  <ContextMenuContent className="w-56">
                    <StudyActions study={row.original} kind="context" onExpand={() => setExpanded(row.id)} onDelete={setPending} />
                  </ContextMenuContent>
                </ContextMenu>
                {expanded === row.id && (
                  <TableRow className="bg-muted/40 hover:bg-muted/40">
                    <TableCell colSpan={7} className="p-0">
                      <StudyDetails study={row.original} onDelete={setPending} />
                    </TableCell>
                  </TableRow>
                )}
              </Fragment>
            ))}
          </TableBody>
        </Table>

        {!studies.isPending && rows.length === 0 && (
          <div className="flex flex-col items-center px-6 py-20 text-center">
            <div className="mb-4 grid size-11 place-items-center rounded-full border bg-background">
              <FileStack className="size-5 text-muted-foreground" strokeWidth={1.75} />
            </div>
            {all.length === 0 ? (
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

        {rows.length > pageSize && (
          <div className="flex items-center justify-end gap-3 border-t px-4 py-2.5 text-sm text-muted-foreground">
            <span className="tabular">
              {pageIndex * pageSize + 1}–{Math.min(rows.length, (pageIndex + 1) * pageSize)} of {rows.length.toLocaleString()}
            </span>
            <Button variant="outline" size="icon-sm" aria-label="Previous page" disabled={!table.getCanPreviousPage()} onClick={() => table.previousPage()}>
              <ChevronLeft />
            </Button>
            <Button variant="outline" size="icon-sm" aria-label="Next page" disabled={!table.getCanNextPage()} onClick={() => table.nextPage()}>
              <ChevronRight />
            </Button>
          </div>
        )}
      </div>

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

function deleteOne(s: Study): PendingDelete {
  return {
    ids: [s.id],
    title: "Delete this study?",
    description: `${s.description || "The study"} of ${personName(s.name)} (${formatDate(s.date)}): ${plural(s.instances, "image")} will be removed from storage. This can't be undone.`,
  };
}

function StudyActions({ study: s, kind, onExpand, onDelete }: { study: Study; kind: "dropdown" | "context"; onExpand: () => void; onDelete: (p: PendingDelete) => void }) {
  const [Item, Separator, Shortcut] = kind === "dropdown" ? [DropdownMenuItem, DropdownMenuSeparator, DropdownMenuShortcut] : [ContextMenuItem, ContextMenuSeparator, ContextMenuShortcut];
  return (
    <>
      <Item onSelect={() => openStudy(s.id)}>
        <Eye /> View
      </Item>
      <Item onSelect={onExpand}>
        <ChevronRight /> Show details <Shortcut>↵</Shortcut>
      </Item>
      <Item asChild>
        <a href={url(`studies/${s.id}/archive`)} download>
          <Download /> Download ZIP
        </a>
      </Item>
      <Item onSelect={() => copy(s.uid, "Study Instance UID")}>
        <Copy /> Copy Study UID
      </Item>
      <Item onSelect={() => go(href.studies(s.patient))}>
        <User /> All studies of this patient
      </Item>
      <Separator />
      <Item
        variant="destructive"
        onSelect={() => onDelete(deleteOne(s))}
      >
        <Trash2 /> Delete study…
      </Item>
    </>
  );
}

function StudyDetails({ study: s, onDelete }: { study: Study; onDelete: (p: PendingDelete) => void }) {
  const details = useStudySeries(s.id);
  const facts: [string, ReactNode][] = [
    ["Study date", `${formatDate(s.date)} ${formatTime(s.time)}`],
    ["Accession", s.accession || "—"],
    ["Study ID", s.studyId || "—"],
    ["Referring", s.referring ? personName(s.referring) : "—"],
    ["Born", isoDate(s.birth) ? formatDate(s.birth) : "—"],
    ["Stored", details.data ? `${bytes(details.data.stats.DiskSize)} · ${plural(details.data.stats.CountInstances, "image")}` : "…"],
  ];
  return (
    <div className="grid gap-6 px-[5.5rem] pt-1 pb-5 lg:grid-cols-[minmax(0,1fr)_320px]">
      <div>
        <div className="mb-2 text-xs font-medium text-muted-foreground">Series</div>
        <div className="grid grid-cols-[repeat(auto-fill,minmax(112px,1fr))] gap-3">
          {details.isPending &&
            Array.from({ length: Math.min(s.series || 2, 6) }, (_, i) => <Skeleton key={i} className="aspect-square rounded-md" />)}
          {details.data?.series.map((x: any) => (
            <button key={x.ID} className="group/thumb text-left" onClick={() => openStudy(s.id, x.ID)}>
              <SeriesThumb series={x} studyUid={s.uid} className="ring-selection ring-offset-2 ring-offset-background transition group-hover/thumb:ring-2" />
              <div className="mt-1.5 truncate text-xs font-medium">{x.MainDicomTags.SeriesDescription || "No description"}</div>
              <div className="text-xs text-muted-foreground">
                {x.MainDicomTags.Modality} {x.MainDicomTags.SeriesNumber ? `#${x.MainDicomTags.SeriesNumber}` : ""} · {x.Instances.length}
              </div>
            </button>
          ))}
        </div>
      </div>
      <div>
        <dl className="grid grid-cols-[100px_minmax(0,1fr)] gap-x-3 gap-y-1.5 text-sm">
          {facts.map(([k, v]) => (
            <Fragment key={k}>
              <dt className="text-muted-foreground">{k}</dt>
              <dd className="tabular truncate">{v}</dd>
            </Fragment>
          ))}
          <dt className="text-muted-foreground">Study UID</dt>
          <dd className="min-w-0">
            <button className="flex max-w-full items-center gap-1.5 text-left hover:text-foreground" onClick={() => copy(s.uid, "Study Instance UID")} title="Copy">
              <span className="truncate font-mono text-xs text-muted-foreground">{s.uid}</span>
              <Copy className="size-3.5 shrink-0 text-muted-foreground" />
            </button>
          </dd>
        </dl>
        <div className="mt-4 flex flex-wrap gap-2">
          <Button size="sm" onClick={() => openStudy(s.id)}>
            <Eye /> View
          </Button>
          <Button size="sm" variant="outline" asChild>
            <a href={url(`studies/${s.id}/archive`)} download>
              <Download /> Download
            </a>
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="ml-auto text-muted-foreground hover:text-destructive"
            onClick={() => onDelete(deleteOne(s))}
          >
            <Trash2 /> Delete
          </Button>
        </div>
      </div>
    </div>
  );
}
