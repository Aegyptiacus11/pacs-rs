// The web interface's build: `npm run build` writes dist/, which pacs-server embeds at
// compile time (crates/pacs-server/build.rs). `npm run dev` serves it with hot reload and
// sends API requests to a pacs-server on PACS_DEV_SERVER (default http://127.0.0.1:8080).
import { defineConfig } from "vite";

const server = process.env.PACS_DEV_SERVER || "http://127.0.0.1:8080";

export default defineConfig({
  // relative asset URLs: the page is served under /ui/, maybe behind a proxy's own prefix
  base: "./",
  build: { target: "es2022", outDir: "dist", assetsInlineLimit: 0, chunkSizeWarningLimit: 8000 },
  worker: { format: "es" },
  optimizeDeps: { exclude: ["@cornerstonejs/dicom-image-loader"], include: ["dicom-parser"] },
  server: {
    proxy: {
      "/config.json": { target: server, rewrite: () => "/ui/config.json" },
      "^/(studies|series|instances|patients|tools|healthz|dicom-web)": server,
    },
  },
});
