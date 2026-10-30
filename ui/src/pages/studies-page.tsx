// The studies: a worklist on the left, the chosen study on the right. A click on a row shows
// the study in the pane (its patient, series, details, the patient's other studies and the
// actions, once); a double click or Enter opens it in the viewer; ↑ ↓ move through the list;
// checkboxes act on several studies at once; a right click has the actions too.

import { Fragment, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
import { ArrowDown, ArrowUp, ChevronLeft, ChevronRight, Copy, Download, Ellipsis, Eye, FileStack, MousePointerClick, Search, Trash2, Upload, User, X } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { ContextMenu, ContextMenuContent, ContextMenuItem, ContextMenuSeparator, ContextMenuTrigger } from "@/components/ui/context-menu";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { FacetedFilter } from "@/components/faceted-filter";
import { ConfirmDelete, type PendingDelete } from "@/components/confirm-delete";
import { SeriesThumb } from "@/components/series-thumb";
import { useUpload } from "@/components/upload";
import { IMAGING_MODALITIES } from "@/lib/modalities";
import { bytes, formatDate, formatTime, initials, isoDate, parseDate, personName, plural, sexAge, url } from "@/lib/api";
import { go, href } from "@/lib/route";
import { deleteStudies, useRefreshStudies, useStudies, useStudySeries, type Study } from "@/lib/studies";
import { cn } from "@/lib/utils";

const RANGES = [
  ["any", "All", null],
  ["today", "Today", 0],
  ["7d", "7 days", 7],
  ["30d", "30 days", 30],
  ["1y", "12 months", 365],
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

// a study opens as its hanging protocol lays it out; a series, on its own
function openStudy(id: string, seriesId?: string) {
  go(href.view(id, seriesId));
}

function copy(text: string, what: string) {
  navigator.clipboard?.writeText(text);
  toast(`${what} copied`);
}

function Modalities({ list, className }: { list: string[]; className?: string }) {
  return (
    <div className={cn("flex flex-wrap gap-1", className)}>
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
  const [current, setCurrent] = useState<string | null>(open);
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const list = useRef<HTMLDivElement>(null);
  useEffect(() => setCurrent(open), [open]);
  // the URL names the study shown, so a reload or a link comes back to it
  useEffect(() => {
    const to = href.studies(patient, current);
    if (location.hash !== to) history.replaceState(null, "", to);
  }, [patient, current]);

  const all = studies.data ?? [];
  const owner = patient ? all.find((s) => s.patient === patient) : null;
  const shown = all.find((s) => s.id === current) ?? null;

  const rows = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    const since = sinceFor(range);
    return all.filter((s) => {
      if (patient && s.patient !== patient) return false;
      if (modalities.length && !s.modalities.some((m) => modalities.includes(m))) return false;
      if (sexes.length && !sexes.includes(s.sex || "O")) return false;
      if (since && !(parseDate(s.date) && parseDate(s.date)! >= since)) return false;
      if (!words.length) return true;
      const text = `${s.name} ${personName(s.name)} ${s.patientId} ${s.accession} ${s.description} ${s.referring} ${s.modalities.join(" ")} ${s.date}`.toLowerCase();
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
      ranges: Object.fromEntries(
        RANGES.map(([value]) => {
          const since = sinceFor(value);
          return [value, since ? count((s) => !!parseDate(s.date) && parseDate(s.date)! >= since) : scope.length];
        }),
      ) as Record<Range, number>,
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
    initialState: { pagination: { pageSize: 100 } },
    autoResetPageIndex: true,
  });

  const selected = all.filter((s) => selection[s.id]);
  const filtered = !!query || modalities.length > 0 || sexes.length > 0 || range !== "any";
  const patients = new Set(rows.map((s) => s.patient)).size;
  const { pageIndex, pageSize } = table.getState().pagination;
  const visible = table.getRowModel().rows;

  // ↑ ↓ through the list, Enter opens, space ticks
  function onListKey(e: React.KeyboardEvent) {
    if ((e.target as HTMLElement).closest("button, input, [role=checkbox]")) return;
    const at = visible.findIndex((r) => r.id === current);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = visible[Math.max(0, Math.min(visible.length - 1, at + (e.key === "ArrowDown" ? 1 : -1)))];
      if (next) {
        setCurrent(next.id);
        list.current?.querySelector(`[data-row="${next.id}"]`)?.scrollIntoView({ block: "nearest" });
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
        <div className="flex flex-wrap items-center gap-x-6 gap-y-3 px-6 pt-5 pb-3">
          <div className="mr-auto min-w-0">
            <h1 className="truncate text-lg font-semibold tracking-tight">{owner ? personName(owner.name) : "Studies"}</h1>
            <p className="text-sm text-muted-foreground">
              {studies.isPending
                ? "Loading…"
                : owner
                  ? [owner.patientId, sexAge(owner.sex, owner.birth), plural(rows.length, "study", "studies")].filter(Boolean).join(" · ")
                  : `${plural(rows.length, "study", "studies")} from ${plural(patients, "patient")}${filtered ? " match" : ""}`}
            </p>
          </div>
          <ToggleGroup
            type="single"
            variant="outline"
            size="sm"
            value={range}
            onValueChange={(v) => v && setRange(v as Range)}
            aria-label="Study date"
            className="bg-card"
          >
            {RANGES.map(([value, label]) => (
              <ToggleGroupItem key={value} value={value} className="gap-1.5 px-3 text-xs data-[state=on]:bg-selection-soft data-[state=on]:text-selection">
                {label}
                <span className="tabular text-[11px] opacity-60">{facets.ranges[value]}</span>
              </ToggleGroupItem>
            ))}
          </ToggleGroup>
        </div>

        <div className="flex flex-wrap items-center gap-2 px-6 pb-3">
          <div className="relative w-full sm:w-72">
            <Search className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
            <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Name, ID, accession, description…" className="h-8 bg-card pl-8" />
          </div>
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

        <div ref={list} tabIndex={0} onKeyDown={onListKey} aria-label="Studies" className="min-h-0 flex-1 overflow-auto border-t outline-none">
          <Table className="table-fixed">
            <colgroup>
              <col className="w-11" />
              <col className="w-[26%]" />
              <col />
              <col className="w-24" />
              <col className="w-28" />
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
              {studies.isPending &&
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
                      onClick={() => setCurrent(row.id)}
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

          {!studies.isPending && rows.length === 0 && (
            <div className="flex flex-col items-center px-6 py-20 text-center">
              <div className="mb-4 grid size-11 place-items-center rounded-full border bg-card">
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
        </div>

        {rows.length > pageSize && (
          <div className="flex items-center justify-end gap-3 border-t px-6 py-2 text-sm text-muted-foreground">
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
      </section>

      <aside aria-label="The chosen study" className="hidden w-[400px] shrink-0 overflow-y-auto border-l bg-card lg:block">
        {shown ? (
          <StudyPane key={shown.id} study={shown} all={all} onPick={setCurrent} onDelete={setPending} />
        ) : (
          <ArchiveSummary studies={all} loading={studies.isPending} />
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

function deleteOne(s: Study): PendingDelete {
  return {
    ids: [s.id],
    title: "Delete this study?",
    description: `${s.description || "The study"} of ${personName(s.name)} (${formatDate(s.date)}): ${plural(s.instances, "image")} will be removed from storage. This can't be undone.`,
  };
}

// the actions beyond Open and Download: in the pane's ⋯ menu and on a right click
function StudyMenu({ study: s, kind, onDelete }: { study: Study; kind: "dropdown" | "context"; onDelete: (p: PendingDelete) => void }) {
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
      <Item onSelect={() => go(href.studies(s.patient))}>
        <User /> All studies of this patient
      </Item>
      <Separator />
      <Item variant="destructive" onSelect={() => onDelete(deleteOne(s))}>
        <Trash2 /> Delete study…
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
function StudyPane({ study: s, all, onPick, onDelete }: { study: Study; all: Study[]; onPick: (id: string) => void; onDelete: (p: PendingDelete) => void }) {
  const details = useStudySeries(s.id);
  const others = all.filter((x) => x.patient === s.patient && x.id !== s.id).sort((a, b) => (b.date + b.time).localeCompare(a.date + a.time));
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

// before a study is chosen: what the archive holds
function ArchiveSummary({ studies, loading }: { studies: Study[]; loading: boolean }) {
  const byModality = useMemo(() => {
    const counts = new Map<string, number>();
    for (const s of studies) for (const m of s.modalities) counts.set(m, (counts.get(m) ?? 0) + 1);
    return [...counts].sort((a, b) => b[1] - a[1]);
  }, [studies]);
  const recent = useMemo(() => [...studies].sort((a, b) => (b.updated || "").localeCompare(a.updated || "")).slice(0, 5), [studies]);
  const top = byModality[0]?.[1] ?? 1;
  const numbers: [string, number][] = [
    ["Patients", new Set(studies.map((s) => s.patient)).size],
    ["Studies", studies.length],
    ["Series", studies.reduce((n, s) => n + s.series, 0)],
    ["Images", studies.reduce((n, s) => n + s.instances, 0)],
  ];
  if (loading) return <div className="space-y-3 p-5">{Array.from({ length: 4 }, (_, i) => <Skeleton key={i} className="h-10" />)}</div>;
  return (
    <div className="flex min-h-full flex-col">
      <div className="flex items-center gap-2.5 px-5 pt-5 pb-4 text-sm text-muted-foreground">
        <MousePointerClick className="size-4" strokeWidth={1.75} />
        Choose a study to see its series and details.
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
              <a key={s.id} href={href.studies(null, s.id)} className="flex items-center gap-3 rounded-md px-2 py-1.5 text-sm hover:bg-accent">
                <span className="min-w-0 flex-1 truncate">
                  {personName(s.name)} <span className="text-muted-foreground">· {s.description || "No description"}</span>
                </span>
                <Modalities list={s.modalities} className="flex-nowrap" />
              </a>
            ))}
          </div>
        </Section>
      )}
    </div>
  );
}
