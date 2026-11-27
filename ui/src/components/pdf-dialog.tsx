// A study's PDF document, read in the browser's own PDF viewer: the frame loads
// /instances/{id}/pdf, with a download token when there is a login (a frame cannot send the
// session's header).

import { useEffect, useState } from "react";
import { Download, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { DownloadLink } from "@/components/download-link";
import { downloadUrl } from "@/lib/session";

export type OpenDocument = { instance: string; title: string; detail?: string };

export function PdfDialog({ document: doc, onClose }: { document: OpenDocument | null; onClose: () => void }) {
  const [src, setSrc] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setSrc(null);
    setError(null);
    if (!doc) return;
    let gone = false;
    downloadUrl(`instances/${doc.instance}/pdf`).then(
      (u) => !gone && setSrc(u),
      (e: Error) => !gone && setError(e.message),
    );
    return () => {
      gone = true;
    };
  }, [doc?.instance]);
  return (
    <Dialog open={!!doc} onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="flex h-[92vh] w-[min(96vw,1100px)] max-w-none flex-col gap-3 p-4 sm:max-w-none">
        <DialogHeader className="flex-row items-center justify-between gap-4 pr-8">
          <div className="min-w-0">
            <DialogTitle className="truncate">{doc?.title}</DialogTitle>
            <DialogDescription className="truncate">{doc?.detail ?? "PDF document"}</DialogDescription>
          </div>
          {doc && (
            <Button variant="outline" size="sm" asChild>
              <DownloadLink path={`instances/${doc.instance}/pdf`}>
                <Download /> Download
              </DownloadLink>
            </Button>
          )}
        </DialogHeader>
        <div className="relative min-h-0 flex-1 overflow-hidden rounded-md border bg-muted">
          {error ? (
            <p role="alert" className="p-4 text-sm text-destructive">
              The document could not be opened: {error}
            </p>
          ) : src ? (
            <iframe src={src} title={doc?.title ?? "PDF document"} className="size-full" />
          ) : (
            <Loader2 className="absolute top-1/2 left-1/2 size-5 -translate-1/2 animate-spin text-muted-foreground" />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
