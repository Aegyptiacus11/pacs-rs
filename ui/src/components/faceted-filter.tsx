// A filter chip: a dashed "+ Title" button opening a searchable checklist, each option with
// how many rows have it, the picked ones shown on the chip, each with an × that takes it off
// (or one × for all, when they are many). `single` makes it pick one.

import { Check, PlusCircle, X } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Command, CommandEmpty, CommandGroup, CommandInput, CommandItem, CommandList, CommandSeparator } from "@/components/ui/command";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Separator } from "@/components/ui/separator";
import { cn } from "@/lib/utils";

export type FacetOption = { value: string; label: string; hint?: string; count?: number };

// an × inside the chip: takes off what it sits on, without opening the list
function Remove({ label, onRemove }: { label: string; onRemove: () => void }) {
  return (
    <span
      role="button"
      tabIndex={-1}
      aria-label={label}
      className="-mr-0.5 ml-0.5 grid size-4 place-items-center rounded-sm text-muted-foreground hover:bg-foreground/10 hover:text-foreground"
      onPointerDown={(e) => e.stopPropagation()}
      onClick={(e) => {
        e.stopPropagation();
        e.preventDefault();
        onRemove();
      }}
    >
      <X className="size-3" />
    </span>
  );
}

export function FacetedFilter({
  title,
  options,
  selected,
  onChange,
  single = false,
  searchable = true,
}: {
  title: string;
  options: FacetOption[];
  selected: string[];
  onChange: (values: string[]) => void;
  single?: boolean;
  searchable?: boolean;
}) {
  const picked = new Set(selected);
  const toggle = (value: string) => {
    if (single) return onChange(picked.has(value) ? [] : [value]);
    const next = new Set(picked);
    next.has(value) ? next.delete(value) : next.add(value);
    onChange([...next]);
  };
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button variant="outline" size="sm" className={cn("h-8 bg-card", picked.size === 0 && "border-dashed")}>
          <PlusCircle />
          {title}
          {picked.size > 0 && (
            <>
              <Separator orientation="vertical" className="mx-1 h-4" />
              {picked.size > 2 ? (
                <Badge variant="secondary" className="gap-0 rounded-sm px-1 font-normal">
                  {picked.size} chosen
                  <Remove label={`Clear ${title.toLowerCase()}`} onRemove={() => onChange([])} />
                </Badge>
              ) : (
                options
                  .filter((o) => picked.has(o.value))
                  .map((o) => (
                    <Badge key={o.value} variant="secondary" className="gap-0 rounded-sm px-1 font-normal">
                      {o.label}
                      <Remove label={`Remove ${o.label}`} onRemove={() => toggle(o.value)} />
                    </Badge>
                  ))
              )}
            </>
          )}
        </Button>
      </PopoverTrigger>
      <PopoverContent className={cn("p-0", options.some((o) => o.hint) ? "w-80" : "w-52")} align="start">
        <Command>
          {searchable && <CommandInput placeholder={title} />}
          <CommandList>
            <CommandEmpty>Nothing found.</CommandEmpty>
            <CommandGroup>
              {options.map((o) => {
                const on = picked.has(o.value);
                return (
                  <CommandItem key={o.value} value={`${o.label} ${o.hint ?? ""}`} onSelect={() => toggle(o.value)} className={cn(o.count === 0 && !on && "text-muted-foreground")}>
                    <span
                      className={cn(
                        "flex size-4 items-center justify-center border border-primary",
                        single ? "rounded-full" : "rounded-[4px]",
                        on ? "bg-primary text-primary-foreground" : "opacity-50 [&_svg]:invisible",
                      )}
                    >
                      <Check className="size-3.5 text-primary-foreground" />
                    </span>
                    <span className={cn(o.hint && "w-12 font-mono text-xs font-medium")}>{o.label}</span>
                    {o.hint && <span className="truncate text-xs text-muted-foreground">{o.hint}</span>}
                    {o.count != null && <span className="ml-auto pl-2 font-mono text-xs text-muted-foreground tabular">{o.count || ""}</span>}
                  </CommandItem>
                );
              })}
            </CommandGroup>
            {picked.size > 0 && (
              <>
                <CommandSeparator />
                <CommandGroup>
                  <CommandItem onSelect={() => onChange([])} className="justify-center text-center">
                    Clear filter
                  </CommandItem>
                </CommandGroup>
              </>
            )}
          </CommandList>
        </Command>
      </PopoverContent>
    </Popover>
  );
}
