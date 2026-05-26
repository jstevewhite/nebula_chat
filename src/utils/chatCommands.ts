import { invoke } from "@tauri-apps/api/core";

export interface CommandContext {
  conversationId: string | null;
  setInput: (value: string) => void;
  setMessages: React.Dispatch<React.SetStateAction<any[]>>;
  addSystemNote: (content: string) => void;
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
          const usage = cmd.usage ? ` \`${cmd.usage}\`` : "";
          output += `- \`/${cmd.name}\`${usage} — ${cmd.description}\n`;
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