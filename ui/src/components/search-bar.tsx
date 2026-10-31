// Two searches with two jobs. The top bar's (Ctrl K, ⌘K on a Mac) finds anything in the
// archive: as words are typed it suggests patients and studies, ↓ ↑ and Enter (or a click)
// show one, on the page already open: on the studies, a patient's studies or the study; on the
// patients, the patient's record. Each list's own box (/) filters that list as words are typed.

import { useEffect, useRef, useState } from "react";
import { FileStack, Search, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { Modalities } from "@/components/modalities";
import { formatDate, initials, personName, plural, sexAge } from "@/lib/api";
import { go, href } from "@/lib/route";
import { usePatientSuggestions, useStudySearch, type Patient, type Study } from "@/lib/studies";
import { cn } from "@/lib/utils";

export const SEARCH_KEY = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘K" : "Ctrl K";

/** A list's own box: the words filter the list under it. */
export function FilterBox({ value, onChange, placeholder, className }: { value: string; onChange: (v: string) => void; placeholder: string; className?: string }) {
  return (
    <div className={cn("relative", className)}>
      <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        data-filter
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={(e) => e.key === "Escape" && value && (e.preventDefault(), onChange(""))}
        placeholder={placeholder}
        aria-label={placeholder}
        className="h-9 bg-card pr-10 pl-9"
      />
      {value ? (
        <button
          type="button"
          aria-label="Clear the filter"
          className="absolute top-1/2 right-2 grid size-6 -translate-y-1/2 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
          onClick={() => onChange("")}
        >
          <X className="size-3.5" />
        </button>
      ) : (
        <Kbd className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2">/</Kbd>
      )}
    </div>
  );
}

type Hit = { kind: "patient"; patient: Patient } | { kind: "study"; study: Study };

/** The top bar's: find a patient or a study anywhere, and open it. */
export function GlobalSearch({ page, className }: { page: "studies" | "patients"; className?: string }) {
  const [value, setValue] = useState("");
  const [focused, setFocused] = useState(false);
  const [asked, setAsked] = useState("");
  const [at, setAt] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const t = setTimeout(() => setAsked(value), 200);
    return () => clearTimeout(t);
  }, [value]);
  useEffect(() => setAt(0), [asked]);
  const words = focused ? asked : "";
  const patients = usePatientSuggestions(words);
  const studies = useStudySearch(words);
  const open = focused && !!value.trim();
  const hits: Hit[] = open
    ? [...(patients.data ?? []).slice(0, 4).map((patient) => ({ kind: "patient" as const, patient })), ...(studies.data ?? []).slice(0, 6).map((study) => ({ kind: "study" as const, study }))]
    : [];
  const searching = open && (patients.isFetching || studies.isFetching);
  const pick = (hit: Hit) => {
    setValue("");
    input.current?.blur();
    // the page stays the one open
    if (page === "patients") go(href.patients(hit.kind === "patient" ? hit.patient.id : hit.study.patient));
    else go(hit.kind === "patient" ? href.studies(hit.patient.id) : href.studies(null, hit.study.id));
  };
  const row = (hit: Hit, i: number) => (
    <button
      key={hit.kind === "patient" ? `p${hit.patient.id}` : `s${hit.study.id}`}
      id={`hit-${i}`}
      role="option"
      aria-selected={i === at}
      onMouseEnter={() => setAt(i)}
      onClick={() => pick(hit)}
      className={cn("flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left text-sm", i === at && "bg-accent")}
    >
      {hit.kind === "patient" ? (
        <>
          <span className="grid size-7 shrink-0 place-items-center rounded-full bg-secondary text-[11px] font-medium text-muted-foreground ring-1 ring-border">{initials(hit.patient.name)}</span>
          <span className="min-w-0 flex-1">
            <span className="block truncate font-medium">{personName(hit.patient.name)}</span>
            <span className="block truncate text-xs text-muted-foreground">{[hit.patient.patientId, sexAge(hit.patient.sex, hit.patient.birth)].filter(Boolean).join(" · ")}</span>
          </span>
          <span className="shrink-0 text-xs text-muted-foreground">{plural(hit.patient.studies, "study", "studies")}</span>
        </>
      ) : (
        <>
          <span className="grid size-7 shrink-0 place-items-center rounded-md bg-secondary text-muted-foreground ring-1 ring-border">
            <FileStack className="size-3.5" />
          </span>
          <span className="min-w-0 flex-1">
            <span className="block truncate font-medium">{hit.study.description || "No description"}</span>
            <span className="block truncate text-xs text-muted-foreground">{[personName(hit.study.name), hit.study.accession && `Acc. ${hit.study.accession}`].filter(Boolean).join(" · ")}</span>
          </span>
          <Modalities list={hit.study.modalities} className="shrink-0 flex-nowrap" />
          <span className="tabular w-24 shrink-0 text-right text-xs text-muted-foreground">{formatDate(hit.study.date)}</span>
        </>
      )}
    </button>
  );
  const firstStudy = hits.findIndex((h) => h.kind === "study");
  return (
    <div className={cn("relative", className)}>
      <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        ref={input}
        data-search
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" && hits.length) {
            e.preventDefault();
            setAt((i) => Math.min(hits.length - 1, i + 1));
          } else if (e.key === "ArrowUp" && hits.length) {
            e.preventDefault();
            setAt((i) => Math.max(0, i - 1));
          } else if (e.key === "Enter" && hits[at]) {
            e.preventDefault();
            pick(hits[at]);
          } else if (e.key === "Escape") {
            e.preventDefault();
            if (value) setValue("");
            else input.current?.blur();
          }
        }}
        placeholder="Find a patient or a study…"
        aria-label="Find a patient or a study"
        role="combobox"
        aria-expanded={hits.length > 0}
        aria-controls="search-hits"
        aria-activedescendant={hits.length ? `hit-${at}` : undefined}
        className="h-9 bg-background pr-16 pl-9"
      />
      {value ? (
        <button
          type="button"
          aria-label="Clear the search"
          className="absolute top-1/2 right-2 grid size-6 -translate-y-1/2 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => setValue("")}
        >
          <X className="size-3.5" />
        </button>
      ) : (
        <Kbd className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2">{SEARCH_KEY}</Kbd>
      )}
      {open && (
        <div
          id="search-hits"
          role="listbox"
          aria-label="Patients and studies"
          className="absolute inset-x-0 top-full z-40 mt-1 overflow-hidden rounded-lg border bg-popover p-1 shadow-lg animate-in fade-in-0 zoom-in-95"
          onMouseDown={(e) => e.preventDefault()}
        >
          {hits.length === 0 ? (
            <div className="px-3 py-4 text-center text-sm text-muted-foreground">{searching ? "Searching…" : "Nothing found"}</div>
          ) : (
            <>
              {hits[0].kind === "patient" && <div className="px-2 pt-1 pb-1 text-xs font-medium text-muted-foreground">Patients</div>}
              {hits.map((hit, i) => (
                <div key={i}>
                  {i === firstStudy && <div className={cn("px-2 pb-1 text-xs font-medium text-muted-foreground", i > 0 ? "mt-1 border-t pt-2" : "pt-1")}>Studies</div>}
                  {row(hit, i)}
                </div>
              ))}
              <div className="mt-1 border-t px-2 pt-1.5 pb-1 text-[11px] text-muted-foreground">↓ ↑ to choose · Enter to open · Esc to close</div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
