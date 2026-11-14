// The pages: the lists framed by the rail and the top bar, or the viewer on its own. When the
// server asks for a login, the sign-in page stands before all of them.

import { lazy, Suspense } from "react";
import { Loader2 } from "lucide-react";
import { AppHeader, AppRail } from "@/components/app-header";
import { useRoute } from "@/lib/route";
import { useSession } from "@/lib/session";
import { PatientsPage } from "@/pages/patients-page";
import { SignInPage } from "@/pages/sign-in-page";
import { StudiesPage } from "@/pages/studies-page";
import { SettingsPage } from "@/pages/settings-page";

// Cornerstone loads only when a series is opened
const ViewerPage = lazy(() => import("@/viewer/viewer-page").then((m) => ({ default: m.ViewerPage })));

export function App() {
  const route = useRoute();
  const session = useSession();
  if (session.status === "loading")
    return (
      <div className="fixed inset-0 grid place-items-center bg-background text-muted-foreground">
        <Loader2 className="size-5 animate-spin" />
      </div>
    );
  if (session.status === "out") return <SignInPage />;
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
    // on a narrow screen the rail lies along the bottom
    <div className="flex h-dvh flex-col-reverse md:flex-row">
      <AppRail route={route} />
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        <AppHeader route={route} />
        <main className="min-h-0 flex-1 overflow-hidden">
          {route.page === "settings" ? (
            <SettingsPage section={route.section} />
          ) : route.page === "patients" ? (
            <PatientsPage open={route.open} />
          ) : (
            <StudiesPage patient={route.page === "studies" ? route.patient : null} open={route.page === "studies" ? route.open : null} />
          )}
        </main>
      </div>
    </div>
  );
}
