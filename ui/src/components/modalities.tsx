// A study's or patient's modalities as small code badges.

import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export function Modalities({ list, className }: { list: string[]; className?: string }) {
  return (
    <div className={cn("flex flex-wrap gap-1", className)}>
      {list.map((m) => (
        <Badge key={m} variant="outline" className="h-5 rounded px-1.5 font-mono text-[11px] font-medium text-muted-foreground">
          {m}
        </Badge>
      ))}
    </div>
  );
}
