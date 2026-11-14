// The patients: a list on the left, the chosen patient's record on the right: who they are,
// what the archive holds of them, and a timeline of their studies, newest first, each with its
// series; two studies ticked open side by side in the viewer. The server pages the list
// (/ui/api/patients).

import { useEffect, useRef, useState, type ReactNode } from "react";
import { ArrowDownUp, Check, Columns2, Download, Ellipsis, Eye, FileStack, Trash2, Users, X } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { ConfirmDelete, type PendingDelete } from "@/components/confirm-delete";
import { may } from "@/lib/session";
import { DownloadLink } from "@/components/download-link";
import { Modalities } from "@/components/modalities";
import { PAGE_SIZE, Pager } from "@/components/pager";
import { FilterBox } from "@/components/search-bar";
import { SeriesThumb } from "@/components/series-thumb";
import { formatDate, formatTime, initials, isoDate, personName, plural, sexAge } from "@/lib/api";
import { go, href } from "@/lib/route";
import { deleteStudies, usePatientPage, usePatientStudies, useRefreshStudies, useStudySeries, type Patient, type PatientQuery, type Study } from "@/lib/studies";
import { cn } from "@/lib/utils";

const SORTS: [PatientQuery["sort"], boolean, string][] = [
  ["last", true, "Latest study first"],
  ["name", false, "Name, A to Z"],
  ["studies", true, "Most studies first"],
];

export function PatientsPage({ open }: { open: string | null }) {
  const refresh = useRefreshStudies();
  const [typed, setTyped] = useState("");
  const [q, setQ] = useState("");
  const [sort, setSort] = useState(0);
  const [pagination, setPagination] = useState({ pageIndex: 0, pageSize: PAGE_SIZE });
  const [current, setCurrent] = useState<string | null>(open);
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
  useEffect(() => {
    const to = href.patients(current);
    if (location.hash !== to) history.replaceState(null, "", to);
  }, [current]);
  useEffect(() => {
    const t = setTimeout(() => setQ(typed), 250);
    return () => clearTimeout(t);
  }, [typed]);
  useEffect(() => setPagination((p) => (p.pageIndex ? { ...p, pageIndex: 0 } : p)), [q, sort]);
  const { pageIndex, pageSize } = pagination;
  const [key, desc] = SORTS[sort];
  const page = usePatientPage({ q, sort: key, desc, offset: pageIndex * pageSize, limit: pageSize });
  const rows = page.data?.patients ?? [];
  const total = page.data?.total ?? 0;
  const known = useRef(new Map<string, Patient>()).current;
  for (const p of rows) known.set(p.id, p);

  // ↑ ↓ through the list, Enter opens the latest study
  function onListKey(e: React.KeyboardEvent) {
    const at = rows.findIndex((p) => p.id === current);
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const next = rows[Math.max(0, Math.min(rows.length - 1, at + (e.key === "ArrowDown" ? 1 : at < 0 ? rows.length : -1)))];
      if (next) {
        setCurrent(next.id);
        list.current?.querySelector(`[data-row="${next.id}"]`)?.scrollIntoView({ block: "nearest" });
      }
    } else if (e.key === "Escape") setCurrent(null);
  }

  return (
    <div className="flex h-full">
      <section className="flex w-[26rem] shrink-0 flex-col border-r">
        <div className="px-5 pt-5 pb-3">
          <div className="flex items-end justify-between gap-3">
            <div>
              <h1 className="text-lg font-semibold tracking-tight">Patients</h1>
              <p className="text-sm text-muted-foreground">{page.isPending ? "Loading…" : `${plural(total, "patient")}${q ? " match" : ""}`}</p>
            </div>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button variant="ghost" size="sm" className="h-8 gap-1.5 text-muted-foreground">
                  <ArrowDownUp className="size-3.5" /> {SORTS[sort][2]}
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-48">
                <DropdownMenuLabel>Order</DropdownMenuLabel>
                {SORTS.map(([, , label], i) => (
                  <DropdownMenuItem key={label} onSelect={() => setSort(i)}>
                    <Check className={cn(sort !== i && "opacity-0")} /> {label}
                  </DropdownMenuItem>
                ))}
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
          <FilterBox value={typed} onChange={setTyped} placeholder="Filter: name or patient ID…" className="mt-3" />
        </div>
        <div ref={list} tabIndex={0} onKeyDown={onListKey} aria-label="Patients" className="min-h-0 flex-1 overflow-y-auto border-t outline-none">
          {page.isPending &&
            Array.from({ length: 8 }, (_, i) => (
              <div key={i} className="flex items-center gap-3 border-b px-5 py-3">
                <Skeleton className="size-9 rounded-full" />
                <Skeleton className="h-3 w-40" />
              </div>
            ))}
          {rows.map((p) => (
            <button
              key={p.id}
              data-row={p.id}
              aria-current={p.id === current ? "true" : undefined}
              // a second click on the chosen patient lets them go
              onClick={() => setCurrent((c) => (c === p.id ? null : p.id))}
              className={cn(
                "flex w-full items-center gap-3 border-b px-5 py-2.5 text-left hover:bg-muted/50",
                p.id === current && "bg-selection-soft shadow-[inset_3px_0_0_var(--selection)] hover:bg-selection-soft",
              )}
            >
              <span className="grid size-9 shrink-0 place-items-center rounded-full bg-secondary text-xs font-medium text-muted-foreground ring-1 ring-border">{initials(p.name)}</span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-medium">{personName(p.name)}</span>
                <span className="block truncate text-xs text-muted-foreground">{[p.patientId, sexAge(p.sex, p.birth)].filter(Boolean).join(" · ")}</span>
              </span>
              <span className="shrink-0 text-right">
                <span className="tabular block text-xs">{formatDate(p.last)}</span>
                <span className="block text-[11px] text-muted-foreground">{plural(p.studies, "study", "studies")}</span>
              </span>
            </button>
          ))}
          {!page.isPending && !page.isPlaceholderData && rows.length === 0 && (
            <div className="flex flex-col items-center px-6 py-16 text-center">
              <div className="mb-4 grid size-11 place-items-center rounded-full border bg-card">
                <Users className="size-5 text-muted-foreground" strokeWidth={1.75} />
              </div>
              <div className="font-medium">{q ? "No patient matches" : "No patients yet"}</div>
            </div>
          )}
        </div>
        {total > 0 && (
          <Pager
            total={total}
            pageIndex={pageIndex}
            pageSize={pageSize}
            noun={["patient", "patients"]}
            loading={page.isPlaceholderData}
            onPage={(i) => {
              setPagination((p) => ({ ...p, pageIndex: i }));
              list.current?.scrollTo({ top: 0 });
            }}
            onPageSize={(size) => {
              setPagination({ pageIndex: 0, pageSize: size });
              list.current?.scrollTo({ top: 0 });
            }}
          />
        )}
      </section>

      <main aria-label="The chosen patient" className="min-w-0 flex-1 overflow-y-auto">
        {current ? (
          <PatientRecord
            onClose={() => setCurrent(null)}
            key={current}
            id={current}
            known={known.get(current)}
            onDeleted={() => {
              setCurrent(null);
              refresh();
            }}
          />
        ) : (
          <div className="grid h-full place-items-center p-10 text-center">
            <div>
              <div className="mx-auto mb-4 grid size-12 place-items-center rounded-full border bg-card">
                <Users className="size-5 text-muted-foreground" strokeWidth={1.75} />
              </div>
              <div className="font-medium">Choose a patient</div>
              <p className="mt-1 max-w-xs text-sm text-muted-foreground">Their record shows every study they have, newest first, with the series of each; tick two to compare them side by side.</p>
            </div>
          </div>
        )}
      </main>
    </div>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="min-w-0 px-5 py-3">
      <div className="tabular truncate text-lg font-semibold tracking-tight">{children}</div>
      <div className="text-xs text-muted-foreground">{label}</div>
    </div>
  );
}

// a patient's record: who, totals, the studies as a timeline
function PatientRecord({ id, known, onClose, onDeleted }: { id: string; known: Patient | undefined; onClose: () => void; onDeleted: () => void }) {
  const studies = usePatientStudies(id);
  const list = studies.data ?? [];
  const [compare, setCompare] = useState<string[]>([]);
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const s0 = list[0];
  const who = known ?? (s0 && { name: s0.name, patientId: s0.patientId, sex: s0.sex, birth: s0.birth, modalities: [...new Set(list.flatMap((s) => s.modalities))].sort() });
  const instances = list.reduce((n, s) => n + s.instances, 0);
  const series = list.reduce((n, s) => n + s.series, 0);
  const picked = compare.map((c) => list.find((s) => s.id === c)).filter((s): s is Study => !!s).sort((a, b) => (b.date + b.time).localeCompare(a.date + a.time));
  // two at most: a third replaces the first ticked
  const tick = (sid: string, on: boolean) => setCompare((c) => (on ? [...c.filter((x) => x !== sid), sid].slice(-2) : c.filter((x) => x !== sid)));

  if (!who) return <div className="space-y-4 p-8">{Array.from({ length: 4 }, (_, i) => <Skeleton key={i} className="h-16" />)}</div>;
  return (
    <div className="flex min-h-full flex-col">
      <header className="flex flex-wrap items-center gap-4 border-b bg-card px-8 py-6">
        <span className="grid size-14 shrink-0 place-items-center rounded-full bg-secondary text-lg font-medium text-muted-foreground ring-1 ring-border">{initials(who.name)}</span>
        <div className="mr-auto min-w-0">
          <h2 className="truncate text-xl font-semibold tracking-tight">{personName(who.name)}</h2>
          <div className="truncate text-sm text-muted-foreground">
            {[who.patientId, sexAge(who.sex, who.birth), isoDate(who.birth) && `born ${formatDate(who.birth)}`].filter(Boolean).join(" · ")}
          </div>
        </div>
        <Button variant="outline" onClick={() => go(href.studies(id))}>
          <FileStack /> In the study list
        </Button>
        <Button variant="outline" asChild>
          <DownloadLink path={`patients/${id}/archive`}>
            <Download /> Download all
          </DownloadLink>
        </Button>
        {may("delete") && <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="outline" size="icon" aria-label="More actions">
              <Ellipsis />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-52">
            <DropdownMenuItem
              variant="destructive"
              disabled={!list.length}
              onSelect={() =>
                setPending({
                  ids: list.map((s) => s.id),
                  title: "Delete this patient?",
                  description: `${personName(who.name)} (${who.patientId || "no ID"}) and all ${plural(list.length, "study", "studies")}, ${plural(instances, "image")}, will be removed from storage. This can't be undone.`,
                })
              }
            >
              <Trash2 /> Delete patient…
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>}
        <Button variant="ghost" size="icon-sm" aria-label="Close the patient" title="Close (Esc)" className="text-muted-foreground" onClick={onClose}>
          <X />
        </Button>
      </header>

      <section className="grid grid-cols-2 divide-x border-b bg-card sm:grid-cols-5">
        <Fact label="Studies">{list.length || "…"}</Fact>
        <Fact label="Series">{series.toLocaleString()}</Fact>
        <Fact label="Images">{instances.toLocaleString()}</Fact>
        <Fact label="First study">{list.length ? formatDate(list[list.length - 1].date) : "…"}</Fact>
        <Fact label="Modalities">
          <Modalities list={who.modalities} className="mt-1 flex-nowrap" />
        </Fact>
      </section>

      <section className="px-8 py-6">
        <div className="mb-4 flex items-baseline justify-between">
          <h3 className="text-sm font-medium">Timeline</h3>
          <span className="text-xs text-muted-foreground">Tick two studies to compare them side by side</span>
        </div>
        {studies.isPending && <Skeleton className="h-40" />}
        <ol className="relative">
          {list.map((s, i) => (
            <li key={s.id} className="relative pb-6 pl-8 last:pb-0">
              {/* the line between the dots */}
              {i < list.length - 1 && <span className="absolute top-3 bottom-0 left-[7px] w-px bg-border" />}
              <span className={cn("absolute top-1.5 left-0 size-[15px] rounded-full border-2 border-background ring-1 ring-border", i === 0 ? "bg-selection" : "bg-muted-foreground/40")} />
              <TimelineStudy study={s} newest={i === 0} ticked={compare.includes(s.id)} onTick={(on) => tick(s.id, on)} />
            </li>
          ))}
        </ol>
      </section>

      {picked.length > 0 && (
        <div className="sticky bottom-0 mt-auto flex items-center gap-3 border-t bg-popover/95 px-8 py-3 backdrop-blur animate-in fade-in slide-in-from-bottom-2">
          <Columns2 className="size-4 text-muted-foreground" />
          <span className="min-w-0 flex-1 truncate text-sm">
            {picked.length === 1 ? (
              <>
                <span className="font-medium">{picked[0].description || "A study"}</span> <span className="text-muted-foreground">({formatDate(picked[0].date)}): tick one more to compare</span>
              </>
            ) : (
              <>
                <span className="font-medium">{picked[0].description || "A study"}</span> <span className="text-muted-foreground">{formatDate(picked[0].date)}</span>
                <span className="mx-2 text-muted-foreground">beside</span>
                <span className="font-medium">{picked[1].description || "a study"}</span> <span className="text-muted-foreground">{formatDate(picked[1].date)}</span>
              </>
            )}
          </span>
          <Button size="sm" disabled={picked.length < 2} onClick={() => go(href.view(picked[0].id, null, picked[1].id))}>
            <Columns2 /> Compare
          </Button>
          <Button variant="ghost" size="icon-sm" aria-label="Clear the comparison" onClick={() => setCompare([])}>
            <X />
          </Button>
        </div>
      )}

      <ConfirmDelete
        pending={pending}
        onCancel={() => setPending(null)}
        onConfirm={async () => {
          const failed = await deleteStudies(pending!.ids);
          setPending(null);
          if (failed.length) toast.error("Some studies were not deleted", { description: failed[0] });
          else toast.success("Patient deleted");
          onDeleted();
        }}
      />
    </div>
  );
}

// one study on the timeline: when, what, its series (a click opens one), open, tick to compare
function TimelineStudy({ study: s, newest, ticked, onTick }: { study: Study; newest: boolean; ticked: boolean; onTick: (on: boolean) => void }) {
  const details = useStudySeries(s.id);
  return (
    <article className={cn("rounded-lg border bg-card p-4 transition-colors", ticked && "border-selection/50 bg-selection-soft/40")}>
      <div className="flex flex-wrap items-start gap-x-4 gap-y-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <span className="tabular font-medium text-foreground">{formatDate(s.date)}</span>
            {formatTime(s.time) && <span className="tabular">{formatTime(s.time)}</span>}
            {newest && <span className="rounded bg-selection-soft px-1.5 py-0.5 text-[10px] font-medium text-selection uppercase">Latest</span>}
          </div>
          <div className="mt-0.5 truncate font-medium">{s.description || <span className="text-muted-foreground">No description</span>}</div>
          <div className="mt-1 flex items-center gap-2 text-xs text-muted-foreground">
            <Modalities list={s.modalities} />
            <span>{[plural(s.series, "series", "series"), plural(s.instances, "image"), s.accession && `Acc. ${s.accession}`].filter(Boolean).join(" · ")}</span>
          </div>
        </div>
        <label className="flex h-8 cursor-pointer items-center gap-2 rounded-md px-2 text-sm text-muted-foreground hover:bg-accent">
          <Checkbox checked={ticked} onCheckedChange={(v) => onTick(!!v)} aria-label="Compare this study" />
          Compare
        </label>
        <Button size="sm" onClick={() => go(href.view(s.id))}>
          <Eye /> Open
        </Button>
      </div>
      <div className="mt-3 flex gap-2 overflow-x-auto pb-1">
        {details.isPending && Array.from({ length: Math.min(s.series || 2, 6) }, (_, i) => <Skeleton key={i} className="size-20 shrink-0 rounded-md" />)}
        {details.data?.series.map((x: any) => (
          <button key={x.ID} className="group/thumb w-20 shrink-0 text-left" onClick={() => go(href.view(s.id, x.ID))} title={`Open ${x.MainDicomTags.SeriesDescription || "this series"}`}>
            <SeriesThumb series={x} studyUid={s.uid} className="ring-selection ring-offset-2 ring-offset-card transition group-hover/thumb:ring-2" />
            <div className="mt-1 truncate text-[11px]">{x.MainDicomTags.SeriesDescription || x.MainDicomTags.Modality}</div>
          </button>
        ))}
      </div>
    </article>
  );
}
