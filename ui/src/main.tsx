import "@fontsource-variable/inter";
import "./index.css";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Toaster } from "sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { UploadProvider } from "@/components/upload";
import { App } from "@/app";
import { restore } from "@/lib/session";
import { applyTheme } from "@/lib/theme";

// before the first paint, so a dark choice never flashes light
applyTheme();

// the session, if this browser has one; the app shows a spinner until it is known
void restore();

const client = new QueryClient({ defaultOptions: { queries: { refetchOnWindowFocus: false, retry: 1 } } });

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <QueryClientProvider client={client}>
      <TooltipProvider delayDuration={300}>
        <UploadProvider>
          <App />
        </UploadProvider>
      </TooltipProvider>
      <Toaster position="bottom-right" toastOptions={{ className: "font-sans" }} />
    </QueryClientProvider>
  </StrictMode>,
);
