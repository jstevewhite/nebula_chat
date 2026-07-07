# Generation-info hover (model + provider + speed)

**Date:** 2026-07-07
**Status:** Approved, ready for implementation plan

## Problem

Each assistant message shows its generation speed inline in the action bar as
`42.1 tok/s` (`ChatInterface.tsx:2471`), with a plain native `title` tooltip
("N tokens in X.Xs"). The user wants to also see **which model produced the
response** (and its provider). Rather than crowd the compact action bar with a
long model id, consolidate all generation metadata into a single hover surface:
**replace the inline tok/s text with a small info icon (ℹ) that reveals a
styled popover card on hover.**

The generation metadata is already in hand: the backend emits a `stream-stats`
event after each streamed response (`lib.rs:1216`), and the enclosing
`send_message` fn has both `model` and `provider_id` in scope (they are already
emitted together elsewhere at `lib.rs:1015-1016`). The response value is a plain
`Message` (provider `chat`/`stream` return `Result<Message>`), so no
provider-reported usage (prompt/completion split, finish reason) is available
without deeper changes — out of scope here.

## Goals

- Surface the **model** and **provider** that produced each assistant response,
  alongside the existing speed/token/duration figures.
- Consolidate all of it into one **hover info popover** triggered by an `Info`
  icon that replaces the inline `tok/s` text.
- Keep the change small and consistent with today's behavior: **live session
  only** — the icon/popover appears for messages generated this session, exactly
  where `tok/s` appears now, and is absent on reloaded/historical messages.

## Non-goals (YAGNI)

- **No persistence.** No DB column, no survival across reload. (This is option A
  from brainstorming; the durable per-response metadata + regeneration "swipes"
  idea is a separate, larger future effort.)
- **No provider-reported usage.** Prompt/completion token split and finish
  reason are not available from the `Message` return type; the token figure
  stays a local tokenizer estimate of the output, shown simply as "Tokens".
- **No inline model text.** The model is shown only in the hover; the bar keeps
  just the icon. (That is the intentional trade for a cleaner bar.)
- No change to non-streamed responses (the `stream-stats` event only fires for
  streamed generations today; unchanged).

## Design

### Backend (`src-tauri/src/lib.rs`)

- Add two fields to `StreamStatsEvent` (struct ~134):
  - `model: String`
  - `provider: String`
- Populate them at the emission site (~1216) from the in-scope `model` and
  `provider_id`: `model: model.clone(), provider: provider_id.clone()`. No other
  backend change; `total_tokens`, `duration_ms`, `tokens_per_second` unchanged.

### Frontend (`src/components/ChatInterface.tsx`)

- Extend the `StreamStatsEvent` interface (~161) with `model: string` and
  `provider: string`.
- Add a small, self-contained presentational component (in this file, next to
  `ChatMessage`) — e.g. `GenerationInfo({ stats }: { stats: StreamStatsEvent })`:
  - Renders a muted lucide `Info` icon (size ~14, same `text-[var(--color-text-tertiary)]`
    treatment as the current tok/s span and neighboring action buttons).
  - On hover (and focus, for basic a11y), shows an absolutely-positioned popover
    **card** anchored to the icon, themed with existing CSS vars
    (`--color-bg-secondary`/`--color-border-primary`/`--color-text-*`), with
    labeled rows:
    - **Model** — `stats.model`
    - **Provider** — `stats.provider`
    - **Speed** — `stats.tokens_per_second.toFixed(1)` tok/s
    - **Tokens** — `stats.total_tokens`
    - **Duration** — `(stats.duration_ms / 1000).toFixed(1)` s
  - The icon carries a plain `title`/`aria-label` (e.g. "Generation info") as a
    fallback for the hover card.
- In the action bar (~2466), **replace** the existing
  `{m.role === "assistant" && genStats && (<span>…tok/s</span>)}` block with
  `{m.role === "assistant" && genStats && <GenerationInfo stats={genStats} />}`.
  The render condition is unchanged, so behavior parity holds: no stats → no
  icon.

### Optional nicety (not required)

The provider row could show the provider emoji already used in the model
selector (`getProviderIcon`) next to the provider name. Include only if trivial;
not part of acceptance.

## Data flow

1. `send_message` finishes a streamed response → emits `stream-stats` now
   carrying `model` + `provider` in addition to the existing figures.
2. The existing `stream-stats` listener stores the event in `messageStats`
   keyed by message id (unchanged).
3. The assistant message's action bar renders `<GenerationInfo>` when
   `messageStats[m.id]` exists; hovering the icon shows the card.

## Error handling / edge cases

- **Missing/empty `model` or `provider`** (older event shape, or an edge where
  they're blank): the row renders the raw value; an empty string shows an em
  dash or is omitted. No crash.
- **No stats for a message** (historical, non-streamed): no icon, as today.
- **Long model ids** live in the popover, not the bar, so they can't overflow
  the action row; the card should `white-space: nowrap` per row or wrap
  gracefully.

## Testing

- **Backend:** the `StreamStatsEvent` change is compile-checked by `cargo build`;
  no logic to unit-test (it forwards two in-scope strings). Confirm the full
  suite still passes.
- **Frontend:** no test suite (per CLAUDE.md). Verify `npm run build` succeeds,
  then manual smoke test via `npm run tauri dev`: generate a response, confirm
  the tok/s text is replaced by an `Info` icon, hover shows the card with Model,
  Provider, Speed, Tokens, Duration, and the values are correct;
  reloaded/historical messages show no icon.
