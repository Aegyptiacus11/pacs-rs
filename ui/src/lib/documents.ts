// PDF documents of a study: Encapsulated PDF instances, each in a series of modality DOC.
// One is added with POST /studies/{id}/documents (the PDF as the body) and read back from
// /instances/{id}/pdf.

import { api } from "@/lib/api";

/** A series of documents (modality DOC). */
export const isDocument = (series: any) => series?.MainDicomTags?.Modality === "DOC";

/** Why `file` cannot be added as a PDF, if it cannot: read before anything is sent. */
export async function notAPdf(file: File): Promise<string | null> {
  if (!file.size) return `${file.name} is empty`;
  const head = new TextDecoder("latin1").decode(await file.slice(0, 5).arrayBuffer());
  return head === "%PDF-" ? null : `${file.name} is not a PDF`;
}

/** The PDF added to the study, titled by its file's name less ".pdf". */
export async function addDocument(study: string, file: File) {
  const title = file.name.replace(/\.pdf$/i, "").trim();
  return api<{ ID: string; ParentSeries: string }>(`studies/${study}/documents?title=${encodeURIComponent(title)}`, {
    method: "POST",
    headers: { "content-type": "application/pdf" },
    body: file,
  });
}
