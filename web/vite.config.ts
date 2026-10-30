import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// Spotify sends the browser back to http://127.0.0.1:8888/auth/callback, so
// the dev server takes that address and forwards the backend's paths to it.
const backend = "http://127.0.0.1:8889";

export default defineConfig({
  plugins: [react()],
  server: {
    host: "127.0.0.1",
    port: 8888,
    strictPort: true,
    proxy: {
      "/api": backend,
      "/auth": backend,
      "/openapi.json": backend,
      "/docs": backend,
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["src/test/setup.ts"],
    restoreMocks: true,
  },
});
