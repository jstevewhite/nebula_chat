import { invoke } from "@tauri-apps/api/core";

export interface CommandContext {
  conversationId: string | null;
  setInput: (value: string) => void;
  setMessages: React.Dispatch<React.SetStateAction<any[]>>;
  addSystemNote: (content: string) => void;

  // Optional extras for commands that need to reach beyond the chat thread.
  // All are filled in by ChatInterface when constructing the context; commands
  // that depend on them should null-check and surface a useful error.

  /** Creates a new conversation (delegating to App.tsx) and switches to it. */
  onCreateConversation?: (title: string) => Promise<string | null>;

  /** Model dropdown state — for `/model`. ID format: `${providerId}::${modelId}`. */
  availableModels?: Array<{ id: string; name: string; providerId: string; providerName?: string }>;
  selectedModel?: string;
  setSelectedModel?: (id: string) => void;
}

export interface ChatCommand {
  name: string;
  description: string;
  usage?: string;
  category: "Memory" | "Search" | "Skills" | "Meta";
  handler: (args: string, ctx: CommandContext) => Promise<void> | void;
}

function createSystemNote(content: string) {
  return {
    role: "system" as const,
    content,
    created_at: Math.floor(Date.now() / 1000),
  };
}

// Shared helper that commands can use (or the caller can use directly)
export function addSystemNoteToMessages(
  setMessages: React.Dispatch<React.SetStateAction<any[]>>,
  content: string
) {
  const note = createSystemNote(content);
  setMessages((prev) => [...prev, note]);
}

const commands: ChatCommand[] = [
  {
    name: "help",
    description: "Show all available slash commands",
    usage: "/help",
    category: "Meta",
    handler: (_args, ctx) => {
      const grouped = commands.reduce<Record<string, ChatCommand[]>>((acc, cmd) => {
        if (!acc[cmd.category]) acc[cmd.category] = [];
        acc[cmd.category].push(cmd);
        return acc;
      }, {});

      let output = "**Available Slash Commands**\n\n";

      const categoryOrder: Array<"Meta" | "Memory" | "Search" | "Skills"> = [
        "Meta",
        "Memory",
        "Search",
        "Skills",
      ];

      for (const cat of categoryOrder) {
        const cmds = grouped[cat];
        if (!cmds || cmds.length === 0) continue;

        output += `**${cat}**\n`;
        for (const cmd of cmds) {
          // `usage` already includes the leading slash + name (e.g. "/remember <text>").
          // Fall back to "/name" for commands that didn't set a usage string.
          const display = cmd.usage || `/${cmd.name}`;
          output += `- \`${display}\` — ${cmd.description}\n`;
        }
        output += "\n";
      }

      output += "_Tip: Type `/` in the composer or click the / button to see suggestions._";

      ctx.addSystemNote(output);
      ctx.setInput("");
    },
  },

  {
    name: "remember",
    description: "Extract and save facts from the provided text (local only)",
    usage: "/remember <text>",
    category: "Memory",
    handler: async (args, ctx) => {
      const text = args.trim();
      if (!text) {
        ctx.addSystemNote("`/remember` requires text. Example: `/remember I prefer dark mode and use Neovim.`");
        ctx.setInput("");
        return;
      }

      ctx.setInput("");

      try {
        const result = await invoke<{ extracted: number; message: string }>(
          "extract_facts_from_text",
          { text }
        );

        const note = `Remembered ${result.extracted} fact${result.extracted === 1 ? "" : "s"} from /remember: ${text}`;
        ctx.addSystemNote(note);
      } catch (e) {
        ctx.addSystemNote(`/remember failed: ${String(e)}`);
      }
    },
  },

  {
    name: "search",
    description: "Full-text search across all your conversations",
    usage: "/search <query>",
    category: "Search",
    handler: async (args, ctx) => {
      const query = args.trim();
      if (!query) {
        ctx.addSystemNote("`/search` requires a query. Example: `/search project roadmap`");
        ctx.setInput("");
        return;
      }

      ctx.setInput("");

      try {
        const results = await invoke<any[]>("search_messages", { query });

        if (!results || results.length === 0) {
          ctx.addSystemNote(`No results found for: "${query}"`);
          return;
        }

        const top = results.slice(0, 5);
        let output = `**Search results for "${query}"** (${results.length} total)\n\n`;

        for (const r of top) {
          const preview = (r.content || "").slice(0, 140).replace(/\n/g, " ");
          const date = r.created_at
            ? new Date(typeof r.created_at === "number" ? r.created_at * 1000 : r.created_at).toLocaleDateString()
            : "";
          output += `- ${date ? `(${date}) ` : ""}${preview}${preview.length === 140 ? "..." : ""}\n`;
        }

        if (results.length > 5) {
          output += `\n_...and ${results.length - 5} more. Use the left sidebar search for full results._`;
        }

        ctx.addSystemNote(output);
      } catch (e) {
        ctx.addSystemNote(`/search failed: ${String(e)}`);
      }
    },
  },

  {
    name: "skills",
    description: "List all skills (or view the body of one without loading it)",
    usage: "/skills [slug]",
    category: "Skills",
    handler: async (args, ctx) => {
      const slug = args.trim().toLowerCase();
      ctx.setInput("");

      try {
        if (slug) {
          const skill = await invoke<any>("get_skill", { slug });
          if (skill) {
            ctx.addSystemNote(
              `**${skill.name}** (\`${skill.slug}\`)\n\n${skill.description}\n\n\`\`\`\n${skill.body}\n\`\`\``
            );
          } else {
            ctx.addSystemNote(`No skill found with slug "${slug}". Use \`/skills\` to list all.`);
          }
        } else {
          const list = await invoke<any[]>("list_skills");
          if (!list || list.length === 0) {
            ctx.addSystemNote("No skills installed yet. Add them in Settings → Skills.");
            return;
          }
          let output = "**Available Skills**\n\n";
          for (const s of list) {
            output += `- \`${s.slug}\` — ${s.description}${s.built_in ? " _(built-in)_" : ""}\n`;
          }
          output += "\n_Use `/skill <slug>` to load a skill into this conversation._";
          ctx.addSystemNote(output);
        }
      } catch (e) {
        ctx.addSystemNote(`/skills failed: ${String(e)}`);
      }
    },
  },

  {
    name: "skill",
    description: "Load/invoke a skill directly into the current conversation context",
    usage: "/skill <slug>",
    category: "Skills",
    handler: async (args, ctx) => {
      const slug = args.trim().toLowerCase();
      ctx.setInput("");

      // No slug provided — show the list of available skills (helpful when user types "/skill ")
      if (!slug) {
        try {
          const list = await invoke<any[]>("list_skills");

          if (!list || list.length === 0) {
            ctx.addSystemNote("No skills installed yet. Add them in Settings → Skills.");
            return;
          }

          let output = "**Available Skills** (type `/skill <slug>` to load one)\n\n";
          for (const s of list) {
            output += `- \`${s.slug}\` — ${s.description}${s.built_in ? " _(built-in)_" : ""}\n`;
          }
          output += "\nExample: `/skill research-assistant`";

          ctx.addSystemNote(output);
        } catch (e) {
          ctx.addSystemNote(`/skill failed: ${String(e)}`);
        }
        return;
      }

      try {
        const skill = await invoke<any>("get_skill", { slug });

        if (!skill) {
          ctx.addSystemNote(
            `No skill found with slug "${slug}".\n\n` +
            "Use `/skills` to list all available skills."
          );
          return;
        }

        // Insert the skill as a system note so it becomes part of conversation context
        const note = [
          `**Skill loaded: ${skill.name}** (\`${skill.slug}\`)`,
          "",
          skill.description ? `${skill.description}\n` : "",
          "```markdown",
          skill.body.trim(),
          "```",
          "",
          "_This skill is now in context for the current conversation._"
        ].join("\n");

        ctx.addSystemNote(note);
      } catch (e) {
        ctx.addSystemNote(`/skill failed: ${String(e)}`);
      }
    },
  },

  {
    name: "clear",
    description: "Clear the current input",
    usage: "/clear",
    category: "Meta",
    handler: (_args, ctx) => {
      ctx.setInput("");
      ctx.addSystemNote("Composer cleared.");
    },
  },

  {
    name: "new",
    description: "Create a new conversation (optionally with a title)",
    usage: "/new [title]",
    category: "Meta",
    handler: async (args, ctx) => {
      ctx.setInput("");
      if (!ctx.onCreateConversation) {
        ctx.addSystemNote("/new is unavailable in this context.");
        return;
      }
      const title = args.trim() || "New Chat";
      try {
        const id = await ctx.onCreateConversation(title);
        if (id) {
          // The conversation has already switched; this note will appear in
          // the new (empty) conversation.
          ctx.addSystemNote(`Started new conversation: **${title}**`);
        } else {
          ctx.addSystemNote("/new failed: no conversation id returned.");
        }
      } catch (e) {
        ctx.addSystemNote(`/new failed: ${String(e)}`);
      }
    },
  },

  {
    name: "model",
    description: "List available models, or switch by name / id substring",
    usage: "/model [name]",
    category: "Meta",
    handler: (args, ctx) => {
      ctx.setInput("");
      const models = ctx.availableModels || [];
      if (models.length === 0) {
        ctx.addSystemNote("No models available. Configure providers in Settings → Providers.");
        return;
      }

      const query = args.trim().toLowerCase();

      // No arg → list models grouped by provider, mark active.
      if (!query) {
        const byProvider = models.reduce<Record<string, typeof models>>((acc, m) => {
          const key = m.providerName || m.providerId;
          (acc[key] ||= []).push(m);
          return acc;
        }, {});
        let output = "**Available models**\n\n";
        for (const [provider, list] of Object.entries(byProvider)) {
          output += `**${provider}**\n`;
          for (const m of list) {
            const fullId = `${m.providerId}::${m.id}`;
            const active = fullId === ctx.selectedModel ? " ← active" : "";
            output += `- \`${m.id}\` — ${m.name}${active}\n`;
          }
          output += "\n";
        }
        output += "_Switch with `/model <name>` (substring match against id or name)._";
        ctx.addSystemNote(output);
        return;
      }

      // Arg → fuzzy match by id or name (case-insensitive substring).
      const matches = models.filter((m) =>
        m.id.toLowerCase().includes(query) || m.name.toLowerCase().includes(query)
      );
      if (matches.length === 0) {
        ctx.addSystemNote(`No model matched "${args.trim()}". Use \`/model\` to list available models.`);
        return;
      }
      if (matches.length > 1) {
        // Prefer an exact id match when ambiguous.
        const exact = matches.find((m) => m.id.toLowerCase() === query);
        if (!exact) {
          let output = `Multiple models match "${args.trim()}":\n\n`;
          for (const m of matches) {
            output += `- \`${m.id}\` — ${m.name} (${m.providerName || m.providerId})\n`;
          }
          output += "\nUse a more specific query.";
          ctx.addSystemNote(output);
          return;
        }
        matches.splice(0, matches.length, exact);
      }
      const target = matches[0];
      const fullId = `${target.providerId}::${target.id}`;
      ctx.setSelectedModel?.(fullId);
      ctx.addSystemNote(`Switched model → **${target.name}** (\`${target.id}\` via ${target.providerName || target.providerId})`);
    },
  },

  {
    name: "recall",
    description: "Hybrid semantic + BM25 search across your memory docs",
    usage: "/recall <query>",
    category: "Memory",
    handler: async (args, ctx) => {
      const query = args.trim();
      if (!query) {
        ctx.addSystemNote("`/recall` requires a query. Example: `/recall how does MCP approval work`");
        ctx.setInput("");
        return;
      }
      ctx.setInput("");

      try {
        const out = await invoke<{ hits: any[] }>("recall_memory_docs", { query, k: 5 });
        const hits = out.hits || [];
        if (hits.length === 0) {
          ctx.addSystemNote(`No memory-doc hits for: "${query}"`);
          return;
        }
        let output = `**Recall results for "${query}"** (${hits.length})\n\n`;
        for (const h of hits) {
          const preview = (h.text || "").slice(0, 200).replace(/\n+/g, " ");
          const trunc = (h.text || "").length > 200 ? "…" : "";
          const score = typeof h.score === "number" ? h.score.toFixed(3) : "?";
          output += `- **\`${h.doc_id}\`** (chunk ${h.ord}, score ${score})\n  ${preview}${trunc}\n\n`;
        }
        output += "_Open the Memory panel → Docs tab to read full bodies._";
        ctx.addSystemNote(output);
      } catch (e) {
        ctx.addSystemNote(`/recall failed: ${String(e)}`);
      }
    },
  },

  {
    name: "facts",
    description: "List recent facts or facts about an entity",
    usage: "/facts [entity]",
    category: "Memory",
    handler: async (args, ctx) => {
      const entity = args.trim();
      ctx.setInput("");

      try {
        if (entity) {
          const facts = await invoke<any[]>("list_facts_for_entity", { entity, limit: 15 });
          if (!facts || facts.length === 0) {
            ctx.addSystemNote(`No facts found for entity "${entity}".`);
            return;
          }
          let output = `**Facts about "${entity}"**\n\n`;
          for (const f of facts) {
            output += `- ${f.subject} → ${f.predicate} → ${f.object} (${Math.round(f.confidence * 100)}%)\n`;
          }
          ctx.addSystemNote(output);
        } else {
          const facts = await invoke<any[]>("list_user_facts", {});
          if (!facts || facts.length === 0) {
            ctx.addSystemNote("No facts stored yet. Use `/remember` or the LLM memory tools.");
            return;
          }
          let output = "**Recent Facts**\n\n";
          for (const f of facts.slice(0, 12)) {
            output += `- ${f.subject} → ${f.predicate} → ${f.object}\n`;
          }
          ctx.addSystemNote(output);
        }
      } catch (e) {
        ctx.addSystemNote(`/facts failed: ${String(e)}`);
      }
    },
  },
];

export function getAllCommands(): ChatCommand[] {
  return [...commands];
}

export function getCommand(name: string): ChatCommand | undefined {
  const lower = name.toLowerCase();
  return commands.find((c) => c.name.toLowerCase() === lower);
}

export function parseSlashCommand(input: string): { name: string; args: string } | null {
  const trimmed = input.trim();
  if (!trimmed.startsWith("/")) return null;

  const withoutSlash = trimmed.slice(1).trim();
  if (!withoutSlash) return null;

  const firstSpace = withoutSlash.indexOf(" ");
  if (firstSpace === -1) {
    return { name: withoutSlash, args: "" };
  }

  const name = withoutSlash.slice(0, firstSpace);
  const args = withoutSlash.slice(firstSpace + 1).trim();
  return { name, args };
}

export function isSlashCommand(input: string): boolean {
  // Treat a bare "/" (or "/ " with spaces) as a valid trigger to show the full palette
  return input.trim().startsWith("/");
}

export async function executeSlashCommand(
  input: string,
  ctx: CommandContext
): Promise<boolean> {
  const parsed = parseSlashCommand(input);

  // Bare "/" with nothing after it — just clear the input, don't send to LLM
  if (!parsed || !parsed.name) {
    ctx.setInput("");
    return true;
  }

  const cmd = getCommand(parsed.name);
  if (!cmd) {
    ctx.addSystemNote(`Unknown command: \`/${parsed.name}\`. Type \`/help\` to see available commands.`);
    ctx.setInput("");
    return true;
  }

  try {
    await cmd.handler(parsed.args, ctx);
  } catch (e) {
    ctx.addSystemNote(`Command \`/${cmd.name}\` failed: ${String(e)}`);
    ctx.setInput("");
  }

  return true;
}