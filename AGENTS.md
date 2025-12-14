# Repository Guidelines

## Project Structure & Module Organization
- `src/` holds the React 19 UI; shared pieces live in `src/components/` (chat shell, memory panel, settings, tools) with entry points in `src/App.tsx` and `src/main.tsx`.
- `src-tauri/` contains the Rust core (commands, persistence, MCP host); app metadata lives in `src-tauri/tauri.conf.json`.
- `public/` is for static assets; `dist/` is the Vite build output. Config lives at the root (`tailwind.config.js`, `postcss.config.js`, `tsconfig.json`, `vite.config.ts`).

## Build, Test, and Development Commands
- `npm install` — install frontend dependencies.
- `npm run dev` — Vite dev server for the UI only.
- `npm run tauri dev` — run the full Tauri app (use `WEBKIT_DISABLE_DMABUF_RENDERER=1` on some Linux/NVIDIA setups).
- `npm run build` — type-checks and builds the React app for production.
- `npm run preview` — serve the built assets locally.
- From `src-tauri/`, `cargo build --release` builds the Rust core; `cargo test` when Rust tests are added.

## Coding Style & Naming Conventions
- TypeScript + React with functional components; prefer hooks over classes.
- Use PascalCase for components (`ChatInterface.tsx`), camelCase for functions/variables, and kebab-case for asset files.
- Indent with 2 spaces, favor double quotes, and keep imports ordered: libraries → internal components → styles/assets.
- Tailwind is the primary styling tool; keep class lists readable and extract repeated patterns into small components.
- Keep UI logic lean; delegate data access to Tauri commands (`invoke`) and reuse shared utilities where possible.

## Testing Guidelines
- No automated frontend suite yet; at minimum, ensure `npm run build` passes before merging.
- For new React logic, prefer adding component-level tests (`*.test.tsx`) with the chosen runner once introduced; mirror file structure under `src/`.
- For Rust additions, place tests in module `#[cfg(test)]` blocks or `src-tauri/tests/` and run `cargo test`.
- Manually verify chat flows, memory panel visibility, provider toggles, and settings save/load in `npm run tauri dev`.

## Commit & Pull Request Guidelines
- Follow the existing history: imperative, concise subjects with optional prefixes (`feat:`, `fix`, `chore`). Example: `fix: handle missing MCP server config`.
- PRs should include: scope/intent, key changes, testing performed (`npm run build`, manual flows), and any configuration notes.
- Attach screenshots or short clips for UI changes (chat layout, settings forms, tool panel).
- Link issues or tasks when applicable and keep diffs focused; favor small, reviewable PRs.
