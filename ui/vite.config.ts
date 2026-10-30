// The web interface's build: `npm run build` writes dist/, which pacs-server embeds at
// compile time (crates/pacs-server/build.rs). `npm run dev` serves it with hot reload and
// sends API requests to a pacs-server on PACS_DEV_SERVER (default http://127.0.0.1:8080).
import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

const server = process.env.PACS_DEV_SERVER || "http://127.0.0.1:8080";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  // relative asset URLs: the page is served under /ui/, maybe behind a proxy's own prefix
  base: "./",
  build: { target: "es2022", outDir: "dist", assetsInlineLimit: 0, chunkSizeWarningLimit: 8000 },
  worker: { format: "es" },
  optimizeDeps: { exclude: ["@cornerstonejs/dicom-image-loader"], include: ["dicom-parser"] },
  server: {
    proxy: {
      "/config.json": { target: server, rewrite: () => "/ui/config.json" },
      "/api/protocols": { target: server, rewrite: (path) => `/ui${path}` },
      "^/(studies|series|instances|patients|tools|healthz|dicom-web)": server,
    },
  },
});
