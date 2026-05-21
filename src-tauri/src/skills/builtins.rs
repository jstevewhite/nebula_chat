//! Starter skills shipped with the app. On first run these are materialised
//! to `<skills_dir>/built-ins/<slug>.md`; subsequent runs leave them alone, so
//! user edits stick. To "reset" a built-in, delete the file and restart.
//!
//! Each entry's `built_in: true` frontmatter lets the UI render a badge and
//! lets the system distinguish overrides if we ever want a per-slug override
//! flow (delete a top-level user skill with the same slug to revert to the
//! built-in).

/// (slug, file body). Bodies are the full markdown file including YAML
/// frontmatter — kept human-readable in one place so it's obvious what each
/// skill does.
pub const ALL: &[(&str, &str)] = &[
    ("code-review", CODE_REVIEW),
    ("summarize-conversation", SUMMARIZE),
    ("debug-help", DEBUG_HELP),
];

const CODE_REVIEW: &str = r#"---
name: Code Review
description: Use when the user asks for a code review, wants feedback on a diff, or asks "what's wrong with this?". Focuses on correctness, security, and idiomatic style.
built_in: true
---

You are reviewing code for the user. Approach this as a senior reviewer who
respects the author's time:

1. **Read first, then react.** Skim the whole change before commenting on any
   single line.
2. **Prioritise.** Lead with correctness bugs, then security issues, then
   anything that will hurt future maintainers. Style nits go last and labelled
   as such.
3. **Be specific.** For each issue: cite the line, explain why it matters, and
   propose a concrete fix or alternative.
4. **No padding.** Skip praise of unchanged code, skip restating what the diff
   does, skip preambles like "I'll now review your code". Get to the findings.
5. **When you don't see a problem, say so plainly.** "Looks good — nothing
   to flag" beats inventing concerns to look thorough.
6. **Match the reviewer's grain to the change.** A one-line bug fix gets a
   one-line review; a 500-line refactor gets a structured one.

If the user hasn't pasted the code yet, ask for it.
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
