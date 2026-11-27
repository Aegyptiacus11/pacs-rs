// Study labels as small chips: on a row (a few, then "+n"), and in the study pane, where a
// role with `write` takes one off with its × and puts one on with "Add label": a list of the
// labels in use, or a new one typed.

import { useState, type ReactNode } from "react";
import { Plus, Tag, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { notALabel, useAllLabels } from "@/lib/labels";
import { cn } from "@/lib/utils";

export function LabelChip({ label, children, className }: { label: string; children?: ReactNode; className?: string }) {
  return (
    <Badge variant="secondary" className={cn("h-5 max-w-40 gap-1 rounded-full px-2 text-[11px] font-normal", className)}>
      <span className="truncate">{label}</span>
      {children}
    </Badge>
  );
}

/** A row's labels: the first `max`, then how many more. */
export function RowLabels({ labels, max = 2 }: { labels: string[]; max?: number }) {
  if (!labels.length) return null;
  const more = labels.length - max;
  return (
    <span className="flex shrink-0 items-center gap-1" aria-label={`Labels: ${labels.join(", ")}`} title={labels.join(", ")}>
      {labels.slice(0, max).map((l) => (
        <LabelChip key={l} label={l} />
      ))}
      {more > 0 && <span className="text-[11px] text-muted-foreground">+{more}</span>}
    </span>
  );
}

/** "Add label": the labels in use (but `except`), or a new one typed. */
export function LabelPicker({ except = [], onPick, children }: { except?: string[]; onPick: (label: string) => void; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const all = useAllLabels(open);
  const text = typed.trim();
  const why = notALabel(text);
  const choices = (all.data ?? []).filter((l) => !except.includes(l));
  const isNew = text && !why && !except.includes(text) && !(all.data ?? []).includes(text);
  const pick = (label: string) => {
    setOpen(false);
    setTyped("");
    onPick(label);
  };
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>{children}</PopoverTrigger>
      <PopoverContent className="w-60 p-0" align="start">
        <Command>
          <CommandInput placeholder="A label…" value={typed} onValueChange={setTyped} aria-label="Label" />
          <CommandList>
            {why ? (
              <div role="alert" className="px-3 py-2.5 text-xs text-destructive">
                {why}
              </div>
            ) : (
              !isNew && <CommandEmpty>{text ? "Already on it." : "No label yet: type one."}</CommandEmpty>
            )}
            <CommandGroup>
              {choices.map((l) => (
                <CommandItem key={l} value={l} onSelect={() => pick(l)}>
                  <Tag className="text-muted-foreground" />
                  {l}
                </CommandItem>
              ))}
            </CommandGroup>
            {/* shown whatever the filter keeps of the list above */}
            {isNew && (
              <CommandGroup forceMount>
                <CommandItem value={`new ${text}`} onSelect={() => pick(text)} forceMount>
                  <Plus className="text-muted-foreground" />
                  Create “{text}”
                </CommandItem>
              </CommandGroup>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}

/** A study's labels, each taken off with its ×, and one put on with "Add label"; without
 * `onAdd` (no `write`), shown only. */
export function LabelEditor({ labels, onAdd, onRemove }: { labels: string[]; onAdd?: (label: string) => void; onRemove?: (label: string) => void }) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      {labels.map((l) => (
        <LabelChip key={l} label={l} className="h-6 text-xs">
          {onRemove && (
            <button type="button" aria-label={`Remove the label ${l}`} className="-mr-1 grid size-4 place-items-center rounded-full text-muted-foreground hover:bg-foreground/10 hover:text-foreground" onClick={() => onRemove(l)}>
              <X className="size-3" />
            </button>
          )}
        </LabelChip>
      ))}
      {!labels.length && !onAdd && <span className="text-sm text-muted-foreground">None</span>}
      {onAdd && (
        <LabelPicker except={labels} onPick={onAdd}>
          <Button variant="ghost" size="sm" className="h-6 rounded-full px-2 text-xs text-muted-foreground">
            <Plus /> Add label
          </Button>
        </LabelPicker>
      )}
    </div>
  );
}
