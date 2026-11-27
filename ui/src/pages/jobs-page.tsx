// The Jobs page: every pull from a remote archive, send to one and write-back of a project's
// results, running or among the latest 100 over, newest first (/jobs). Asked again every two
// seconds while one runs; a running one can be cancelled (C-CANCEL).

import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { ArrowDownToLine, ArrowUpFromLine, ListChecks, Loader2, RotateCcw, Square } from "lucide-react";
import { plural } from "@/lib/api";
import { cancelJob, jobKind, useJobs, type Job } from "@/lib/remote";
import { href } from "@/lib/route";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";

const STATE_STYLE: Record<string, string> = {
  Running: "text-selection",
  Success: "text-emerald-700 dark:text-emerald-400",
  Warning: "text-amber-700 dark:text-amber-400",
  Cancelled: "text-muted-foreground",
  Failure: "text-destructive",
};

function duration(ms: number) {
  if (ms < 1000) return "under 1 s";
  const s = Math.round(ms / 1000);
  return s < 60 ? `${s} s` : `${Math.floor(s / 60)} min ${String(s % 60).padStart(2, "0")} s`;
}

function Progress({ job }: { job: Job }) {
  const done = (job.Completed ?? 0) + (job.Failed ?? 0) + (job.Warning ?? 0);
  const total = job.Remaining == null ? null : done + job.Remaining;
  const percent = job.Progress ?? (total ? Math.round((100 * done) / total) : null);
  return (
    <div className="flex min-w-40 flex-col gap-1">
      {job.State === "Running" && (
        <div
          role="progressbar"
          aria-label={`Job ${job.ID}`}
          aria-valuenow={percent ?? undefined}
          className="h-1.5 overflow-hidden rounded-full bg-muted"
        >
          <div className={cn("h-full rounded-full bg-selection", percent == null && "w-1/3 animate-pulse")} style={percent == null ? undefined : { width: `${percent}%` }} />
        </div>
      )}
      <span className="tabular text-xs text-muted-foreground">
        {plural(job.Completed ?? 0, "image")}
        {job.Failed ? `, ${job.Failed} failed` : ""}
        {job.Warning ? `, ${job.Warning} with warnings` : ""}
        {job.Remaining ? `, ${job.Remaining} to go` : ""}
      </span>
    </div>
  );
}

export function JobsPage({ open }: { open: string | null }) {
  const jobs = useJobs();
  const client = useQueryClient();
  const [stopping, setStopping] = useState<string | null>(null);

  async function cancel(job: Job) {
    setStopping(job.ID);
    try {
      await cancelJob(job.ID);
      toast(`Job ${job.ID} asked to stop`, { description: "What was moved by then stays." });
      await client.invalidateQueries({ queryKey: ["jobs"] });
    } catch (e) {
      toast.error(`Job ${job.ID} could not be stopped`, { description: (e as Error).message });
    } finally {
      setStopping(null);
    }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="px-4 pt-5 pb-3 md:px-6">
        <h1 className="text-lg font-semibold tracking-tight">Jobs</h1>
        <p className="text-sm text-muted-foreground">
          {jobs.data
            ? `${plural(jobs.data.filter((j) => j.State === "Running").length, "job")} running, ${jobs.data.filter((j) => j.State !== "Running").length} over`
            : "Pulls from remote archives, sends to them and write-backs"}
        </p>
      </div>
      <div className="min-h-0 flex-1 overflow-auto border-t">
        {jobs.isPending ? (
          <div className="space-y-3 px-6 py-5">
            {Array.from({ length: 5 }, (_, i) => (
              <Skeleton key={i} className="h-10" />
            ))}
          </div>
        ) : jobs.isError ? (
          <div role="alert" className="px-6 py-16 text-center">
            <div className="font-medium">The jobs could not be listed</div>
            <p className="mt-1 text-sm text-muted-foreground">{jobs.error.message}</p>
            <Button variant="outline" className="mt-5" onClick={() => jobs.refetch()}>
              Try again
            </Button>
          </div>
        ) : jobs.data.length === 0 ? (
          <div className="flex flex-col items-center px-6 py-20 text-center">
            <div className="mb-4 grid size-11 place-items-center rounded-full border bg-card">
              <ListChecks className="size-5 text-muted-foreground" strokeWidth={1.75} />
            </div>
            <div className="font-medium">No job yet</div>
            <p className="mt-1 max-w-sm text-sm text-muted-foreground">
              Pull studies from a <a className="underline underline-offset-4" href={href.remote()}>remote archive</a>, or send them to one from the study list.
            </p>
          </div>
        ) : (
          <Table aria-label="Jobs">
            <TableHeader className="sticky top-0 z-10 bg-background shadow-[inset_0_-1px_0_var(--border)]">
              <TableRow className="border-0 hover:bg-transparent">
                <TableHead className="pl-6 text-xs">Job</TableHead>
                <TableHead className="text-xs">Archive</TableHead>
                <TableHead className="text-xs">Studies</TableHead>
                <TableHead className="text-xs">State</TableHead>
                <TableHead className="text-xs">Progress</TableHead>
                <TableHead className="text-xs">Took</TableHead>
                <TableHead className="pr-6" />
              </TableRow>
            </TableHeader>
            <TableBody>
              {jobs.data.map((job) => {
                const Icon = jobKind(job) === "Pull" ? ArrowDownToLine : jobKind(job) === "Send" ? ArrowUpFromLine : RotateCcw;
                return (
                  <TableRow key={job.ID} aria-current={job.ID === open ? "true" : undefined} className={cn("h-14", job.ID === open && "bg-selection-soft")}>
                    <TableCell className="pl-6">
                      <div className="flex items-center gap-2 font-medium">
                        <Icon className="size-4 text-muted-foreground" /> {jobKind(job)}
                      </div>
                      <div className="text-xs text-muted-foreground">
                        #{job.ID} · {job.Type}
                        {job.Restarts ? ` · taken up again ${plural(job.Restarts, "time")}` : ""}
                      </div>
                    </TableCell>
                    <TableCell className="font-medium">{job.Remote}</TableCell>
                    <TableCell className="tabular text-muted-foreground" title={job.StudyInstanceUIDs.join("\n")}>
                      {plural(job.StudyInstanceUIDs.length, "study", "studies")}
                    </TableCell>
                    <TableCell>
                      <div className={cn("text-sm font-medium", STATE_STYLE[job.State])}>{job.State === "Success" ? "Done" : job.State}</div>
                      {job.Error && <div className="max-w-72 truncate text-xs text-destructive" title={job.Error}>{job.Error}</div>}
                    </TableCell>
                    <TableCell>
                      <Progress job={job} />
                    </TableCell>
                    <TableCell className="tabular text-muted-foreground">{duration(job.ElapsedMs)}</TableCell>
                    <TableCell className="pr-6 text-right">
                      {job.State === "Running" && (
                        <Button variant="outline" size="sm" disabled={stopping === job.ID} onClick={() => cancel(job)}>
                          {stopping === job.ID ? <Loader2 className="animate-spin" /> : <Square />} Stop
                        </Button>
                      )}
                    </TableCell>
                  </TableRow>
                );
              })}
            </TableBody>
          </Table>
        )}
      </div>
    </div>
  );
}
