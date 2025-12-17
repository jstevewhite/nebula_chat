SUPERSEDED BY: memory2-revised.md

# Memory System Phase 2: Knowledge Graph Integration

## Overview

Extend Nebula's memory system with a **knowledge graph layer** for structured fact storage and retrieval. This complements the existing full-text search (Tantivy) with explicit entity-relationship modeling, enabling:

1. **Factual lookups**: Fast retrieval of structured facts (user preferences, project attributes, decisions)
2. **Personalization**: "Since you're a systems engineer..." style context injection
3. **Relationship traversal**: Graph queries to find connected facts
4. **Deduplication**: Canonical fact storage vs. scattered conversational mentions
5. **Provenance tracking**: Link facts back to source messages

## Architecture

### Hybrid Memory System

```
┌─────────────────────────────────────────────────────┐
│                   User Query                        │
└──────────────────┬──────────────────────────────────┘
                   │
                   v
         ┌─────────────────────┐
         │  Strategist         │
         │  Orchestrator       │
         └─────────┬───────────┘
                   │
        ┌──────────┴──────────┐
        │                     │
        v                     v
┌───────────────┐     ┌──────────────────┐
│  Full-text    │     │  Knowledge Graph │
│  Search       │     │  (Facts)         │
│  (Tantivy)    │     │  (SQLite)        │
└───────┬───────┘     └────────┬─────────┘
        │                      │
        │    ┌─────────────────┘
        │    │
        v    v
    ┌───────────────┐
    │  Synthesizer  │
    │  (combines    │
    │   both)       │
    └───────────────┘
```

### Fact Model

**Triplet structure**: `(subject, predicate, object)`

**Examples:**
- `(user, role, systems_engineer)`
- `(user, prefers, vim_keybindings)`
- `(nebula_chat, uses, tauri)`
- `(nebula_chat, has_feature, mcp_integration)`
- `(user, decided, "use strategist for memory")`

### Fact Categories

1. **User Profile**: Role, expertise, preferences, background
2. **Project Attributes**: Tech stack, features, architecture decisions
3. **Relationships**: User-project, project-technology, technology-technology
4. **Decisions**: Architectural choices, trade-offs, rationale

## Implementation Phases

---

## Phase 4a: Schema & Storage Layer

**Goal**: Add facts table to SQLite, basic CRUD operations

### 4a.1: Database Schema

Create facts table in `sqlite_manager.rs`:

```sql
CREATE TABLE IF NOT EXISTS facts (
    id TEXT PRIMARY KEY,
    subject TEXT NOT NULL,
    predicate TEXT NOT NULL,
    object TEXT NOT NULL,
    confidence REAL DEFAULT 1.0,
    source_message_id TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE SET NULL
);

CREATE INDEX idx_facts_subject ON facts(subject);
CREATE INDEX idx_facts_predicate ON facts(predicate);
CREATE INDEX idx_facts_object ON facts(object);
CREATE INDEX idx_facts_subject_predicate ON facts(subject, predicate);
CREATE INDEX idx_facts_source ON facts(source_message_id);
```

### 4a.2: Rust Types

Add to `src-tauri/src/memory/mod.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    pub source_message_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelevantFact {
    #[serde(flatten)]
    pub fact: Fact,
    pub relevance_score: f32,
    pub retrieval_reason: String, // "user_profile", "direct_match", "project_context", "relationship"
}
```

### 4a.3: SqliteManager Methods

Add to `src-tauri/src/memory/sqlite_manager.rs`:

```rust
impl SqliteManager {
    pub fn save_fact(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
        confidence: f32,
        source_message_id: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        self.conn.execute(
            "INSERT INTO facts (id, subject, predicate, object, confidence, source_message_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![id, subject, predicate, object, confidence, source_message_id, now, now],
        )?;

        Ok(id)
    }

    pub fn update_fact(&self, id: &str, confidence: f32) -> Result<()> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE facts SET confidence = ?, updated_at = ? WHERE id = ?",
            params![confidence, now, id],
        )?;
        Ok(())
    }

    pub fn get_fact(&self, id: &str) -> Result<Option<Fact>> {
        // Implementation
    }

    pub fn delete_fact(&self, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM facts WHERE id = ?", params![id])?;
        Ok(())
    }

    pub fn delete_facts_by_source(&self, message_id: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM facts WHERE source_message_id = ?",
            params![message_id],
        )?;
        Ok(())
    }
}
```

### 4a.4: Migration

Add migration method to `SqliteManager`:

```rust
pub fn migrate_facts_table(&self) -> Result<()> {
    self.conn.execute(
        "CREATE TABLE IF NOT EXISTS facts (
            id TEXT PRIMARY KEY,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            confidence REAL DEFAULT 1.0,
            source_message_id TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            FOREIGN KEY (source_message_id) REFERENCES messages(id) ON DELETE SET NULL
        )",
        [],
    )?;

    self.conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_facts_subject ON facts(subject)",
        [],
    )?;
    self.conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_facts_predicate ON facts(predicate)",
        [],
    )?;
    self.conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_facts_object ON facts(object)",
        [],
    )?;
    self.conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_facts_subject_predicate ON facts(subject, predicate)",
        [],
    )?;

    Ok(())
}
```

Call in `Librarian::new()`:

```rust
let _ = sqlite.migrate_facts_table();
```

**Commit**: `feat(memory): add facts table schema and CRUD operations`

---

## Phase 4b: Librarian Query Methods

**Goal**: High-level fact query API with filtering, traversal, and relevance scoring

### 4b.1: Pattern-based Queries

Add to `src-tauri/src/memory/librarian.rs`:

```rust
impl Librarian {
    /// Query facts by pattern: (subject, predicate, object)
    /// Use "*" or None for wildcards
    pub fn query_facts(&self, patterns: &[(&str, &str, Option<&str>)]) -> Result<Vec<Fact>> {
        let mut all_facts = Vec::new();

        for (subject, predicate, object) in patterns {
            let query = if let Some(obj) = object {
                format!(
                    "SELECT * FROM facts WHERE subject = ? AND predicate = ? AND object = ?
                     ORDER BY confidence DESC, updated_at DESC"
                )
            } else if *predicate == "*" {
                format!(
                    "SELECT * FROM facts WHERE subject = ?
                     ORDER BY confidence DESC, updated_at DESC"
                )
            } else {
                format!(
                    "SELECT * FROM facts WHERE subject = ? AND predicate = ?
                     ORDER BY confidence DESC, updated_at DESC"
                )
            };

            // Execute query and parse results...
            all_facts.extend(results);
        }

        Ok(all_facts)
    }

    /// Get all facts about an entity (subject OR object)
    pub fn get_facts_about(&self, entity: &str) -> Result<Vec<Fact>> {
        self.sqlite.query_facts_by_entity(entity)
    }

    /// Get user profile facts (for personalization)
    pub fn get_user_profile_facts(&self) -> Result<Vec<Fact>> {
        self.query_facts(&[
            ("user", "role", None),
            ("user", "expertise_in", None),
            ("user", "prefers", None),
            ("user", "background", None),
            ("user", "works_on", None),
        ])
    }

    /// Get project context facts for a conversation
    pub fn get_conversation_project_facts(&self, conversation_id: &str) -> Result<Vec<Fact>> {
        // Try to infer project from conversation metadata or messages
        // For now, simple heuristic: check if conversation title mentions a project
        let convos = self.list_conversations()?;
        let project_name = convos.iter()
            .find(|(id, _, _)| id == conversation_id)
            .and_then(|(_, title, _)| {
                // Extract project name from title (simple version)
                // Could be more sophisticated
                Some(title.to_lowercase())
            });

        if let Some(project) = project_name {
            self.query_facts(&[(project.as_str(), "*", None)])
        } else {
            Ok(vec![])
        }
    }
}
```

### 4b.2: Graph Traversal

```rust
impl Librarian {
    /// Traverse facts graph from entity
    /// max_hops: maximum depth to traverse (1-3 recommended)
    pub fn traverse_facts(&self, entity: &str, max_hops: usize) -> Result<Vec<Fact>> {
        let mut visited = HashSet::new();
        let mut facts = Vec::new();
        let mut queue = VecDeque::new();

        queue.push_back((entity.to_string(), 0));
        visited.insert(entity.to_string());

        while let Some((current_entity, depth)) = queue.pop_front() {
            if depth >= max_hops {
                continue;
            }

            // Get facts where current_entity is subject
            let entity_facts = self.get_facts_about(&current_entity)?;

            for fact in entity_facts {
                facts.push(fact.clone());

                // Add object to queue if not visited
                if !visited.contains(&fact.object) {
                    visited.insert(fact.object.clone());
                    queue.push_back((fact.object.clone(), depth + 1));
                }
            }
        }

        Ok(facts)
    }
}
```

### 4b.3: Entity Extraction

Add helper module `src-tauri/src/memory/entity_extraction.rs`:

```rust
use anyhow::Result;

/// Simple entity extraction (heuristic-based)
pub fn extract_entities_simple(query: &str) -> Vec<String> {
    let mut entities = HashSet::new();

    // Capitalized words (likely proper nouns)
    for word in query.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_alphanumeric());
        if let Some(first) = cleaned.chars().next() {
            if first.is_uppercase() && cleaned.len() > 1 {
                entities.insert(cleaned.to_lowercase());
            }
        }
    }

    // Known tech terms (maintain a dictionary)
    let tech_terms = [
        "react", "rust", "tauri", "mcp", "database", "api", "nebula",
        "sqlite", "tantivy", "llm", "openai", "anthropic", "claude",
    ];

    let query_lower = query.to_lowercase();
    for term in tech_terms {
        if query_lower.contains(term) {
            entities.insert(term.to_string());
        }
    }

    entities.into_iter().collect()
}

/// LLM-based entity extraction (more accurate, slower)
pub async fn extract_entities_llm(
    query: &str,
    provider: &dyn crate::llm::provider::LlmProvider,
) -> Result<Vec<String>> {
    let prompt = format!(
        r#"Extract key entities from this query. Return ONLY a JSON array of entity names.

Query: "{}"

Entities (people, technologies, projects, concepts):
"#,
        query
    );

    let messages = vec![crate::llm::provider::Message {
        id: None,
        role: "user".to_string(),
        content: Some(prompt),
        tool_calls: None,
        attachments: None,
        tool_call_id: None,
    }];

    let response = provider.chat(messages, vec![], None).await?;
    let content = response.content.unwrap_or_default();

    // Extract JSON array from response
    let json_str = if let Some(start) = content.find('[') {
        if let Some(end) = content.rfind(']') {
            &content[start..=end]
        } else {
            &content
        }
    } else {
        &content
    };

    serde_json::from_str(json_str)
        .map_err(|e| anyhow::anyhow!("Failed to parse entities: {}", e))
}
```

**Commit**: `feat(memory): add fact query methods and entity extraction`

---

## Phase 4c: Strategist Integration

**Goal**: Integrate fact retrieval into strategist planner and synthesizer

### 4c.1: Fact Retrieval Orchestration

Add to `src-tauri/src/memory/strategist.rs`:

```rust
use super::entity_extraction::extract_entities_simple;
use super::{Fact, RelevantFact};

/// Get relevant facts for a query
async fn get_relevant_facts(
    query: &str,
    conversation_id: &str,
    librarian: Arc<Mutex<Librarian>>,
) -> Result<Vec<RelevantFact>> {
    let mut facts = Vec::new();
    let lib = librarian.lock().await;

    // 1. USER PROFILE (always include for personalization)
    let profile = lib.get_user_profile_facts()?;
    facts.extend(profile.into_iter().map(|fact| RelevantFact {
        fact,
        relevance_score: 0.9,
        retrieval_reason: "user_profile".to_string(),
    }));

    // 2. DIRECT ENTITY MATCHES
    let entities = extract_entities_simple(query);
    for entity in &entities {
        let entity_facts = lib.get_facts_about(entity)?;
        facts.extend(entity_facts.into_iter().map(|fact| RelevantFact {
            fact,
            relevance_score: 1.0,
            retrieval_reason: "direct_match".to_string(),
        }));
    }

    // 3. PROJECT CONTEXT
    let project_facts = lib.get_conversation_project_facts(conversation_id)?;
    facts.extend(project_facts.into_iter().map(|fact| RelevantFact {
        fact,
        relevance_score: 0.8,
        retrieval_reason: "project_context".to_string(),
    }));

    // 4. RELATIONSHIP TRAVERSAL (1-2 hops)
    for entity in &entities {
        let related = lib.traverse_facts(entity, 2)?;
        facts.extend(related.into_iter().map(|fact| RelevantFact {
            fact,
            relevance_score: 0.6,
            retrieval_reason: "relationship".to_string(),
        }));
    }

    // 5. DEDUPLICATE & SORT
    facts.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut seen = HashSet::new();
    facts.retain(|rf| {
        let key = (&rf.fact.subject, &rf.fact.predicate, &rf.fact.object);
        seen.insert(key)
    });

    facts.truncate(20); // Limit to top 20

    Ok(facts)
}

/// Format facts for LLM consumption
fn format_facts(facts: &[RelevantFact]) -> String {
    if facts.is_empty() {
        return "(none)".to_string();
    }

    // Group by subject
    let mut grouped: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for rf in facts {
        grouped
            .entry(rf.fact.subject.clone())
            .or_default()
            .push(format!(
                "  - {} {} (confidence: {:.2})",
                rf.fact.predicate, rf.fact.object, rf.fact.confidence
            ));
    }

    grouped
        .iter()
        .map(|(subj, preds)| format!("{}:\n{}", subj, preds.join("\n")))
        .collect::<Vec<_>>()
        .join("\n\n")
}
```

### 4c.2: Update Planner Prompt

Modify `call_planner` in `strategist.rs`:

```rust
async fn call_planner(
    query: &str,
    recent_history: &[Message],
    context_turns: usize,
    initial_hits: &[MemoryHit],
    relevant_facts: &[RelevantFact],  // NEW
    provider: &dyn LlmProvider,
) -> Result<Option<SearchPlan>> {
    let recent_context = Self::format_recent_context(recent_history, context_turns);

    // Separate profile facts from context facts
    let profile_facts: Vec<_> = relevant_facts
        .iter()
        .filter(|rf| rf.retrieval_reason == "user_profile")
        .cloned()
        .collect();
    let context_facts: Vec<_> = relevant_facts
        .iter()
        .filter(|rf| rf.retrieval_reason != "user_profile")
        .cloned()
        .collect();

    let profile_block = if profile_facts.is_empty() {
        String::new()
    } else {
        format!("USER PROFILE:\n{}\n\n", format_facts(&profile_facts))
    };

    let context_block = if context_facts.is_empty() {
        String::new()
    } else {
        format!("KNOWN FACTS:\n{}\n\n", format_facts(&context_facts))
    };

    let hits_preview = /* ... same as before ... */;

    let prompt = format!(
        r#"You are a Memory Search Planner. Analyze the user query and decide if additional searches would help.

{}{}USER QUERY: {}{}

INITIAL SEARCH RESULTS ({} hits):
{}

TASK:
The USER PROFILE shows the user's background and preferences.
The KNOWN FACTS show what we already know about entities mentioned in the query.
Decide if additional targeted searches would fill gaps in context.

SEARCH PLAN SCHEMA:
{{
  "queries": [
    {{
      "q": "search query string",
      "limit": 5,
      "scope": "conversation_id",
      "roles": ["assistant", "user"],
      "max_age_days": 7
    }}
  ],
  "notes": "reasoning"
}}

GUIDELINES:
- If initial results + known facts are sufficient, return {{"queries": []}}
- Max 3 follow-up queries
- Use scope for conversation-local searches
- Use roles to target message types
- Keep queries focused

OUTPUT (JSON only, no prose):"#,
        profile_block,
        context_block,
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

    // ... rest of planner logic
}
```

### 4c.3: Update Synthesizer Prompt

Similar changes to `call_synthesizer`:

```rust
async fn call_synthesizer(
    query: &str,
    recent_history: &[Message],
    context_turns: usize,
    all_hits: &[MemoryHit],
    relevant_facts: &[RelevantFact],  // NEW
    provider: &dyn LlmProvider,
) -> Result<String> {
    let recent_context = Self::format_recent_context(recent_history, context_turns);

    let profile_facts: Vec<_> = relevant_facts
        .iter()
        .filter(|rf| rf.retrieval_reason == "user_profile")
        .cloned()
        .collect();
    let context_facts: Vec<_> = relevant_facts
        .iter()
        .filter(|rf| rf.retrieval_reason != "user_profile")
        .cloned()
        .collect();

    let profile_block = if profile_facts.is_empty() {
        String::new()
    } else {
        format!("USER PROFILE:\n{}\n\n", format_facts(&profile_facts))
    };

    let context_block = if context_facts.is_empty() {
        String::new()
    } else {
        format!("KNOWN FACTS:\n{}\n\n", format_facts(&context_facts))
    };

    let hits_block = /* ... same as before ... */;

    let prompt = format!(
        r#"You are a Memory Context Synthesizer. Create a concise context block for the main LLM.

{}{}USER QUERY: {}{}

RETRIEVED MEMORIES ({} hits):
---
{}
---

TASK:
Synthesize relevant facts into coherent context. The USER PROFILE enables personalization.
The KNOWN FACTS provide structured information about entities.
Combine with RETRIEVED MEMORIES to create comprehensive context.

GUIDELINES:
- Be concise but complete
- Remove duplicates
- Organize related facts
- Use profile facts for personalization
- If nothing relevant, say "No relevant context found."
- NO conversational filler

OUTPUT:"#,
        profile_block,
        context_block,
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

    // ... rest of synthesizer logic
}
```

### 4c.4: Update Orchestrator

Modify `assemble_context` to fetch facts:

```rust
pub async fn assemble_context(
    query: &str,
    recent_history: &[Message],
    librarian: Arc<Mutex<Librarian>>,
    context_turns: usize,
    context_model_id: Option<&str>,
    settings: &Settings,
    conversation_id: &str,  // NEW parameter
) -> Result<StrategistContextResult> {
    // Step 1: Initial retrieval (unchanged)
    let initial_options = SearchOptions {
        limit: 10,
        ..Default::default()
    };

    let initial_results = {
        let lib = librarian.lock().await;
        lib.search_with_options(query, initial_options)?
    };
    let initial_hits: Vec<MemoryHit> = /* ... same as before ... */;

    // Step 1b: Fetch relevant facts (NEW)
    let relevant_facts = get_relevant_facts(query, conversation_id, librarian.clone()).await?;

    // If no context model, return baseline
    let Some(model_id) = context_model_id else {
        return Ok(Self::baseline_context(&initial_hits, &relevant_facts));
    };

    // ... rest unchanged, but pass relevant_facts to planner/synthesizer
}
```

Update `baseline_context` to include facts:

```rust
fn baseline_context(hits: &[MemoryHit], facts: &[RelevantFact]) -> StrategistContextResult {
    let mut context_parts = Vec::new();

    // Add facts if present
    if !facts.is_empty() {
        let profile: Vec<_> = facts.iter()
            .filter(|f| f.retrieval_reason == "user_profile")
            .collect();
        if !profile.is_empty() {
            context_parts.push(format!("USER PROFILE:\n{}", format_facts(&profile)));
        }

        let other: Vec<_> = facts.iter()
            .filter(|f| f.retrieval_reason != "user_profile")
            .collect();
        if !other.is_empty() {
            context_parts.push(format!("KNOWN FACTS:\n{}", format_facts(&other)));
        }
    }

    // Add memory hits
    if !hits.is_empty() {
        let hits_text = hits
            .iter()
            .map(|h| format!("[{}] {}", h.role, h.snippet))
            .collect::<Vec<_>>()
            .join("\n\n");
        context_parts.push(format!("RELEVANT MEMORIES:\n{}", hits_text));
    }

    let context_text = if context_parts.is_empty() {
        String::new()
    } else {
        context_parts.join("\n\n")
    };

    StrategistContextResult {
        context_text,
        selected_message_ids: hits.iter().map(|h| h.message_id.clone()).collect(),
        search_plan: None,
    }
}
```

**Commit**: `feat(memory): integrate fact retrieval into strategist planner and synthesizer`

---

## Phase 4d: Fact Extraction Pipeline

**Goal**: Automatically extract facts from conversations

### 4d.1: Extraction Prompt

Add to `strategist.rs`:

```rust
async fn extract_facts_from_message(
    message: &Message,
    provider: &dyn LlmProvider,
) -> Result<Vec<(String, String, String, f32)>> {
    let content = message.content.clone().unwrap_or_default();
    if content.trim().is_empty() {
        return Ok(vec![]);
    }

    let prompt = format!(
        r#"Extract factual triplets from this message. Focus on durable facts about people, projects, preferences, and decisions.

MESSAGE (role: {}):
{}

Output JSON array of facts:
[
  {{"subject": "user", "predicate": "prefers", "object": "dark mode", "confidence": 0.9}},
  {{"subject": "project_x", "predicate": "uses", "object": "postgresql", "confidence": 1.0}}
]

GUIDELINES:
- subject: entity name (user, project name, technology, etc.)
- predicate: relationship type (prefers, uses, works_on, decided, expertise_in, etc.)
- object: value or related entity
- confidence: 0.0-1.0 based on certainty
- Only extract clear, factual statements
- Skip conversational filler and temporary state
- Output empty array [] if no facts

OUTPUT (JSON only):"#,
        message.role,
        content
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
            let json_str = if let Some(start) = content.find('[') {
                if let Some(end) = content.rfind(']') {
                    &content[start..=end]
                } else {
                    &content
                }
            } else {
                &content
            };

            #[derive(Deserialize)]
            struct FactExtraction {
                subject: String,
                predicate: String,
                object: String,
                confidence: f32,
            }

            let extractions: Vec<FactExtraction> = serde_json::from_str(json_str)
                .unwrap_or_default();

            Ok(extractions
                .into_iter()
                .map(|e| (e.subject, e.predicate, e.object, e.confidence))
                .collect())
        }
        Err(e) => {
            tracing::warn!("Fact extraction failed: {}", e);
            Ok(vec![])
        }
    }
}
```

### 4d.2: Background Extraction Service

Add to `lib.rs`:

```rust
async fn extract_facts_background(
    message_id: String,
    message: Message,
    librarian: Arc<Mutex<Librarian>>,
    settings: Settings,
) {
    // Only extract from user and assistant messages
    if message.role != "user" && message.role != "assistant" {
        return;
    }

    // Skip if no context_model configured
    let Some(context_model_id) = settings.context_model.as_ref() else {
        return;
    };

    let parts: Vec<&str> = context_model_id.split("::").collect();
    if parts.len() != 2 {
        return;
    }

    let (provider_id, model_name) = (parts[0], parts[1]);

    // Create provider
    let provider = match crate::memory::strategist::StrategistMemoryOrchestrator::create_provider(
        provider_id,
        model_name,
        &settings,
    ) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!("Failed to create provider for fact extraction: {}", e);
            return;
        }
    };

    // Extract facts
    match crate::memory::strategist::extract_facts_from_message(&message, provider.as_ref()).await {
        Ok(facts) => {
            let lib = librarian.lock().await;
            for (subject, predicate, object, confidence) in facts {
                if let Err(e) = lib.sqlite.save_fact(
                    &subject,
                    &predicate,
                    &object,
                    confidence,
                    Some(&message_id),
                ) {
                    tracing::error!("Failed to save fact: {}", e);
                }
            }
            tracing::debug!("Extracted {} facts from message {}", facts.len(), message_id);
        }
        Err(e) => {
            tracing::error!("Fact extraction failed: {}", e);
        }
    }
}
```

### 4d.3: Integrate into Message Pipeline

Modify `send_message_stream` in `lib.rs`:

```rust
// After saving each message (user and assistant)
if settings.memory_enabled {
    // Save message first
    lib.save_full_message(
        conversation_id,
        &msg.role,
        msg.content.as_deref(),
        /* ... */
    )?;

    // Extract facts in background (don't block)
    let msg_for_extraction = msg.clone();
    let librarian_clone = state.librarian.clone();
    let settings_clone = settings.clone();
    tokio::spawn(async move {
        extract_facts_background(
            msg_id,
            msg_for_extraction,
            librarian_clone,
            settings_clone,
        ).await;
    });
}
```

**Commit**: `feat(memory): add background fact extraction from conversations`

---

## Phase 4e: Frontend Visualization (Optional)

**Goal**: Display facts in Memory Panel

### 4e.1: Add Tauri Command

```rust
#[tauri::command]
async fn get_user_profile(state: State<'_, AppState>) -> Result<Vec<Fact>, String> {
    let lib = state.librarian.lock().await;
    lib.get_user_profile_facts()
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_facts_about(
    state: State<'_, AppState>,
    entity: String,
) -> Result<Vec<Fact>, String> {
    let lib = state.librarian.lock().await;
    lib.get_facts_about(&entity)
        .map_err(|e| e.to_string())
}
```

### 4e.2: Update MemoryPanel Component

Add facts tab to `src/components/MemoryPanel.tsx`:

```typescript
const [activeTab, setActiveTab] = useState<'memories' | 'facts'>('memories');
const [userProfile, setUserProfile] = useState<Fact[]>([]);

useEffect(() => {
  if (activeTab === 'facts') {
    invoke<Fact[]>('get_user_profile')
      .then(setUserProfile)
      .catch(console.error);
  }
}, [activeTab]);

// Render facts grouped by predicate
<div className="space-y-2">
  {Object.entries(groupBy(userProfile, 'predicate')).map(([predicate, facts]) => (
    <div key={predicate} className="p-2 bg-gray-800 rounded">
      <div className="font-semibold text-blue-400">{predicate}:</div>
      <ul className="ml-4 space-y-1">
        {facts.map(fact => (
          <li key={fact.id} className="text-gray-300">
            {fact.object}
            <span className="text-xs text-gray-500 ml-2">
              (confidence: {fact.confidence.toFixed(2)})
            </span>
          </li>
        ))}
      </ul>
    </div>
  ))}
</div>
```

**Commit**: `feat(ui): add facts visualization to memory panel`

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fact_crud() {
        let temp_dir = tempfile::tempdir().unwrap();
        let librarian = Librarian::new(temp_dir.path()).unwrap();

        // Save fact
        let id = librarian.sqlite.save_fact(
            "user",
            "prefers",
            "vim",
            1.0,
            None,
        ).unwrap();

        // Query fact
        let facts = librarian.query_facts(&[("user", "prefers", None)]).unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].object, "vim");

        // Delete fact
        librarian.sqlite.delete_fact(&id).unwrap();
        let facts = librarian.query_facts(&[("user", "prefers", None)]).unwrap();
        assert_eq!(facts.len(), 0);
    }

    #[test]
    fn test_entity_extraction() {
        let query = "How should I structure my React app?";
        let entities = extract_entities_simple(query);
        assert!(entities.contains(&"react".to_string()));
    }

    #[test]
    fn test_fact_traversal() {
        // Create test graph: user -> nebula -> tauri -> rust
        // Test traversal with different hop counts
    }
}
```

### Integration Tests

1. **Fact extraction accuracy**: Manually verify extracted facts from sample conversations
2. **Strategist with facts**: Test that facts improve context quality for queries like:
   - "What technologies do I use?" (should list from facts)
   - "What's my preferred editor?" (should use profile facts)
   - "Tell me about Nebula" (should combine facts + memories)
3. **Performance**: Measure latency impact of fact retrieval

### Manual Testing Checklist

- [ ] Facts table created on first launch
- [ ] Facts extracted from new messages
- [ ] User profile facts displayed in Memory Panel
- [ ] Strategist includes facts in planner prompt
- [ ] Strategist includes facts in synthesizer prompt
- [ ] Fact-based personalization works ("Since you're a...")
- [ ] Entity extraction finds relevant entities
- [ ] Graph traversal returns connected facts
- [ ] Confidence scores are reasonable

---

## Performance Considerations

### Indexing
- Multi-column indexes on (subject, predicate) and (subject) are critical
- Consider full-text index on object column for partial matching

### Caching
- Cache user profile facts (rarely change)
- Cache project context facts per conversation

### Batch Operations
- Extract facts in background (already planned)
- Batch insert facts if extracting from multiple messages

### Limits
- Cap fact traversal depth at 2-3 hops
- Limit total facts returned to 20-30
- Set confidence threshold for inclusion (e.g., >= 0.5)

---

## Expected Benefits

1. **Faster factual lookups**: O(1) indexed queries vs. semantic search
2. **Better personalization**: Explicit user profile enables "since you're a..." responses
3. **Relationship understanding**: Graph traversal finds connected facts
4. **Deduplication**: One canonical fact vs. many conversational mentions
5. **Provenance**: Track where facts came from (source_message_id)

---

## Trade-offs

1. **Extraction quality**: Depends on context_model capability
2. **Storage overhead**: Facts table grows with conversations
3. **Latency**: Fact queries add ~10-50ms to strategist pipeline
4. **Maintenance**: Facts can become stale (need update/deletion logic)
5. **Complexity**: More moving parts, more potential bugs

---

## Future Enhancements

### Phase 5a: Fact Validation & Conflict Resolution
- Detect contradictory facts (user prefers X vs. user prefers Y)
- Timestamp-based resolution (newer wins)
- Confidence-based merging

### Phase 5b: Semantic Entity Linking
- Embed entity names for fuzzy matching
- "React" == "ReactJS" == "React.js"
- Link mentions to canonical entities

### Phase 5c: Fact Decay
- Reduce confidence of old facts over time
- Prompt user to confirm/update stale facts

### Phase 5d: User-editable Facts
- UI to view, edit, delete facts manually
- Bulk import/export facts

### Phase 5e: Cross-conversation Insights
- "Show me all decisions I've made about authentication"
- "What are my top 5 most-used technologies?"
- Aggregate facts across conversations

---

## Migration from Phase 1-3

The knowledge graph layer is **additive** - it doesn't replace existing full-text search, it complements it. No changes needed to Phase 1-3 code except:

1. Add `conversation_id` parameter to `assemble_context` (Phase 4c)
2. Update `baseline_context` to include facts (Phase 4c)
3. Update planner/synthesizer prompts to include facts (Phase 4c)

All existing functionality remains intact.

---

## Success Metrics

After implementation, measure:

1. **Context quality**: User feedback on relevance of retrieved context
2. **Personalization**: Count of "profile fact" inclusions in responses
3. **Fact extraction accuracy**: Manual review of 50-100 extracted facts
4. **Query performance**: P50/P95 latency for fact retrieval
5. **Storage growth**: Facts table size over time

Target: 80%+ accuracy on fact extraction, <100ms P95 latency for fact queries.
