// The pages: the lists under the top bar, or the viewer on its own.

import { lazy, Suspense } from "react";
import { Loader2 } from "lucide-react";
import { AppHeader } from "@/components/app-header";
import { useRoute } from "@/lib/route";
import { PatientsPage } from "@/pages/patients-page";
import { StudiesPage } from "@/pages/studies-page";

// Cornerstone loads only when a series is opened
const ViewerPage = lazy(() => import("@/viewer/viewer-page").then((m) => ({ default: m.ViewerPage })));

export function App() {
  const route = useRoute();
  if (route.page === "view")
    return (
      <Suspense
        fallback={
          <div className="dark fixed inset-0 grid place-items-center bg-background text-muted-foreground">
            <Loader2 className="size-5 animate-spin" />
          </div>
        }
      >
        <ViewerPage key={`${route.study}/${route.compare ?? ""}`} study={route.study} series={route.series} compare={route.compare} />
      </Suspense>
    );
  return (
    <div className="flex h-dvh flex-col">
      <AppHeader route={route} />
      <main className="min-h-0 flex-1 overflow-hidden">
        {route.page === "patients" ? <PatientsPage open={route.open} /> : <StudiesPage patient={route.patient} open={route.open} />}
      </main>
    </div>
  );
}
