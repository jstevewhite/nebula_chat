//! Skills shipped with the app. These are re-materialised to
//! `<skills_dir>/built-ins/<slug>.md` on every startup — the binary is the
//! source of truth, and any edits to files in `built-ins/` are overwritten
//! on next launch. To customise a built-in, create a top-level user skill
//! with the same slug; the user skill is returned first by `get()` and
//! shadows the built-in.
//!
//! Each entry's `built_in: true` frontmatter lets the UI render a "BUILT IN"
//! badge so users can see at a glance which skills are app-shipped (and
//! therefore will be reset on update).

/// (slug, file body). Bodies are the full markdown file including YAML
/// frontmatter — kept human-readable in one place so it's obvious what each
/// skill does.
pub const ALL: &[(&str, &str)] = &[
    ("code-review", CODE_REVIEW),
    ("summarize-conversation", SUMMARIZE),
    ("debug-help", DEBUG_HELP),
    ("skill-architect", SKILL_ARCHITECT),
    ("mcp-diagnostics", MCP_DIAGNOSTICS),
    ("explain-code", EXPLAIN_CODE),
    ("readme-pro", README_PRO),
    ("nebula-setup", NEBULA_SETUP),
];

const CODE_REVIEW: &str = r#"---
name: Code Review
description: Use when the user asks for a code review, wants feedback on a diff, or asks "what's wrong with this?". Hunts correctness, security, and maintainability issues in priority order, calibrated to change size and type.
built_in: true
---

The user wants a code review. Approach as a senior reviewer who respects
the author's time: lead with what would block merging, defer style nits,
skip anything that doesn't change the disposition of the change.

## Step 1 — Get the change

You need to see *what changed*, not just the final state. Access paths
in order of preference:

1. **Diff or patch pasted directly.** Use as-is.
2. **PR or commit URL + web-fetch tool available.** Fetch it.
3. **File path + filesystem-style MCP tool.** Read the file *and* try
   to get the diff (e.g. `git diff` via a shell tool if available).
   Reviewing final state without seeing what changed is half-blind.
4. **Final state only, no diff.** Say so to the user, then review the
   whole file as written — and flag that you can't tell new bugs from
   pre-existing ones.

If you can't see the change at all, ask for it. Don't review by
guessing what they probably wrote.

## Step 2 — Classify the change

Different change types warrant different lenses:

- **Bug fix** — does the fix address the *root cause* or paper over the
  symptom? Is there a regression test? Could the same bug exist in
  adjacent code that wasn't touched?
- **New feature** — error paths, edge cases, and the public-API surface
  decisions. API shape matters more than implementation details.
- **Refactor** — behavior should not change. Hunt for subtle semantic
  drift: default values, exception types, ordering, side-effect timing.
- **Performance** — measurements before/after? Or speculation?
- **Tests only** — different review entirely; check coverage of edge
  cases, not internal style.
- **Docs only** — quick correctness/clarity pass.
- **Mixed** — ask the author to split, *unless* the changes are
  genuinely interdependent.

If the change is generated (formatter output, codegen, vendored deps),
say so and skip detailed review of the generated portion.

## Step 3 — Read the whole change first

Skim the entire diff before commenting on any single line. You'll miss
load-bearing context if you start commenting at line 1.

While reading, note:
- Anything that surprised you (subtle semantic shifts, unusual choices)
- What the change is *trying* to accomplish
- What it *doesn't* touch that you'd expect it to (missing callsite
  update, missing test, missing migration, missing changelog entry)

## Step 4 — Hunt by priority

Work top-down. Stop at any tier if the change is too small to warrant
deeper review.

### Tier 1 — Correctness (blockers)

- Off-by-one, null/None/empty, integer overflow
- Race conditions, shared mutable state, lock ordering
- Error paths: what happens when the call below fails? Swallowed,
  propagated, transformed correctly?
- Return-value semantics: typed Result vs. thrown exception, partial-
  failure modes
- Edge inputs: empty, max, malformed, NaN, infinity, unicode
- Resource leaks: file handles, connections, goroutines/tasks
- Concurrent mutation of supposedly-local state
- Tests exercise only the happy path
- Missing callsite update when a signature changed
- Migration / schema change without a backfill or rollback story

### Tier 2 — Security

- Injection: SQL, shell, path traversal, template, deserialization
- Auth / authz: check at the right layer, before the action, with the
  right principal
- Secrets in code, logs, error messages, or test fixtures
- Input validation at trust boundaries; output escaping at output
  boundaries
- DIY crypto, weak primitives, hardcoded keys / IVs
- Resource exhaustion: unbounded loops, allocations, recursion depth
- Untrusted data deserialized into rich object types
- TLS / cert verification disabled "temporarily"

### Tier 3 — Maintainability

- Naming inconsistent with surrounding code
- Premature or leaky abstraction; abstraction at the wrong layer
- Comments explaining *what* instead of *why* (the rare case for *why*
  comments: hidden constraints, workarounds, subtle invariants)
- Dead code, unused parameters, commented-out blocks
- Logging at the wrong level, PII in logs, or log spam
- Public API surface wider than it needs to be; types looser than they
  could be
- A function or file fighting its container (too big, doing too much)

### Tier 4 — Style and nits

Only if nothing higher-priority is on the table. Label explicitly as
nits or preferences, not findings. Defer to the project's formatter and
lint config — don't relitigate settled questions.

## Step 5 — Match grain to change size

- One-line fix → one-line review. Don't pad.
- 50-line change → 2–5 grouped findings, terse.
- Multi-hundred-line change → structured output (see Step 6).
- "Looks good — nothing to flag" is a complete response when accurate.
  Inventing concerns to look thorough is worse than no review.

## Step 6 — Output format

For non-trivial reviews, use this shape:

```
**Verdict:** [Block / Approve with nits / Approve / Approve, follow-up suggested]

**Critical** (must fix before merge)
- `path/to/file.rs:42` — [what's wrong] — [why it matters] — [suggested fix]

**Major** (should fix; can be follow-up if scope is tight)
- ...

**Minor / nits** (taste)
- ...

**Bottom line:** One sentence.
```

Omit any severity bucket that's empty. Don't print headings just to fill
them.

## Guardrails

- Never review unchanged code. The diff is the unit of review.
- Never restate what the diff does. The author knows.
- Never pad with praise of unchanged code or preamble like "I'll now
  review your change".
- Never suggest a refactor that expands PR scope unless it's load-
  bearing for correctness or security. File a follow-up issue instead.
- Never bikeshed style while a correctness or security issue is
  unaddressed.
- Never claim a bug exists without naming the file, the line, and the
  input that triggers it.
- Never invent concerns. "Nothing to flag" is a real answer.
"#;

const SUMMARIZE: &str = r#"---
name: Summarize Conversation
description: Use when the user asks for a recap, summary, or "what have we covered" for the current chat. Produces a structured summary with topics, decisions, and open questions.
built_in: true
---

Produce a concise structured summary of the conversation so far. Format:

**Topics covered**
- one-line bullet per distinct topic

**Decisions / conclusions**
- only things the user actually decided or you concluded together

**Open questions**
- anything raised but not resolved

**Action items** (only if any were mentioned)
- bulleted

Keep the whole thing under ~300 words. Skip preamble, skip closing fluff.
If a section has no content, omit the heading entirely.
"#;

const DEBUG_HELP: &str = r#"---
name: Debug Help
description: Use when the user is stuck on a bug, error, or unexpected behavior. Walks through systematic debugging without jumping to solutions.
built_in: true
---

The user is debugging something. Resist the urge to guess the fix immediately
— you'll do better by helping them isolate the problem. Walk through:

1. **What is the actual observed behavior?** Get the user to state it
   precisely — not "it's broken" but "X returns Y when I expected Z".
2. **What is the smallest reproduction?** If they're running a whole pipeline,
   can it be reduced to one function call?
3. **What changed last?** Bugs usually correlate with a recent change. Ask.
4. **What does the layer below say?** Logs, stack traces, network tab,
   `strace`, `RUST_LOG=debug`, browser console — whichever applies. If the
   user hasn't checked, push them to.
5. **State your hypothesis explicitly** before testing it. "I think X because
   Y; if so, then Z should also be true. Let's check Z."
6. **One change at a time** when trying fixes. Reverting becomes impossible
   otherwise.

Only after isolation should you propose a fix. If the user already knows the
fix and just wants confirmation, skip steps 1-4.
"#;

const SKILL_ARCHITECT: &str = r#"---
name: Skill Architect
description: Use when the user wants to design a new skill, formalize a vague capability into a reusable skill, or upgrade a weak skill into a professional-grade one. Runs a structured interview → draft → refine loop.
built_in: true
---

You are a meta-skill: your job is to help the user design *another* skill.
Treat this as a consulting engagement, not a one-shot generation. The output
is a finished skill definition the user will paste into their skills library.

## When this skill applies

- The user wants to create a new specialized skill.
- The user has a vague capability ("I want a skill that helps me write
  emails") that needs to be narrowed and formalized.
- The user wants to convert a manual workflow into a repeatable instruction
  set.

## When to bail out

- The request is a one-off task ("write me an email" — just write it).
- The request is general chat or creative writing.
- The request is a single atomic action ("summarize this paragraph").

If the request is a bail-out case, say so and offer to just do the task
directly instead of architecting a skill for it.

## Phase 1 — Discovery interview

Before drafting anything, extract the following from the user. Ask at most
3–5 targeted questions per turn, then wait for their response. Don't draft
the skill until you have answers (or explicit "skip it") for each:

1. **Core capability** — What is the single most important thing this skill
   must achieve? (Push back if the answer is broad: "a coding skill" is too
   vague; "a Python refactoring skill" is workable.)
2. **Trigger** — What context, phrase, or user intent should signal that
   this skill is needed?
3. **Input / output contract** — What goes in? What does a perfect result
   look like? (Ask for a concrete example if they can give one.)
4. **Guardrails** — What are the "never" rules? (e.g. "never suggest a new
   dependency", "never change the user's tone".)
5. **Success metric** — How will the user know the skill performed well?

## Phase 2 — Structural design

Once the interview is done, translate the answers into structure:

- **Narrow the scope.** If anything is still broad, push the user to pick a
  specific variant before drafting.
- **Decompose the workflow** into a logical sequence (e.g. Analyse →
  Validate → Execute → Review). Each phase should have a clear entry and
  exit condition.
- **Encode decision logic.** Write explicit if/then rules for the edge
  cases the interview surfaced — don't leave them to the model's judgment.

## Phase 3 — Draft and feedback loop

Produce the first draft in the **Output Format** below. After presenting
it, ask the user *exactly* this question:

> Does this capture the nuance you were looking for, or should we tighten
> the scope or rules in a specific area?

Iterate until the user is satisfied. Each revision should be a full
re-draft, not a diff — the user is going to paste the final version
somewhere, so they need to see the whole thing.

## Phase 4 — Finalization

Before declaring done, sweep the text for:

- **Imperative voice.** "Analyse the input for X" beats "you should try to
  analyse the input for X". Remove hedges.
- **Concrete triggers.** The "Use when" line should let another agent
  decide in one read whether this skill applies.
- **No dead phases.** If a phase has no actual instructions, cut it.

## Output format

Present the draft using this template, verbatim structure:

```
---
name: [Name]
description: Use when [specific trigger]. [One-line summary of capability.]
---

## When this skill applies
- [Trigger condition]
- [Trigger condition]

## When to bail out
- [Adjacent task this is NOT for]
- [Adjacent task this is NOT for]

## Instructions

### [Phase 1 name]
[Imperative step-by-step instructions]

### [Phase 2 name]
[Imperative step-by-step instructions]

## Guardrails
- [Never rule]
- [Never rule]

## Output format
[Explicit structure of the final response the skill produces]

## Success criteria
[How the skill validates its own work]
```

## Guardrails

- Never skip the interview, even if the user seems impatient. A skill built
  on assumptions is worse than no skill — it will fail silently.
- Never invent guardrails the user didn't ask for. If a guardrail seems
  obviously needed, raise it as a question in Phase 1, don't bake it in
  unilaterally.
- Never produce a skill broader than the user asked for. Narrow scope is a
  feature.
"#;

const MCP_DIAGNOSTICS: &str = r#"---
name: MCP Diagnostics
description: Use when an MCP server won't connect, tools aren't appearing, tool calls are failing, or the model isn't invoking tools it should. Walks layer-by-layer from settings → transport → advertisement → invocation.
built_in: true
---

The user has an MCP problem. Resist the urge to guess — MCP failures look
identical at the chat surface ("the tool didn't work") but originate at one
of four distinct layers. Isolate the layer before proposing fixes.

## Step 1 — Pin down the symptom

Get the user to state which of these they're seeing. Don't move on until
you know:

- **A.** Server isn't connecting at all (red dot, error in settings).
- **B.** Server connects but no tools appear in the available list.
- **C.** Tools appear but the model never calls them.
- **D.** Model calls a tool but it fails, times out, or returns garbage.
- **E.** Tool approval prompt never fires (or fires every time despite
  "always allow").

The fix lives in a different layer for each symptom. Misdiagnosing here
wastes the rest of the session.

## Step 2 — Diagnose by layer

### Layer 1 — Settings (symptoms A, B, E)

Check `settings.json` (Linux: `~/.config/nebula/settings.json`).

- Is the server entry present under `mcp_servers`?
- Is `enabled: true`?
- For `Stdio`: does `command` resolve on the user's PATH? (Ask them to run
  the exact command + args in a terminal — it must start without error.)
- For `Sse`: is the URL reachable? Any auth headers configured?
- If they edited via the UI: did the settings page hang or error on save?
  (Known historical issue — see CLAUDE.md.)

### Layer 2 — Transport / process (symptom A)

- For `Stdio`: the server runs as a child process. If it crashes on init,
  `McpManager` logs the failure but the app continues. Ask the user to
  check the app logs / dev console.
- Common stdio failures: missing runtime (`npx`, `python`), missing deps,
  path arg pointing at a directory the server can't read, server printing
  to stdout instead of using JSON-RPC framing.
- For `Sse`: 4xx = auth/URL wrong; 5xx = server-side; timeout = network or
  the server isn't actually an MCP endpoint.
- Have the user toggle the server off and on in settings to force a fresh
  spawn — but only after gathering logs from the failing run.

### Layer 3 — Tool advertisement (symptom B)

- Server connected but no tools = server isn't responding to
  `tools/list` correctly, or the server genuinely exposes no tools.
- Confirm by running the same server outside nebula (most reference
  servers print their tool list on `--help` or first JSON-RPC exchange).
- Watch for tool-name collisions: if two servers expose the same tool
  name, behavior depends on `McpManager` merge order. Rename one.

### Layer 4 — Invocation (symptoms C, D)

For **C (model not calling)**:
- Is the tool description specific enough? Vague descriptions
  ("does stuff with files") get skipped.
- Provider matters: OpenAI uses `tools:` array, Anthropic uses the same
  but with different schema requirements. If only one provider is
  affected, suspect schema translation in `llm/<provider>.rs`.
- System prompt: does the user have a custom system prompt that overrides
  the tool-use guidance?

For **D (tool failing)**:
- Get the actual error. The frontend may swallow some MCP errors — push
  the user to look at the Rust logs.
- Reproduce the call by hand if possible (same args, same server, outside
  nebula).
- Tool returning the *wrong* answer is usually a server bug, not a nebula
  bug — confirm by calling the server directly.

### Layer 5 — Approval flow (symptom E)

Nebula has two layers of auto-approve, both surfaced as shield icons in
the Tools panel:

- **Server-level auto-approve** (shield next to the server name) sets
  `auto_approve: true` on the server config; every tool from that
  server skips the approval prompt.
- **Per-tool auto-approve** (shield next to an individual tool,
  hover-revealed) adds the tool name to the server's
  `auto_approve_tools` list. Locked out when server-level is on.

Diagnostic questions:

- "Always allow" not behaving as expected? Confirm which level the user
  toggled — they overlap but resolve at different layers.
- If approval never fires for a write/execute tool: check whether
  server-level auto-approve is on; if so, that's the culprit. Turn it
  off and use per-tool instead.
- If approval *always* fires despite a toggle: the persisted state may
  not be saving. Read `settings.json` and verify `auto_approve` or
  `auto_approve_tools` actually contains the expected value.

## Step 3 — Propose the fix

Only after you've named the layer, propose a fix. State your hypothesis
explicitly first: "I think this is Layer 2 (transport) because [evidence].
The fix is [X]. If it's actually Layer 3, we'd see [Y] instead — let's
verify before changing anything."

One change at a time. MCP failures compound — fixing settings while also
swapping servers makes it impossible to attribute the next outcome.

## Guardrails

- Don't suggest "restart the app" as a first move. It usually masks the
  root cause and the bug recurs.
- Don't recommend disabling approval prompts to "make it work" — that's a
  security regression, not a fix.
- Don't blame the model for not calling a tool until you've checked the
  tool description and provider schema. The model is usually right to skip
  a poorly-described tool.
"#;

const EXPLAIN_CODE: &str = r#"---
name: Explain Code
description: Use when the user wants a walkthrough of unfamiliar code — a pasted snippet, an attached file, or a path they want explained. Focuses on intent and structure, not syntax tutorial.
built_in: true
---

The user wants to understand code they didn't write (or wrote a while
ago). Your job is to make the *purpose and shape* of the code legible —
not to narrate every line.

## Step 1 — Get the code

Three access paths, in order of preference:

1. **Path with file-reading tool available.** If the user gave a path and
   a filesystem-style MCP tool is in your available tools list (anything
   that can read a file by path), use it directly. Don't ask permission
   to read what they explicitly handed you.
2. **Path without a file-reading tool.** Tell the user plainly: "I don't
   have filesystem access here — paste the contents, attach the file, or
   set up a filesystem MCP server." Don't pretend the path works.
3. **Pasted or attached code.** Use what they gave you. If it's an image
   of code, OCR it mentally but flag any ambiguous characters
   (`l` vs `1`, `O` vs `0`) before relying on them.

If they gave you a *directory* path, ask which file(s) they want
explained before reading the whole tree.

## Step 2 — Calibrate the audience

Before writing the walkthrough, infer or ask:

- How familiar are they with the **language**? (Skip "this is a closure"
  for a senior dev; spell it out for a beginner.)
- How familiar are they with the **domain**? (HTTP middleware vs. signal
  processing vs. game loops — domain shapes which parts are surprising.)
- Are they trying to **modify**, **debug**, or just **read** this code?
  The answer changes what to emphasise.

If unclear, default to: assume language fluency, no domain context,
reading-not-modifying. One calibration question is fine; three is a
stall.

## Step 3 — Walk through, in priority order

Cover in this order. Stop as soon as the user has what they need:

1. **What it does, in one sentence.** Not "this function does X, Y, and Z"
   — the single purpose. If you can't name it in one sentence, the code
   is doing too much and that itself is the explanation.
2. **Shape.** Inputs, outputs, side effects. What gets called, what gets
   mutated, what gets returned. A signature-level view before any
   line-level detail.
3. **Key control flow.** The two or three branches or loops that carry
   the actual logic. Skip boilerplate (imports, trivial getters, type
   declarations) unless the user asks.
4. **Non-obvious bits.** Anything a competent reader would pause at: a
   workaround, an unusual API choice, a subtle invariant, a comment
   that hints at history. These are the parts worth your token budget.
5. **What's missing.** If the code has no error handling for an obvious
   failure, or relies on caller-side validation, say so — but as
   observation, not criticism.

## Step 4 — Stop before drowning them

Resist line-by-line narration. If the code is 200 lines and you've used
600 words, you're probably explaining too much. Offer a follow-up
("want me to drill into the retry logic specifically?") instead of
pre-emptively covering everything.

## Guardrails

- Never narrate syntax the user obviously knows. If they pasted Python,
  don't explain what `def` means.
- Never invent context that isn't in the code. If a function name suggests
  a purpose the body doesn't deliver on, flag the mismatch — don't paper
  over it.
- Never recommend changes unless asked. This skill is *explain*, not
  *review*. If you spot a real bug, mention it once at the end as a
  side note, then stop.
- If the code calls into something you can't see (an imported function,
  an external API), say "this depends on X which I don't have visibility
  into" rather than guessing its behavior.
"#;

const README_PRO: &str = r#"---
name: README Pro
description: Use when the user wants to write, generate, audit, or improve a project README. Produces a README sized to the project type (library, CLI, service, app) — not a one-size-fits-all template.
built_in: true
---

The user wants a README written or audited. Default behavior: size the
README to the project. A 50-line CLI tool gets a 50-line README, not a
500-line corporate template. Steamroll the existing voice and you've
made the README worse than it was.

## Step 1 — Mode and access

Decide the mode from the user's request:

- **Generate** — no README, or they want a rewrite from scratch.
- **Audit** — there's an existing README and they want critique or
  targeted improvements.

For access:

- If a filesystem-style MCP tool is available, read at minimum:
  `README.md` (if any), the manifest (`package.json`, `Cargo.toml`,
  `pyproject.toml`, `go.mod`, etc.), the entry point, `.env.example`,
  `LICENSE`, and any `CONTRIBUTING.md` or `docs/`.
- If no filesystem access, tell the user plainly and ask them to paste
  the relevant files (or, minimum: description, language/runtime,
  install steps, one usage example, license). Don't fabricate details
  you can't see — flag unknowns as `<!-- TODO -->` and list them at the
  end.

## Step 2 — Classify the project

Different project types have different essential sections. Pick the row
that matches; mix rows if the project is hybrid (e.g. library + CLI).

| Project type        | Include                                                       | Skip                              |
|---------------------|---------------------------------------------------------------|-----------------------------------|
| Library / package   | Install, Usage example, API summary, License                  | Deployment, Architecture diagram  |
| CLI tool            | Install, Commands & flags, Usage examples, License            | API Reference, Deployment         |
| Service / API       | Install, Configuration, API endpoints, Deployment, License    | (keep all)                        |
| Full-stack app      | All sections                                                  | (none)                            |
| Internal / private  | About, Setup, Structure                                       | Roadmap, Contributing, Acks       |

## Step 3 — Match the existing voice

If the project has any prose at all (current README, CONTRIBUTING, docs),
match its tone — terse vs. expansive, formal vs. casual, emoji vs. none.
A README that suddenly speaks in corporate template-voice in a
hacker-voice project reads worse than no README.

## Step 4 — Section menu

Default ordering. Skip any section that doesn't earn its place, and add
a brief HTML comment when you do (`<!-- Section omitted: no public API -->`)
so reviewers can see it was a choice.

1. **Title + one-sentence tagline.** Plain language, no marketing.
2. **Badges.** Only ones that are accurate and stay current — license,
   CI, version. Stale badges are worse than none.
3. **About.** 2–3 paragraphs: what, why, who for.
4. **Features.** One line each; link out for detail.
5. **Tech stack.** Table — only when the stack is non-obvious or
   multi-component.
6. **Getting started.** Prerequisites → install → configure → run. A
   stranger should reach a running state in under 10 minutes.
7. **Configuration.** Table of every env var the code reads. Columns:
   name, required, default, description.
8. **Usage.** Smallest working example with expected output. For
   apps, one screenshot or GIF.
9. **Architecture / structure.** Directory tree with a one-line
   description per top-level entry. Diagram only when interactions are
   non-obvious.
10. **API reference.** Tabular endpoint list with example request and
    response. Skip for non-API projects.
11. **Testing.** Commands to run tests; brief note on strategy
    (unit / integration / e2e) and external dependencies.
12. **Deployment.** Skip for libraries and tools.
13. **Roadmap.** Only if you can keep it current. Stale roadmap items
    mislead about project health — better to omit than to lie.
14. **Contributing.** Link to `CONTRIBUTING.md` rather than inlining
    process, unless the project is very small.
15. **Changelog.** Link to `CHANGELOG.md`. Don't inline release notes.
16. **License.** Always present. State it; link the LICENSE file.
17. **Acknowledgments / contact.** Optional.

The README is a front door, not a manual. Push API deep-dives,
architectural rationale, and runbooks into linked files.

## Step 5 — Quality check

Before delivery, every item below must be yes or explicitly N/A:

- A stranger could clone and run in under 10 minutes.
- Every env var the code reads is documented.
- At least one working usage example with expected output.
- Directory structure shown and briefly explained.
- No hardcoded secrets, keys, or production URLs anywhere.
- License stated, LICENSE file referenced.
- All badges link to live URLs (or are removed).
- Internal TOC links resolve — or the README is short enough (under
  ~200 lines) that no TOC is needed.
- Markdown renders on GitHub: fenced code blocks have language tags,
  tables align, no broken nesting.

## Audit mode — additional steps

If auditing an existing README:

1. Note what's *present and good* — don't recommend rewrites of working
   sections.
2. List missing essentials per the project-type table in Step 2.
3. Flag outdated items: dead links, commands that fail on current
   versions, env vars referenced in code but undocumented, env vars
   documented but no longer read.
4. Prioritize fixes: high (security / correctness) → medium (missing
   essential section) → low (polish).
5. For each gap, give a concrete fix, not just "this section is weak".

## Anti-patterns

| Pattern                                       | Why it's bad                                      | Fix                                          |
|-----------------------------------------------|---------------------------------------------------|----------------------------------------------|
| Hardcoded API keys or secrets                 | Indexed by search, leaks credentials              | `.env.example` + configuration table         |
| "Just install it" with no commands            | Not reproducible                                  | Exact copy-paste commands                    |
| Screenshot of code instead of a code block    | Not searchable, not copy-pasteable                | Fenced code block with language tag          |
| TOC on a 100-line README                      | Maintenance burden, no benefit                    | Skip — GitHub anchors headings anyway        |
| "See the code" with no architecture prose     | Hostile to newcomers                              | Brief architecture section + diagram         |
| Unchecked roadmap items from two years ago    | Signals dead project                              | Remove, version, or mark as paused           |
| License section missing                       | Legal ambiguity; nobody can safely use it         | State the license; link LICENSE              |
| Boilerplate with unfilled `{TODO}` markers    | Worse than no README                              | Fill every placeholder or remove the section |

## Guardrails

- Never invent install steps, env vars, or features you can't verify
  from the code or the user. Mark unknowns as `<!-- TODO -->` and list
  them at the end of your output.
- Never wipe out the existing README's voice for template-speak.
- Never include a section just because the template lists it. Earned
  presence beats reflexive completeness.
- Never recommend full rewrites in audit mode when targeted edits will
  do — the user has to maintain the result.
"#;

const NEBULA_SETUP: &str = r#"---
name: Nebula Setup
description: Use when the user is setting up nebula for the first time, wants recommended MCP servers, or asks what to install to get started. Walks through a curated stack with live-verified install steps.
built_in: true
---

The user wants to bootstrap nebula with a useful MCP stack. Your job is
not to dump a list — it's to help them pick what fits their work, then
get each one running with *current* install instructions.

## Step 0 — Verify live, never quote from memory

MCP server install commands, repo locations, and config flags change
often. The single biggest failure mode of this skill is confidently
pasting a stale `npx` command. Before telling the user to run anything:

- If a web-fetch or search tool is available (Jina, Serper, Context7,
  WebFetch, etc.), look up the current install instructions from the
  project's GitHub README or official docs in this session.
- If no such tool is available — common on a fresh nebula install
  where the whole point of this skill is to add the first ones — say so
  plainly and give the user the canonical repo URL (GitHub org/repo) to
  copy the command from themselves. Do not paste a remembered command.

This rule overrides any specific commands shown later in this skill;
those are illustrative of *shape*, not current truth.

## Step 0.5 — Fresh install? Bootstrap with Jina

If the user has zero MCP servers configured, you have no fetch or
search tool to verify *anything* live (chicken-and-egg). In that case,
start with Jina — it's a remote MCP server, signup is free, and the
free tier ships with enough tokens to get going. Once Jina is
connected, you can use it to look up install instructions for
everything else in this skill.

Tell the user:

1. Sign up at `https://jina.ai/` to get an API key (free tier).
2. In nebula, open **Settings → MCP servers → Add Server**. Fill in:
   - **Name:** `jina`
   - **Transport Type:** Streamable HTTP
   - **Server URL:** `https://mcp.jina.ai/v1`
   - **Headers:** `Authorization: Bearer {JINA_API_KEY}` (one line —
     the UI accepts `Header: Value` lines)
3. Replace `{JINA_API_KEY}` with the key from step 1.
4. Save Changes. Nebula spawns the server live — no app restart needed.
5. Confirm in the MCP servers list that `jina` is connected and its
   tools are populated.

If the URL no longer works (Jina has changed it before), tell the user
to check the current endpoint at `https://jina.ai/` rather than
guessing. Then loop back to Step 1 with the rest of the stack.

## Step 1 — Match servers to what the user actually does

Don't recommend everything. One short question first: what do they want
the assistant to help with? Map their answer:

- **Reading web pages, fetching docs** → Jina (web reader).
- **Searching the web** → Serper (Google API) or Jina (built-in search).
- **Working with local code** → Serena *or* DesktopCommander (pick one;
  see Step 2).
- **Looking up library / framework docs** → Context7.

If they say "everything", default starter set: Jina + DesktopCommander
+ Context7. Add Serper later if Jina's search isn't enough.

## Step 2 — The recommended stack

| Server            | Role                                       | Transport | Notes                                            |
|-------------------|--------------------------------------------|-----------|--------------------------------------------------|
| Jina              | Web fetch + search, LLM-friendly markdown  | StreamableHttp | Remote — no local install, just URL + API key |
| Serena            | Semantic code navigation (LSP-based)       | Stdio     | Powerful; heavier setup (Python deps, LSPs)      |
| DesktopCommander  | Filesystem + terminal + process control    | Stdio     | Lighter setup via npx; broader scope than Serena |
| Context7          | Live library / framework documentation     | Stdio     | Run via npx, no API key                          |
| Serper            | Google search API                          | Stdio     | Needs Serper API key (free tier exists)          |

**Serena vs. DesktopCommander — pick one, not both.**

- Choose **Serena** if the user works in a few well-defined codebases
  and wants semantic navigation (definitions, references, refactors).
- Choose **DesktopCommander** if the user wants broader filesystem and
  terminal access, or doesn't want to fight LSP setup.

Installing both is a mistake — their tool surfaces overlap and the
model will sometimes call the wrong one.

## Step 3 — Add to nebula

Nebula has a GUI for MCP servers. Use it — do not tell the user to
hand-edit `settings.json` unless they specifically ask to. For each
server the user picked:

1. Look up the current install command and config shape live (Step 0).
2. Have the user open **Settings → MCP servers → Add Server**. The
   dialog exposes:

   - **Name** — short identifier (used as the JSON key).
   - **Transport Type** — three buttons: Stdio (Local), SSE,
     Streamable HTTP.
   - **For Stdio:** Command (single string), Args (comma-separated),
     Env (one `KEY=value` per line).
   - **For SSE / Streamable HTTP:** Server URL, Headers (one
     `Header: Value` per line; the UI also accepts `Header=Value`).
   - **Allowlist / Denylist** — comma-separated tool names; leave
     empty to allow all.

3. Save Changes — nebula spawns the server live. No restart needed.
4. For servers needing an API key: tell the user *where to get it*
   (link the signup page you verified live), and where to paste it
   (the Headers field for remote servers; the Env field for Stdio
   servers that read it from the environment). Don't paste the key
   anywhere else.

Add one server at a time and verify each before moving to the next.
Batching makes attribution impossible when something fails.

### Reference: the JSON the UI writes

For diagnostics (see `mcp-diagnostics`) or for users who specifically
want to hand-edit, the underlying `settings.json` shape per transport:

```json
"{name}": {
  "type": "Stdio",
  "command": "{cmd}",
  "args": ["..."],
  "env": { "VAR": "value" }
}

"{name}": {
  "type": "Sse",
  "url": "{URL}",
  "headers": { "Authorization": "Bearer {KEY}" }
}

"{name}": {
  "type": "StreamableHttp",
  "url": "{URL}",
  "headers": { "Authorization": "Bearer {KEY}" }
}
```

Each entry also accepts optional `auto_approve: bool`,
`auto_approve_tools: [string]`, and
`permissions: { allowlist: [], denylist: [] }`. There is no per-server
`enabled` flag; to disable a server use the settings UI (which adds all
of its tool names to the top-level `disabled_tools` list — the server
stays connected but its tools are hidden from the LLM), or delete the
entry entirely.

## Step 4 — Verify

Nebula spawns the server immediately when Save Changes is clicked — no
restart needed.

1. In the MCP servers list, confirm the server shows as connected and
   its tools are populated.
2. If it doesn't connect, hand off to `mcp-diagnostics`. Don't try to
   debug it inline from this skill — that's a different workflow.

## Step 5 — Approval and enable/disable toggles

Approval and tool-visibility controls live in nebula's **Tools panel**,
grouped by server. Four toggles per server:

- **Server-level auto-approve** (shield icon next to the server name)
  — every tool from this server runs without an approval prompt.
- **Per-tool auto-approve** (shield icon on an individual tool, visible
  on hover) — same idea, scoped to one tool. Locked out when
  server-level auto-approve is on.
- **Server-level enable/disable** (checkbox icon next to the server
  name, "Enable All" / "Disable All") — toggles every tool from the
  server in one click. The server stays connected; its tools are
  hidden from the LLM (added to the top-level `disabled_tools` list).
- **Per-tool enable/disable** (per-tool checkbox) — same, scoped to
  one tool.

Recommendations after a server is verified:

- **Read-only servers** (Jina fetch, Context7, Serper search) — server-
  level auto-approve is reasonable; they don't mutate state.
- **Write / execute servers** (DesktopCommander shell, Serena edits)
  — keep per-call approval on, at least until the user has built trust
  with specific tools. If they want to skip prompts for one safe tool,
  use *per-tool* auto-approve, not server-level.

Never recommend disabling approval wholesale to "make it easier". That
removes nebula's primary safety boundary.

## Guardrails

- Never paste an install command, repo path, or URL you didn't verify
  in this session. If you have no way to verify, say so and link the
  canonical repo so the user copies it themselves.
- Never recommend a server you can't describe in one sentence. If a
  user asks about a server you don't know, look it up — don't bluff.
- Never install Serena *and* DesktopCommander. Pick one.
- Never put API keys directly in `settings.json` if the server supports
  env-var loading. Document the env var instead.
- Never install more than the user needs. A small connected stack
  beats a sprawling one where tools fight for the model's attention.
"#;
