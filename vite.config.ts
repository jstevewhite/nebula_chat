import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
// @ts-expect-error node builtin, no @types/node installed
import { execSync } from "node:child_process";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// Build-time metadata injected into the frontend (see src/vite-env.d.ts).
function gitHash(): string {
  // Allow release pipelines to override (e.g. building without a .git dir).
  // @ts-expect-error process is a nodejs global
  const fromEnv = process.env.NEBULA_BUILD_HASH || process.env.GIT_HASH;
  if (fromEnv) return fromEnv;
  try {
    return execSync("git rev-parse --short HEAD").toString().trim();
  } catch {
    return "unknown";
  }
}

const BUILD_GIT_HASH = gitHash();
const BUILD_DATE = new Date().toISOString();

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],

  define: {
    __GIT_HASH__: JSON.stringify(BUILD_GIT_HASH),
    __BUILD_DATE__: JSON.stringify(BUILD_DATE),
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
