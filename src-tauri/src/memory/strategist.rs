use crate::llm::provider::{LlmProvider, Message};
use crate::memory::{librarian::Librarian, MemoryHit, SearchOptions};
use crate::mcp::config::Settings;
use anyhow::Result;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Hard limits for strategist operation (safety bounds)
const MAX_RETRIEVAL_ROUNDS: usize = 2;
const MAX_QUERIES_PER_ROUND: usize = 3;
const MAX_TOTAL_HITS: usize = 20;
const DEFAULT_SNIPPET_CHARS: usize = 400;

/// A single search query requested by the planner
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub scope: Option<String>, // conversation_id
    #[serde(default)]
    pub roles: Option<Vec<String>>,
    #[serde(default)]
    pub max_age_days: Option<u64>,
}

/// Search plan returned by the planner
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SearchPlan {
    #[serde(default)]
    pub queries: Vec<SearchQuery>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl SearchPlan {
    /// Validate and clamp the plan to safety limits
    pub fn validate_and_clamp(&mut self) {
        // Limit number of queries
        self.queries.truncate(MAX_QUERIES_PER_ROUND);

        // Clamp limits on individual queries
        for query in &mut self.queries {
            if let Some(limit) = query.limit {
                query.limit = Some(limit.min(MAX_TOTAL_HITS));
            }
        }
    }
}

/// Result from the strategist orchestrator
#[derive(Debug, Clone)]
pub struct StrategistContextResult {
    pub context_text: String,
    pub selected_message_ids: Vec<String>,
    pub search_plan: Option<SearchPlan>,
}

/// Orchestrates memory retrieval with optional strategist planning
pub struct StrategistMemoryOrchestrator;

impl StrategistMemoryOrchestrator {
    /// Assemble context with optional strategist-driven multi-stage retrieval
    pub async fn assemble_context(
        query: &str,
        recent_history: &[Message],
        librarian: Arc<Mutex<Librarian>>,
        context_turns: usize,
        context_model_id: Option<&str>,
        settings: &Settings,
    ) -> Result<StrategistContextResult> {
        // Step 1: Initial retrieval (fast, deterministic)
        let initial_options = SearchOptions {
            limit: 10,
            // Include tool outputs - model may need to reference prior tool results
            ..Default::default()
        };

        let initial_results = {
            let lib = librarian.lock().await;
            lib.search_with_options(query, initial_options)?
        };
        let initial_hits: Vec<MemoryHit> = initial_results
            .into_iter()
            .map(|res| MemoryHit::from_search_result(res, DEFAULT_SNIPPET_CHARS))
            .collect();

        // If no context model, return baseline formatted context
        let Some(model_id) = context_model_id else {
            return Ok(Self::baseline_context(&initial_hits));
        };

        // Step 2: Run strategist planner loop
        Self::run_strategist_loop(
            query,
            recent_history,
            context_turns,
            initial_hits,
            librarian,
            model_id,
            settings,
        )
        .await
    }

    /// Fallback: baseline context without strategist
    fn baseline_context(hits: &[MemoryHit]) -> StrategistContextResult {
        if hits.is_empty() {
            return StrategistContextResult {
                context_text: String::new(),
                selected_message_ids: vec![],
                search_plan: None,
            };
        }

        let context_text = hits
            .iter()
            .map(|h| format!("[{}] {}", h.role, h.snippet))
            .collect::<Vec<_>>()
            .join("\n\n");

        let selected_ids = hits.iter().map(|h| h.message_id.clone()).collect();

        StrategistContextResult {
            context_text: format!("Relevant Memories:\n{}", context_text),
            selected_message_ids: selected_ids,
            search_plan: None,
        }
    }

    /// Run the strategist planner loop
    async fn run_strategist_loop(
        query: &str,
        recent_history: &[Message],
        context_turns: usize,
        initial_hits: Vec<MemoryHit>,
        librarian: Arc<Mutex<Librarian>>,
        model_id: &str,
        settings: &Settings,
    ) -> Result<StrategistContextResult> {
        // Parse model_id
        let parts: Vec<&str> = model_id.split("::").collect();
        if parts.len() != 2 {
            tracing::warn!("Invalid context_model format, falling back to baseline");
            return Ok(Self::baseline_context(&initial_hits));
        }

        let (provider_id, model_name) = (parts[0], parts[1]);

        // Instantiate provider
        let provider = Self::create_provider(provider_id, model_name, settings)?;

        // Step 2a: Planner call
        let search_plan = Self::call_planner(
            query,
            recent_history,
            context_turns,
            &initial_hits,
            provider.as_ref(),
        )
        .await?;

        // Step 2b: Execute follow-up searches (if any)
        let mut all_hits = initial_hits;
        let plan_for_result = if let Some(mut plan) = search_plan {
            plan.validate_and_clamp();
            let follow_up_hits =
                Self::execute_search_plan(&plan, librarian).await?;
            all_hits.extend(follow_up_hits);
            Some(plan)
        } else {
            None
        };

        // Deduplicate by message_id
        let mut seen_ids = HashSet::new();
        all_hits.retain(|hit| seen_ids.insert(hit.message_id.clone()));

        // Enforce total hit limit
        all_hits.truncate(MAX_TOTAL_HITS);

        // Step 2c: Synthesis call
        let context_text = Self::call_synthesizer(
            query,
            recent_history,
            context_turns,
            &all_hits,
            provider.as_ref(),
        )
        .await?;

        let selected_ids = all_hits.iter().map(|h| h.message_id.clone()).collect();

        Ok(StrategistContextResult {
            context_text,
            selected_message_ids: selected_ids,
            search_plan: plan_for_result,
        })
    }

    /// Create LLM provider instance
    fn create_provider(
        provider_id: &str,
        model_name: &str,
        settings: &Settings,
    ) -> Result<Box<dyn LlmProvider + Send + Sync>> {
        use crate::llm::{anthropic::AnthropicProvider, ollama::OllamaProvider, openai::OpenAiProvider};
        use crate::mcp::config::ProviderType;

        let config = settings
            .providers
            .get(provider_id)
            .ok_or_else(|| anyhow::anyhow!("Provider '{}' not found", provider_id))?;

        let provider: Box<dyn LlmProvider + Send + Sync> = match config.provider_type {
            ProviderType::OpenAI | ProviderType::OpenAICompatible => {
                let key = config.api_key.clone().unwrap_or_default();
                let base_url = config.base_url.clone();
                Box::new(OpenAiProvider::new(key, base_url, model_name.to_string()))
            }
            ProviderType::Anthropic => {
                let key = config.api_key.clone().unwrap_or_default();
                Box::new(AnthropicProvider::new(key, model_name.to_string()))
            }
            ProviderType::Ollama => {
                let base_url = config
                    .base_url
                    .clone()
                    .unwrap_or("http://localhost:11434".to_string());
                Box::new(OllamaProvider::new(base_url, model_name.to_string()))
            }
        };

        Ok(provider)
    }

    /// Call the planner to get search plan
    async fn call_planner(
        query: &str,
        recent_history: &[Message],
        context_turns: usize,
        initial_hits: &[MemoryHit],
        provider: &dyn LlmProvider,
    ) -> Result<Option<SearchPlan>> {
        // Format recent conversation context
        let recent_context = Self::format_recent_context(recent_history, context_turns);

        // Format initial hits
        let hits_preview = initial_hits
            .iter()
            .enumerate()
            .map(|(i, hit)| {
                format!(
                    "{}. [{}] {} (score: {:.2}, created: {})\n   {}",
                    i + 1,
                    hit.role,
                    hit.conversation_id,
                    hit.score,
                    hit.created_at,
                    hit.snippet.chars().take(200).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = format!(
            r#"You are a Memory Search Planner. Analyze the user query and initial search results to decide if additional targeted searches would improve context quality.

USER QUERY: {}{}

INITIAL SEARCH RESULTS ({} hits):
{}

TASK:
Decide if follow-up searches would help. If yes, output a JSON search plan. If the initial results are sufficient, output {{"queries": []}}.

SEARCH PLAN SCHEMA:
{{
  "queries": [
    {{
      "q": "search query string",
      "limit": 5,  // optional, max results
      "scope": "conversation_id",  // optional, null = global
      "roles": ["assistant", "user"],  // optional, filter by roles
      "max_age_days": 7  // optional, recent only
    }}
  ],
  "notes": "reasoning for plan"  // optional
}}

GUIDELINES:
- If initial results are good, return empty queries: {{"queries": []}}
- Max 3 follow-up queries
- Use scope for conversation-local searches
- Use roles to target specific message types
- Use max_age_days for recent context
- Keep queries focused and specific

OUTPUT (JSON only, no prose):"#,
            query,
            if recent_context.is_empty() {
                String::new()
            } else {
                format!("\n\n{}", recent_context)
            },
            initial_hits.len(),
            if hits_preview.is_empty() {
                "(none)".to_string()
            } else {
                hits_preview
            }
        );

        let messages = vec![Message {
            id: None,
            role: "user".to_string(),
            content: Some(prompt),
            tool_calls: None,
            attachments: None,
            tool_call_id: None,
        }];

        match provider.chat(messages, vec![], None).await {
            Ok(response) => {
                let content = response.content.unwrap_or_default();
                // Try to parse JSON from response
                match Self::extract_and_parse_json::<SearchPlan>(&content) {
                    Ok(plan) if plan.queries.is_empty() => Ok(None),
                    Ok(plan) => Ok(Some(plan)),
                    Err(e) => {
                        tracing::warn!("Failed to parse search plan JSON: {}", e);
                        Ok(None) // Fallback: no follow-up searches
                    }
                }
            }
            Err(e) => {
                tracing::error!("Planner call failed: {}", e);
                Ok(None) // Fallback
            }
        }
    }

    /// Extract JSON from response and parse it
    fn extract_and_parse_json<T: serde::de::DeserializeOwned>(content: &str) -> Result<T> {
        // Try to find JSON in the response (handles cases where model adds prose)
        let json_str = if let Some(start) = content.find('{') {
            if let Some(end) = content.rfind('}') {
                &content[start..=end]
            } else {
                content
            }
        } else {
            content
        };

        serde_json::from_str(json_str)
            .map_err(|e| anyhow::anyhow!("JSON parse error: {}", e))
    }

    /// Execute the search plan
    async fn execute_search_plan(
        plan: &SearchPlan,
        librarian: Arc<Mutex<Librarian>>,
    ) -> Result<Vec<MemoryHit>> {
        let mut all_hits = Vec::new();

        for query_spec in &plan.queries {
            let options = SearchOptions {
                limit: query_spec.limit.unwrap_or(5),
                conversation_id: query_spec.scope.clone(),
                include_roles: query_spec.roles.clone(),
                exclude_roles: None,
                max_age_days: query_spec.max_age_days,
            };

            let lib = librarian.lock().await;
            match lib.search_with_options(&query_spec.q, options) {
                Ok(results) => {
                    let hits: Vec<MemoryHit> = results
                        .into_iter()
                        .map(|res| MemoryHit::from_search_result(res, DEFAULT_SNIPPET_CHARS))
                        .collect();
                    all_hits.extend(hits);
                }
                Err(e) => {
                    tracing::warn!("Follow-up search failed: {}", e);
                    // Continue with other queries
                }
            }
        }

        Ok(all_hits)
    }

    /// Call synthesizer to produce final context
    async fn call_synthesizer(
        query: &str,
        recent_history: &[Message],
        context_turns: usize,
        all_hits: &[MemoryHit],
        provider: &dyn LlmProvider,
    ) -> Result<String> {
        let recent_context = Self::format_recent_context(recent_history, context_turns);

        let hits_block = all_hits
            .iter()
            .map(|hit| {
                format!(
                    "[{}] {} (score: {:.2})\n{}",
                    hit.role, hit.created_at, hit.score, hit.snippet
                )
            })
            .collect::<Vec<_>>()
            .join("\n---\n");

        let prompt = format!(
            r#"You are a Memory Context Synthesizer. Create a concise, relevant context block for the main LLM.

USER QUERY: {}{}

RETRIEVED MEMORIES ({} hits):
---
{}
---

TASK:
Synthesize the relevant facts into a coherent context block. Focus on information that directly helps answer the query.

GUIDELINES:
- Be concise but complete
- Remove duplicates and noise
- Organize related facts together
- If nothing is relevant, say "No relevant context found."
- NO conversational filler, just the facts

OUTPUT:"#,
            query,
            if recent_context.is_empty() {
                String::new()
            } else {
                format!("\n\n{}", recent_context)
            },
            all_hits.len(),
            if hits_block.is_empty() {
                "(none)".to_string()
            } else {
                hits_block
            }
        );

        let messages = vec![Message {
            id: None,
            role: "user".to_string(),
            content: Some(prompt),
            tool_calls: None,
            attachments: None,
            tool_call_id: None,
        }];

        match provider.chat(messages, vec![], None).await {
            Ok(response) => {
                let content = response.content.unwrap_or_default();
                if content.contains("No relevant context") {
                    Ok(String::new())
                } else {
                    Ok(content)
                }
            }
            Err(e) => {
                tracing::error!("Synthesizer call failed: {}", e);
                // Fallback: return raw snippets
                Ok(all_hits
                    .iter()
                    .map(|h| h.snippet.clone())
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
        }
    }

    /// Format recent conversation context
    fn format_recent_context(recent_history: &[Message], context_turns: usize) -> String {
        if context_turns == 0 {
            return String::new();
        }

        let max_msgs = context_turns.saturating_mul(2);
        let mut recent: Vec<String> = Vec::new();

        for m in recent_history.iter().rev() {
            if recent.len() >= max_msgs {
                break;
            }
            // Include user, assistant, and tool messages (tool outputs may be critical context)
            if m.role != "user" && m.role != "assistant" && m.role != "tool" {
                continue;
            }
            let content = m.content.clone().unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }
            recent.push(format!("{}: {}", m.role, content.trim()));
        }
        recent.reverse();

        if recent.is_empty() {
            String::new()
        } else {
            format!(
                "RECENT CONVERSATION (last {} turns max):\n---\n{}\n---",
                context_turns,
                recent.join("\n")
            )
        }
    }
}
