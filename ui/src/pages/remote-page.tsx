// The Remote page: the other archives pacs-rs knows (PACS_DIMSE_REMOTES), each tested with a
// C-ECHO when the page opens; a search of one's studies (C-FIND), and the ticked studies
// pulled into this archive, or a project's de-identified copy, as a job (C-MOVE or C-GET).

import { useEffect, useMemo, useState, type FormEvent } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowDownToLine, CircleAlert, ListChecks, Loader2, RefreshCw, Search, Server, TriangleAlert } from "lucide-react";
import { formatDate, formatTime, personName, plural, sexAge } from "@/lib/api";
import { IMAGING_MODALITIES } from "@/lib/modalities";
import { emptyQuery, pull, useEcho, useRemotes, useRemoteSearch, type RemoteAe, type RemoteQuery } from "@/lib/remote";
import { href } from "@/lib/route";
import { may } from "@/lib/session";
import { cn } from "@/lib/utils";
import { useProjects } from "@/pages/projects-choice";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Modalities } from "@/components/modalities";

function EchoState({ ae }: { ae: RemoteAe }) {
  const echo = useEcho(ae.id);
  if (echo.isPending)
    return (
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <Loader2 className="size-3 animate-spin" /> Testing…
      </span>
    );
  if (echo.isError)
    return (
      <span className="flex items-center gap-1.5 text-xs text-destructive" title={echo.error.message}>
        <span className="size-1.5 rounded-full bg-destructive" /> Not reachable
      </span>
    );
  return (
    <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
      <span className="size-1.5 rounded-full bg-emerald-500" /> Answers · {echo.data.ElapsedMs} ms
    </span>
  );
}

function RemoteList({ remotes, chosen, onChoose }: { remotes: RemoteAe[]; chosen: string | null; onChoose: (id: string) => void }) {
  const client = useQueryClient();
  return (
    <aside className="flex w-full shrink-0 flex-col border-b md:w-72 md:border-r md:border-b-0">
      <div className="flex items-center justify-between px-5 pt-5 pb-3">
        <div>
          <h1 className="text-lg font-semibold tracking-tight">Remote archives</h1>
          <p className="text-sm text-muted-foreground">{plural(remotes.length, "archive")}</p>
        </div>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label="Test every archive again"
          title="Test every archive again (C-ECHO)"
          onClick={() => client.invalidateQueries({ queryKey: ["remotes"] })}
        >
          <RefreshCw />
        </Button>
      </div>
      <ul aria-label="Remote archives" className="flex gap-1 overflow-x-auto px-3 pb-3 md:flex-col md:overflow-visible">
        {remotes.map((ae) => (
          <li key={ae.id} className="shrink-0">
            <button
              aria-current={ae.id === chosen ? "true" : undefined}
              onClick={() => onChoose(ae.id)}
              className={cn(
                "flex w-full flex-col items-start gap-1 rounded-lg px-3 py-2.5 text-left hover:bg-accent",
                ae.id === chosen && "bg-selection-soft hover:bg-selection-soft",
              )}
            >
              <span className="font-medium">{ae.id}</span>
              <span className="font-mono text-xs text-muted-foreground">
                {ae.aet} · {ae.host}:{ae.port}
              </span>
              <EchoState ae={ae} />
            </button>
          </li>
        ))}
      </ul>
      <a href={href.jobs()} className="mx-3 mt-auto mb-3 hidden items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-muted-foreground hover:bg-accent hover:text-foreground md:flex">
        <ListChecks className="size-4" /> Jobs
      </a>
    </aside>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="flex min-w-0 flex-col gap-1.5 text-xs font-medium text-muted-foreground">
      {label}
      {children}
    </label>
  );
}

function SearchForm({ onSearch, busy }: { onSearch: (q: RemoteQuery) => void; busy: boolean }) {
  const [q, setQ] = useState<RemoteQuery>(emptyQuery);
  const set = (key: keyof RemoteQuery) => (e: { target: { value: string } }) => setQ((x) => ({ ...x, [key]: e.target.value }));
  const submit = (e: FormEvent) => {
    e.preventDefault();
    onSearch({ ...q });
  };
  return (
    <form onSubmit={submit} aria-label="Search the remote archive" className="grid grid-cols-2 gap-3 px-4 pb-4 sm:grid-cols-3 md:px-6 xl:grid-cols-[repeat(6,minmax(0,1fr))_auto] xl:items-end">
      <Field label="Patient name">
        <Input value={q.name} onChange={set("name")} />
      </Field>
      <Field label="Patient ID">
        <Input value={q.patientId} onChange={set("patientId")} />
      </Field>
      <Field label="Accession">
        <Input value={q.accession} onChange={set("accession")} />
      </Field>
      <Field label="From">
        <Input type="date" value={q.from} onChange={set("from")} />
      </Field>
      <Field label="To">
        <Input type="date" value={q.to} onChange={set("to")} />
      </Field>
      <Field label="Modality">
        <select
          value={q.modality}
          onChange={set("modality")}
          className="h-9 rounded-md border bg-background px-2.5 text-sm text-foreground shadow-xs dark:bg-input/30"
        >
          <option value="">Any</option>
          {IMAGING_MODALITIES.map(([code, name]) => (
            <option key={code} value={code}>
              {code} · {name}
            </option>
          ))}
        </select>
      </Field>
      <Button type="submit" disabled={busy} className="col-span-2 sm:col-span-1">
        {busy ? <Loader2 className="animate-spin" /> : <Search />} Search
      </Button>
    </form>
  );
}

function Results({ ae, search }: { ae: RemoteAe; search: ReturnType<typeof useRemoteSearch> }) {
  const [ticked, setTicked] = useState<Set<string>>(new Set());
  const [project, setProject] = useState("");
  const [pulling, setPulling] = useState(false);
  // only an administrator may list the projects (GET /projects needs `users`)
  const projects = useProjects(may("admin"));
  const client = useQueryClient();
  useEffect(() => setTicked(new Set()), [search.data]);

  const studies = search.data?.studies ?? [];
  const all = studies.length > 0 && studies.every((s) => ticked.has(s.uid));
  const toggle = (uid: string) =>
    setTicked((t) => {
      const next = new Set(t);
      if (!next.delete(uid)) next.add(uid);
      return next;
    });

  async function start() {
    setPulling(true);
    try {
      const job = await pull(ae.id, [...ticked], project || null);
      toast.success(`Pulling ${plural(ticked.size, "study", "studies")} from ${ae.id}`, {
        description: project ? `Into project ${project}, de-identified.` : "Into this archive.",
        action: { label: "Jobs", onClick: () => (location.hash = href.jobs(job.ID)) },
      });
      setTicked(new Set());
      client.invalidateQueries({ queryKey: ["jobs"] });
    } catch (e) {
      toast.error(`The pull from ${ae.id} did not start`, { description: (e as Error).message });
    } finally {
      setPulling(false);
    }
  }

  if (search.isPending)
    return (
      <div className="space-y-3 px-6 py-5">
        {Array.from({ length: 6 }, (_, i) => (
          <Skeleton key={i} className="h-8" />
        ))}
      </div>
    );
  if (search.isError)
    return (
      <div role="alert" className="mx-4 my-6 flex gap-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4 text-sm md:mx-6">
        <CircleAlert className="mt-0.5 size-4 shrink-0 text-destructive" />
        <div>
          <div className="font-medium">{ae.id} could not be searched</div>
          <p className="mt-1 text-muted-foreground">{search.error.message}</p>
        </div>
      </div>
    );

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      {search.data.warning && (
        <p role="status" className="mx-4 mb-3 flex items-start gap-2 rounded-md bg-amber-500/10 px-3 py-2 text-sm text-amber-800 md:mx-6 dark:text-amber-300">
          <TriangleAlert className="mt-0.5 size-4 shrink-0" /> {search.data.warning.replace(/^\d+\s+\S+\s+"?|"$/g, "")}
        </p>
      )}
      <div className="min-h-0 flex-1 overflow-auto border-t">
        {studies.length === 0 ? (
          <p className="px-6 py-16 text-center text-sm text-muted-foreground">No study of {ae.id} matches.</p>
        ) : (
          <Table aria-label={`Studies of ${ae.id}`}>
            <TableHeader className="sticky top-0 z-10 bg-background shadow-[inset_0_-1px_0_var(--border)]">
              <TableRow className="border-0 hover:bg-transparent">
                <TableHead className="w-11 pl-6">
                  <Checkbox
                    aria-label="Tick every study shown"
                    checked={all ? true : ticked.size ? "indeterminate" : false}
                    onCheckedChange={() => setTicked(all ? new Set() : new Set(studies.map((s) => s.uid)))}
                  />
                </TableHead>
                <TableHead className="text-xs">Date</TableHead>
                <TableHead className="text-xs">Patient</TableHead>
                <TableHead className="text-xs">Study</TableHead>
                <TableHead className="text-xs">Modality</TableHead>
                <TableHead className="pr-6 text-right text-xs">Series · images</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {studies.map((s) => (
                <TableRow key={s.uid} data-state={ticked.has(s.uid) ? "selected" : undefined} className="h-12 cursor-pointer" onClick={() => toggle(s.uid)}>
                  <TableCell className="pl-6" onClick={(e) => e.stopPropagation()}>
                    <Checkbox aria-label={`Tick ${s.description || "the study"} of ${personName(s.name)}`} checked={ticked.has(s.uid)} onCheckedChange={() => toggle(s.uid)} />
                  </TableCell>
                  <TableCell className="tabular">
                    <div>{formatDate(s.date)}</div>
                    <div className="text-xs text-muted-foreground">{formatTime(s.time)}</div>
                  </TableCell>
                  <TableCell>
                    <div className="font-medium">{personName(s.name)}</div>
                    <div className="text-xs text-muted-foreground">{[s.patientId, sexAge(s.sex, s.birth, s.date)].filter(Boolean).join(" · ")}</div>
                  </TableCell>
                  <TableCell>
                    <div className={cn(!s.description && "text-muted-foreground")}>{s.description || "No description"}</div>
                    {s.accession && <div className="text-xs text-muted-foreground">Acc. {s.accession}</div>}
                  </TableCell>
                  <TableCell>
                    <Modalities list={s.modalities} />
                  </TableCell>
                  <TableCell className="pr-6 text-right tabular text-muted-foreground">
                    {s.series ?? "—"} · {s.instances ?? "—"}
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        )}
      </div>
      <div className="flex flex-wrap items-center gap-3 border-t px-4 py-3 md:px-6">
        <span className="mr-auto text-sm text-muted-foreground">
          {plural(studies.length, "study", "studies")}
          {ticked.size ? ` · ${ticked.size} ticked` : ""}
        </span>
        {may("admin") && projects.data && projects.data.length > 0 && (
          <label className="flex items-center gap-2 text-sm text-muted-foreground">
            Into
            <select value={project} onChange={(e) => setProject(e.target.value)} className="h-8 rounded-md border bg-background px-2 text-sm text-foreground dark:bg-input/30">
              <option value="">this archive</option>
              {projects.data.map((p) => (
                <option key={p} value={p}>
                  project {p} (de-identified)
                </option>
              ))}
            </select>
          </label>
        )}
        <Button size="sm" disabled={!ticked.size || pulling} onClick={start}>
          {pulling ? <Loader2 className="animate-spin" /> : <ArrowDownToLine />} Pull {ticked.size ? plural(ticked.size, "study", "studies") : ""}
        </Button>
      </div>
    </div>
  );
}

export function RemotePage({ open }: { open: string | null }) {
  const remotes = useRemotes();
  const [query, setQuery] = useState<RemoteQuery | null>(null);
  const chosen = useMemo(() => remotes.data?.find((r) => r.id === open) ?? remotes.data?.[0] ?? null, [remotes.data, open]);
  const search = useRemoteSearch(chosen?.id ?? null, query);
  useEffect(() => setQuery(null), [chosen?.id]);

  if (remotes.isPending)
    return (
      <div className="grid h-full place-items-center text-muted-foreground">
        <Loader2 className="size-5 animate-spin" />
      </div>
    );
  if (remotes.isError)
    return (
      <div role="alert" className="mx-auto mt-20 max-w-md px-6 text-center">
        <div className="font-medium">The remote archives could not be listed</div>
        <p className="mt-1 text-sm text-muted-foreground">{remotes.error.message}</p>
      </div>
    );
  if (!chosen)
    return (
      <div className="mx-auto mt-20 flex max-w-md flex-col items-center px-6 text-center">
        <div className="mb-4 grid size-11 place-items-center rounded-full border bg-card">
          <Server className="size-5 text-muted-foreground" strokeWidth={1.75} />
        </div>
        <div className="font-medium">No remote archive yet</div>
        <p className="mt-1 text-sm text-muted-foreground">
          List the archives pacs-rs may search and pull from in <code className="font-mono text-xs">PACS_DIMSE_REMOTES</code>, as{" "}
          <code className="font-mono text-xs">id=AE_TITLE@host:port</code>, and register pacs-rs on each of them.
        </p>
      </div>
    );

  return (
    <div className="flex h-full flex-col md:flex-row">
      <RemoteList remotes={remotes.data} chosen={chosen.id} onChoose={(id) => (location.hash = href.remote(id))} />
      <section className="flex min-h-0 min-w-0 flex-1 flex-col">
        <div className="px-4 pt-5 pb-3 md:px-6">
          <h2 className="text-lg font-semibold tracking-tight">{chosen.id}</h2>
          <p className="font-mono text-xs text-muted-foreground">
            {chosen.aet} · {chosen.host}:{chosen.port}
          </p>
        </div>
        <SearchForm
          key={chosen.id}
          busy={search.isFetching}
          onSearch={(q) => (JSON.stringify(q) === JSON.stringify(query) ? search.refetch() : setQuery(q))}
        />
        {query ? (
          <Results key={JSON.stringify(query)} ae={chosen} search={search} />
        ) : (
          <p className="border-t px-6 py-16 text-center text-sm text-muted-foreground">
            Search {chosen.id}'s studies, then tick the ones to pull into this archive.
          </p>
        )}
      </section>
    </div>
  );
}
