// Patients: one row per Patient ID; a row shows that patient's studies.

import { useMemo, useState } from "react";
import { Download, FileStack, MoreHorizontal, Search, Trash2, Users } from "lucide-react";
import { toast } from "sonner";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger } from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { ConfirmDelete, type PendingDelete } from "@/components/confirm-delete";
import { api, formatDate, initials, personName, plural, sexAge, url } from "@/lib/api";
import { go, href } from "@/lib/route";
import { usePatients, useRefreshStudies } from "@/lib/studies";

export function PatientsPage() {
  const patients = usePatients();
  const refresh = useRefreshStudies();
  const [query, setQuery] = useState("");
  const [pending, setPending] = useState<PendingDelete | null>(null);
  const rows = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return (patients.data ?? [])
      .filter((p) => words.every((w) => `${p.name} ${personName(p.name)} ${p.patientId}`.toLowerCase().includes(w)))
      .sort((a, b) => personName(a.name).localeCompare(personName(b.name)));
  }, [patients.data, query]);

  return (
    <div className="mx-auto max-w-[1440px] px-6 py-8">
      <div className="mb-6">
        <h1 className="text-xl font-semibold tracking-tight">Patients</h1>
        <p className="mt-1 text-sm text-muted-foreground">{patients.isPending ? "Loading…" : plural(rows.length, "patient")}</p>
      </div>
      <div className="relative mb-4 w-full max-w-sm">
        <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Filter by name or patient ID" className="h-9 bg-card pl-9" />
      </div>
      <div className="overflow-hidden rounded-lg border bg-card">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="h-10 text-xs font-medium text-muted-foreground">Patient</TableHead>
              <TableHead className="h-10 text-xs font-medium text-muted-foreground">Born</TableHead>
              <TableHead className="h-10 text-xs font-medium text-muted-foreground">Modalities</TableHead>
              <TableHead className="h-10 text-xs font-medium text-muted-foreground">Last study</TableHead>
              <TableHead className="h-10 text-right text-xs font-medium text-muted-foreground">Studies</TableHead>
              <TableHead className="h-10 w-12" />
            </TableRow>
          </TableHeader>
          <TableBody>
            {patients.isPending &&
              Array.from({ length: 4 }, (_, i) => (
                <TableRow key={i} className="h-14">
                  <TableCell colSpan={6}>
                    <Skeleton className="h-3 w-64" />
                  </TableCell>
                </TableRow>
              ))}
            {rows.map((p) => (
              <TableRow key={p.id} className="h-14 cursor-pointer" tabIndex={0} onClick={() => go(href.studies(p.id))} onKeyDown={(e) => e.key === "Enter" && go(href.studies(p.id))}>
                <TableCell>
                  <div className="flex items-center gap-3">
                    <span className="grid size-8 place-items-center rounded-full bg-secondary text-xs font-medium text-muted-foreground ring-1 ring-border">{initials(p.name)}</span>
                    <div>
                      <div className="font-medium">{personName(p.name)}</div>
                      <div className="text-xs text-muted-foreground">{[p.patientId, sexAge(p.sex, p.birth)].filter(Boolean).join(" · ")}</div>
                    </div>
                  </div>
                </TableCell>
                <TableCell className="tabular">{formatDate(p.birth)}</TableCell>
                <TableCell>
                  <div className="flex gap-1">
                    {p.modalities.sort().map((m) => (
                      <Badge key={m} variant="outline" className="h-5 rounded px-1.5 font-mono text-[11px] text-muted-foreground">
                        {m}
                      </Badge>
                    ))}
                  </div>
                </TableCell>
                <TableCell className="tabular">{formatDate(p.last)}</TableCell>
                <TableCell className="tabular text-right">
                  {p.studies.length}
                  <span className="ml-1.5 text-xs text-muted-foreground">{p.instances.toLocaleString()} images</span>
                </TableCell>
                <TableCell onClick={(e) => e.stopPropagation()}>
                  <DropdownMenu>
                    <DropdownMenuTrigger asChild>
                      <Button variant="ghost" size="icon-sm" aria-label="Patient actions">
                        <MoreHorizontal />
                      </Button>
                    </DropdownMenuTrigger>
                    <DropdownMenuContent align="end" className="w-52">
                      <DropdownMenuItem onSelect={() => go(href.studies(p.id))}>
                        <FileStack /> Show studies
                      </DropdownMenuItem>
                      <DropdownMenuItem asChild>
                        <a href={url(`patients/${p.id}/archive`)} download>
                          <Download /> Download ZIP
                        </a>
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                      <DropdownMenuItem
                        variant="destructive"
                        onSelect={() =>
                          setPending({
                            ids: [p.id],
                            title: "Delete this patient?",
                            description: `${personName(p.name)} (${p.patientId || "no ID"}) and all ${plural(p.studies.length, "study", "studies")}, ${plural(p.instances, "image")}, will be removed from storage. This can't be undone.`,
                          })
                        }
                      >
                        <Trash2 /> Delete patient…
                      </DropdownMenuItem>
                    </DropdownMenuContent>
                  </DropdownMenu>
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
        {!patients.isPending && rows.length === 0 && (
          <div className="flex flex-col items-center px-6 py-20 text-center">
            <div className="mb-4 grid size-11 place-items-center rounded-full border bg-background">
              <Users className="size-5 text-muted-foreground" strokeWidth={1.75} />
            </div>
            <div className="font-medium">{query ? "No patient matches" : "No patients yet"}</div>
          </div>
        )}
      </div>
      <ConfirmDelete
        pending={pending}
        onCancel={() => setPending(null)}
        onConfirm={async () => {
          try {
            await api(`patients/${pending!.ids[0]}`, { method: "DELETE" });
            toast.success("Patient deleted");
          } catch (e: any) {
            toast.error("Couldn't delete the patient", { description: e.message });
          }
          setPending(null);
          await refresh();
        }}
      />
    </div>
  );
}
