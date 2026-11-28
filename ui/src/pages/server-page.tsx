// The Server page, for administrators: what the archive holds, what is happening now (DIMSE
// associations, jobs, the quarantine) and what went wrong lately (the warnings and errors the
// server logged). Asked again every two seconds; nothing here changes anything.

import type { ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { api, bytes, plural } from "@/lib/api";
import { config } from "@/lib/config";
import { href } from "@/lib/route";
import { cn } from "@/lib/utils";
import { Skeleton } from "@/components/ui/skeleton";

type Server = {
  version: string;
  archive: { studies: number; instances: number; bytes: number; files_failed: number; files_never_checked: number; trash: number };
  associations: { peer: string; calling: string; called: string; since_ms: number; stored: number; queries: number; retrievals: number }[];
  jobs: {
    id: string;
    remote: string;
    studies: number;
    state: string;
    progress: { Remaining?: number | null; Completed?: number | null; Failed?: number | null; Warning?: number | null };
    error: string | null;
    started_ms: number;
    finished_ms: number | null;
    restarts: number;
  }[];
  quarantine: null | { files: number; bytes: number; latest_ms: number | null } | { error: string };
  log: { time_ms: number; level: "WARN" | "ERROR"; target: string; message: string; fields: string }[];
};

const clock = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
const day = (ms: number) => new Date(ms).toLocaleDateString([], { month: "short", day: "numeric" });

/** 0:42, 12:05, 1:02:10: how long since. */
function since(ms: number, now: number) {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  const [h, m, sec] = [Math.floor(s / 3600), Math.floor((s % 3600) / 60), s % 60];
  const two = (n: number) => String(n).padStart(2, "0");
  return h ? `${h}:${two(m)}:${two(sec)}` : `${m}:${two(sec)}`;
}

function Figure({ label, value, note, alarm }: { label: string; value: ReactNode; note?: ReactNode; alarm?: boolean }) {
  return (
    <div className="min-w-0 py-4 pr-4 sm:px-5 sm:first:pl-0">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div className={cn("mt-1 text-2xl font-semibold tabular-nums tracking-tight", alarm && "text-destructive")}>{value}</div>
      {note && <div className="mt-0.5 truncate text-xs text-muted-foreground">{note}</div>}
    </div>
  );
}

function Block({ title, count, link, children }: { title: string; count?: string; link?: { to: string; label: string }; children: ReactNode }) {
  return (
    <section>
      <div className="mb-2 flex items-baseline gap-2">
        <h2 className="text-sm font-semibold">{title}</h2>
        {count && <span className="text-xs text-muted-foreground">{count}</span>}
        {link && (
          <a href={link.to} className="ml-auto text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline">
            {link.label}
          </a>
        )}
      </div>
      {children}
    </section>
  );
}

const Empty = ({ children }: { children: ReactNode }) => (
  <p className="rounded-md border border-dashed px-4 py-5 text-center text-sm text-muted-foreground">{children}</p>
);

function JobProgress({ job }: { job: Server["jobs"][number] }) {
  const p = job.progress;
  const done = (p.Completed ?? 0) + (p.Failed ?? 0) + (p.Warning ?? 0);
  const total = p.Remaining == null ? null : done + p.Remaining;
  const percent = total ? Math.round((100 * done) / total) : job.state === "Running" ? null : 100;
  if (job.state !== "Running")
    return (
      <span className={cn("text-xs", job.state === "Failure" ? "text-destructive" : "text-muted-foreground")} title={job.error ?? undefined}>
        {job.state === "Success" ? "Done" : job.state}
        {job.finished_ms ? ` · ${clock(job.finished_ms)}` : ""}
      </span>
    );
  return (
    <div className="flex items-center gap-2">
      <div className="h-1 w-24 overflow-hidden rounded-full bg-muted">
        <div className="h-full bg-primary transition-[width]" style={{ width: `${percent ?? 0}%` }} />
      </div>
      <span className="text-xs tabular-nums text-muted-foreground">{percent == null ? "starting" : `${percent} %`}</span>
    </div>
  );
}

export function ServerPage() {
  const server = useQuery({ queryKey: ["server"], queryFn: () => api<Server>("ui/api/server"), refetchInterval: 2_000 });
  const now = server.dataUpdatedAt || Date.now();
  const d = server.data;

  return (
    <div className="h-full overflow-y-auto">
      <div className="px-4 py-5 md:px-6">
        <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
          <h1 className="text-lg font-semibold tracking-tight">Server</h1>
          <p className="text-xs text-muted-foreground" aria-live="polite">
            pacs-rs {d?.version ?? config.version}
            {server.isError ? (
              <span className="text-destructive"> · not answering: {server.error instanceof Error ? server.error.message : ""}</span>
            ) : (
              server.dataUpdatedAt > 0 && ` · as of ${clock(server.dataUpdatedAt)}`
            )}
          </p>
        </div>

        {/* the archive, in figures */}
        <div className="mt-4 grid grid-cols-2 divide-y border-y sm:grid-cols-5 sm:divide-x sm:divide-y-0">
          {d ? (
            <>
              <Figure label="Studies" value={d.archive.studies.toLocaleString()} />
              <Figure label="Images" value={d.archive.instances.toLocaleString()} />
              <Figure label="Stored" value={bytes(d.archive.bytes)} />
              <Figure label="Trash" value={d.archive.trash.toLocaleString()} note={d.archive.trash ? "deletions restorable" : "empty"} />
              <Figure
                label="File checks"
                value={d.archive.files_failed ? `${d.archive.files_failed} failed` : "All pass"}
                note={d.archive.files_never_checked ? `${d.archive.files_never_checked.toLocaleString()} not checked yet` : undefined}
                alarm={d.archive.files_failed > 0}
              />
            </>
          ) : (
            Array.from({ length: 5 }, (_, i) => (
              <div key={i} className="py-4 pr-4 sm:px-5 sm:first:pl-0">
                <Skeleton className="h-3 w-14" />
                <Skeleton className="mt-2 h-6 w-20" />
              </div>
            ))
          )}
        </div>

        <div className="mt-8 grid gap-10 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
          {/* what is happening now */}
          <div className="grid content-start gap-8">
            <Block title="Associations" count={d ? `${d.associations.length} open` : undefined}>
              {!d ? (
                <Skeleton className="h-16 w-full" />
              ) : d.associations.length === 0 ? (
                <Empty>No association open.</Empty>
              ) : (
                <ul className="divide-y rounded-md border">
                  {d.associations.map((a) => (
                    <li key={`${a.peer}${a.since_ms}`} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 px-4 py-2.5">
                      <div className="min-w-0">
                        <div className="truncate text-sm font-medium">
                          {a.calling} <span className="font-normal text-muted-foreground">→</span> {a.called}
                        </div>
                        <div className="truncate font-mono text-xs text-muted-foreground">{a.peer}</div>
                      </div>
                      <div className="text-right text-xs tabular-nums text-muted-foreground">
                        <div className="text-sm text-foreground">{since(a.since_ms, now)}</div>
                        {[a.stored && plural(a.stored, "image") + " in", a.queries && plural(a.queries, "query", "queries"), a.retrievals && plural(a.retrievals, "retrieval")]
                          .filter(Boolean)
                          .join(" · ") || "connected"}
                      </div>
                    </li>
                  ))}
                </ul>
              )}
            </Block>

            <Block title="Jobs" count={d ? `${d.jobs.filter((j) => j.state === "Running").length} running` : undefined} link={{ to: href.jobs(), label: "All jobs" }}>
              {!d ? (
                <Skeleton className="h-16 w-full" />
              ) : d.jobs.length === 0 ? (
                <Empty>No move from a remote archive yet.</Empty>
              ) : (
                <ul className="divide-y rounded-md border">
                  {d.jobs.map((j) => (
                    <li key={j.id} className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-x-4 px-4 py-2.5">
                      <div className="min-w-0">
                        <div className="truncate text-sm">
                          <span className="font-medium">From {j.remote}</span>
                          <span className="text-muted-foreground"> · {plural(j.studies, "study", "studies")}</span>
                        </div>
                        <div className="text-xs text-muted-foreground">
                          #{j.id} · {day(j.started_ms)} {clock(j.started_ms)}
                          {j.restarts > 0 && ` · taken up after ${plural(j.restarts, "restart")}`}
                        </div>
                      </div>
                      <JobProgress job={j} />
                    </li>
                  ))}
                </ul>
              )}
            </Block>

            <Block title="Quarantine">
              {!d ? (
                <Skeleton className="h-10 w-full" />
              ) : d.quarantine === null ? (
                <Empty>No quarantine set: a project refuses what its profile cannot de-identify (PACS_QUARANTINE).</Empty>
              ) : "error" in d.quarantine ? (
                <p className="rounded-md border border-destructive/40 px-4 py-3 text-sm text-destructive">Not readable: {d.quarantine.error}</p>
              ) : d.quarantine.files === 0 ? (
                <Empty>Nothing in quarantine.</Empty>
              ) : (
                <p className="rounded-md border px-4 py-3 text-sm">
                  <span className="font-medium">{plural(d.quarantine.files, "file")}</span>
                  <span className="text-muted-foreground">
                    {" "}
                    · {bytes(d.quarantine.bytes)}
                    {d.quarantine.latest_ms && ` · the latest ${day(d.quarantine.latest_ms)} ${clock(d.quarantine.latest_ms)}`}
                  </span>
                </p>
              )}
            </Block>
          </div>

          {/* what went wrong lately */}
          <Block title="Warnings and errors" count={d ? (d.log.length ? `the latest ${d.log.length}` : undefined) : undefined}>
            {!d ? (
              <Skeleton className="h-40 w-full" />
            ) : d.log.length === 0 ? (
              <Empty>Nothing logged at warning or error since the server started.</Empty>
            ) : (
              <ol className="max-h-[calc(100dvh-16rem)] divide-y overflow-y-auto rounded-md border">
                {d.log.map((l, i) => (
                  <li key={`${l.time_ms}-${i}`} className="px-4 py-2.5">
                    <div className="flex items-baseline gap-2 text-xs">
                      <span className={cn("font-semibold", l.level === "ERROR" ? "text-destructive" : "text-amber-600 dark:text-amber-500")}>
                        {l.level === "ERROR" ? "Error" : "Warning"}
                      </span>
                      <span className="tabular-nums text-muted-foreground">{clock(l.time_ms)}</span>
                      <span className="ml-auto truncate font-mono text-muted-foreground">{l.target}</span>
                    </div>
                    <p className="mt-1 text-sm">{l.message}</p>
                    {l.fields && <p className="mt-1 font-mono text-[11px] leading-relaxed break-all text-muted-foreground">{l.fields}</p>}
                  </li>
                ))}
              </ol>
            )}
          </Block>
        </div>
      </div>
    </div>
  );
}
