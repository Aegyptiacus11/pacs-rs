// The viewer's left panel: the patient, then every study of theirs (the open one first and
// expanded, priors under it), each with its series. A click shows a series in the active
// viewport; dragging it onto a viewport shows it there.

import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { ChevronDown, ChevronRight, Layers } from "lucide-react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { Skeleton } from "@/components/ui/skeleton";
import { SeriesThumb } from "@/components/series-thumb";
import { api, formatDate, personName, sexAge } from "@/lib/api";
import { sortSeries } from "@/lib/studies";
import { cn } from "@/lib/utils";

export type Registry = Map<string, { study: any; series: any }>;

function StudySeries({ study, registry, shown, active, onPick }: { study: any; registry: Registry; shown: Set<string>; active: string | null; onPick: (id: string) => void }) {
  const series = useQuery({
    queryKey: ["study-series", study.ID],
    queryFn: async () => {
      const list = sortSeries(await api<any[]>(`studies/${study.ID}/series`));
      for (const s of list) registry.set(s.ID, { study, series: s });
      return list;
    },
  });
  if (series.isPending) return <Skeleton className="mx-2 my-2 h-16" />;
  return (
    <div className="pb-1">
      {series.data!.map((s) => {
        const t = s.MainDicomTags;
        const isActive = s.ID === active;
        return (
          <button
            key={s.ID}
            draggable
            onDragStart={(e) => {
              e.dataTransfer.setData("application/x-pacs-series", s.ID);
              e.dataTransfer.effectAllowed = "copy";
            }}
            onClick={() => onPick(s.ID)}
            title="Click to show in the active viewport, or drag onto a viewport"
            className={cn(
              "group flex w-full items-start gap-2.5 border-l-2 border-transparent px-2.5 py-1.5 text-left hover:bg-accent",
              shown.has(s.ID) && "border-selection/40",
              isActive && "border-selection bg-selection-soft hover:bg-selection-soft",
            )}
          >
            <div className="relative w-16 shrink-0">
              <SeriesThumb series={s} studyUid={study.MainDicomTags.StudyInstanceUID} className="rounded-sm" />
              <span className="absolute top-0.5 left-0.5 flex items-center gap-0.5 rounded-sm bg-black/70 px-1 text-[10px] leading-4 text-white tabular">
                <Layers className="size-2.5" />
                {s.Instances.length}
              </span>
            </div>
            <div className="min-w-0 pt-0.5">
              <div className="flex items-baseline gap-1.5 text-xs">
                <span className="font-mono font-semibold text-selection">{t.Modality || "—"}</span>
                <span className="text-muted-foreground tabular">
                  {t.SeriesNumber ?? "?"}/{s.Instances.length}
                </span>
              </div>
              <div className="line-clamp-2 text-xs leading-snug">{t.SeriesDescription || "No description"}</div>
            </div>
          </button>
        );
      })}
    </div>
  );
}

export function StudyBrowser({
  study,
  registry,
  shown,
  active,
  onPick,
}: {
  study: any;
  registry: Registry;
  shown: Set<string>;
  active: string | null;
  onPick: (id: string) => void;
}) {
  const p = study.PatientMainDicomTags;
  const studies = useQuery({
    queryKey: ["patient-studies", study.ParentPatient],
    queryFn: () => api<any[]>(`patients/${study.ParentPatient}/studies`),
  });
  const [open, setOpen] = useState<Set<string>>(() => new Set([study.ID]));
  const list = [...(studies.data ?? [study])].sort((a, b) => {
    if (a.ID === study.ID) return -1;
    if (b.ID === study.ID) return 1;
    return String(b.MainDicomTags.StudyDate).localeCompare(String(a.MainDicomTags.StudyDate));
  });
  const toggle = (id: string) => setOpen((s) => (s.has(id) ? (s.delete(id), new Set(s)) : new Set(s.add(id))));

  return (
    <aside className="flex w-60 shrink-0 flex-col border-r bg-card">
      <div className="border-b px-3 py-2.5">
        <div className="truncate text-sm font-semibold">{personName(p.PatientName)}</div>
        <div className="truncate text-xs text-muted-foreground">{[p.PatientID, sexAge(p.PatientSex, p.PatientBirthDate)].filter(Boolean).join(" · ")}</div>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        {list.map((s) => {
          const t = s.MainDicomTags;
          const isOpen = open.has(s.ID);
          return (
            <div key={s.ID} className="border-b">
              <button onClick={() => toggle(s.ID)} className="flex w-full items-center gap-1.5 px-2 py-2 text-left hover:bg-accent">
                {isOpen ? <ChevronDown className="size-3.5 shrink-0 text-muted-foreground" /> : <ChevronRight className="size-3.5 shrink-0 text-muted-foreground" />}
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-xs">
                    <span className="tabular font-medium">{formatDate(t.StudyDate)}</span>
                    {s.ID === study.ID ? (
                      <span className="rounded-sm bg-selection px-1 text-[10px] leading-4 font-medium text-white">Current</span>
                    ) : (
                      <span className="rounded-sm border px-1 text-[10px] leading-4 text-muted-foreground">
                        {String(t.StudyDate) < String(study.MainDicomTags.StudyDate) ? "Prior" : "Later"}
                      </span>
                    )}
                  </div>
                  <div className="truncate text-xs text-muted-foreground">{t.StudyDescription || "No description"}</div>
                </div>
              </button>
              {isOpen && <StudySeries study={s} registry={registry} shown={shown} active={active} onPick={onPick} />}
            </div>
          );
        })}
      </ScrollArea>
    </aside>
  );
}
