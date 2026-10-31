// The one search: a box at the top of a list that filters it as words are typed and, under
// it, suggests the patients whose name or ID has them (↓ ↑ and Enter, or a click, opens the
// patient). Ctrl K (⌘K on a Mac) or / reaches it from anywhere: the rail's handler focuses
// whichever box carries `data-search`.

import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { initials, personName, plural, sexAge } from "@/lib/api";
import { usePatientSuggestions, type Patient } from "@/lib/studies";
import { cn } from "@/lib/utils";

export const SEARCH_KEY = /Mac|iPhone|iPad/.test(navigator.platform) ? "⌘K" : "Ctrl K";

export function SearchBar({
  value,
  onChange,
  placeholder,
  onPatient,
  suggest = true,
  className,
}: {
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  /** a suggested patient picked */
  onPatient?: (p: Patient) => void;
  suggest?: boolean;
  className?: string;
}) {
  const [focused, setFocused] = useState(false);
  const [asked, setAsked] = useState("");
  const [at, setAt] = useState(-1);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const t = setTimeout(() => setAsked(value), 200);
    return () => clearTimeout(t);
  }, [value]);
  useEffect(() => setAt(-1), [asked]);
  const suggestions = usePatientSuggestions(suggest && focused ? asked : "");
  const list = suggest && focused && value.trim() ? (suggestions.data ?? []) : [];
  const pick = (p: Patient) => {
    setFocused(false);
    input.current?.blur();
    onPatient?.(p);
  };
  return (
    <div className={cn("relative", className)}>
      <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-muted-foreground" />
      <Input
        ref={input}
        data-search
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onFocus={() => setFocused(true)}
        onBlur={() => setFocused(false)}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" && list.length) {
            e.preventDefault();
            setAt((i) => Math.min(list.length - 1, i + 1));
          } else if (e.key === "ArrowUp" && list.length) {
            e.preventDefault();
            setAt((i) => Math.max(-1, i - 1));
          } else if (e.key === "Enter" && at >= 0 && list[at]) {
            e.preventDefault();
            pick(list[at]);
          } else if (e.key === "Escape") {
            if (value) onChange("");
            else input.current?.blur();
          }
        }}
        placeholder={placeholder}
        aria-label={placeholder}
        role="combobox"
        aria-expanded={list.length > 0}
        aria-controls="search-suggestions"
        aria-activedescendant={at >= 0 ? `suggestion-${at}` : undefined}
        className="h-9 bg-card pr-16 pl-9"
      />
      {value ? (
        <button
          type="button"
          aria-label="Clear the search"
          className="absolute top-1/2 right-2 grid size-6 -translate-y-1/2 place-items-center rounded text-muted-foreground hover:bg-accent hover:text-foreground"
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => onChange("")}
        >
          <X className="size-3.5" />
        </button>
      ) : (
        <Kbd className="pointer-events-none absolute top-1/2 right-2 -translate-y-1/2">{SEARCH_KEY}</Kbd>
      )}
      {list.length > 0 && (
        <div
          id="search-suggestions"
          role="listbox"
          aria-label="Patients"
          className="absolute inset-x-0 top-full z-30 mt-1 overflow-hidden rounded-lg border bg-popover p-1 shadow-lg animate-in fade-in-0 zoom-in-95"
          onMouseDown={(e) => e.preventDefault()}
        >
          <div className="px-2 pt-1 pb-1.5 text-xs font-medium text-muted-foreground">Patients</div>
          {list.map((p, i) => (
            <button
              key={p.id}
              id={`suggestion-${i}`}
              role="option"
              aria-selected={i === at}
              onMouseEnter={() => setAt(i)}
              onClick={() => pick(p)}
              className={cn("flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-left text-sm", i === at && "bg-accent")}
            >
              <span className="grid size-7 shrink-0 place-items-center rounded-full bg-secondary text-[11px] font-medium text-muted-foreground ring-1 ring-border">{initials(p.name)}</span>
              <span className="min-w-0 flex-1">
                <span className="block truncate font-medium">{personName(p.name)}</span>
                <span className="block truncate text-xs text-muted-foreground">{[p.patientId, sexAge(p.sex, p.birth)].filter(Boolean).join(" · ")}</span>
              </span>
              <span className="shrink-0 text-xs text-muted-foreground">{plural(p.studies, "study", "studies")}</span>
            </button>
          ))}
          <div className="border-t px-2 pt-1.5 pb-1 text-[11px] text-muted-foreground">↓ ↑ and Enter open a patient; the list below shows the studies</div>
        </div>
      )}
    </div>
  );
}
