
export type ProviderType = "OpenAI" | "Anthropic" | "Ollama" | "OpenAICompatible";

export function getProviderIcon(
    type: ProviderType | string | undefined,
    providerId?: string,
    customIcon?: string,
): string {
    // A user-chosen emoji/glyph always wins over the heuristics below.
    const trimmedCustom = customIcon?.trim();
    if (trimmedCustom) return trimmedCustom;

    const normalizedId = providerId?.toLowerCase() || "";
    if (normalizedId.includes("openrouter")) return "⚡";
    if (normalizedId.includes("lmstudio") || normalizedId.includes("lm-studio")) return "🖥️";
    if (normalizedId.includes("groq")) return "🚀";

    if (normalizedId.includes("openai") && !normalizedId.includes("compatible")) return "🤖";
    if (normalizedId.includes("anthropic")) return "🧠";
    if (normalizedId.includes("ollama")) return "🦙";
    if (normalizedId.includes("local") || normalizedId.includes("custom")) return "🔌";

    if (!type) return "❓";

    // Safety check if type is not a string (e.g. enum number serialization edge case)
    if (typeof type !== 'string') {
        console.warn("getProviderIcon received non-string type:", type);
        return "❓";
    }

    // Normalize string input if it comes from raw settings
    const normalized = type.toLowerCase();

    if (normalized.includes("openai") && !normalized.includes("compatible")) return "🤖";
    if (normalized.includes("anthropic")) return "🧠";
    if (normalized.includes("ollama")) return "🦙";
    if (normalized.includes("openaicompatible") || normalized.includes("custom")) return "🔌";

    return "❓";
}
