use serde::Serialize;
use serde_json::Value;

use mfs_semantic::ProcessingMode;
use mfs_semantic::chat_provider_from_env;

use crate::query_plan::{PlannedQuery, QueryPlan, QueryPlanMode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TypedQuery {
    pub query: String,
    pub context_type: String,
}

#[derive(Default)]
pub struct QueryPlanner {
    enable_llm: bool,
}

impl QueryPlanner {
    pub fn new(enable_llm: bool) -> Self {
        Self { enable_llm }
    }

    pub async fn plan_find(&self, query: &str) -> QueryPlan {
        if self.enable_llm {
            if let Some(plan) = self.try_llm_plan(query, None, QueryPlanMode::Find).await {
                return plan;
            }
        }
        self.deterministic_plan_find(query)
    }

    fn deterministic_plan_find(&self, query: &str) -> QueryPlan {
        QueryPlan {
            mode: QueryPlanMode::Find,
            typed_queries: base_queries(query)
                .into_iter()
                .map(|typed_query| PlannedQuery {
                    query: typed_query.query,
                    context_type: typed_query.context_type,
                    intent: "reference_lookup".to_owned(),
                    priority: 1,
                    source: "raw_query".to_owned(),
                })
                .collect(),
            skip_reason: None,
        }
    }

    pub async fn plan(&self, query: &str, session: Option<&str>) -> Vec<TypedQuery> {
        let query_plan = if session.is_some() {
            self.plan_search(query, session).await
        } else {
            self.plan_find(query).await
        };

        query_plan
            .typed_queries
            .into_iter()
            .map(|typed_query| TypedQuery {
                query: typed_query.query,
                context_type: typed_query.context_type,
            })
            .collect()
    }

    pub async fn plan_search(&self, query: &str, session: Option<&str>) -> QueryPlan {
        if is_non_retrieval_query(query) {
            return QueryPlan {
                mode: QueryPlanMode::Search,
                typed_queries: Vec::new(),
                skip_reason: Some("non_retrieval_query".to_owned()),
            };
        }

        if self.enable_llm {
            if let Some(plan) = self
                .try_llm_plan(query, session, QueryPlanMode::Search)
                .await
            {
                return plan;
            }
        }
        self.deterministic_plan_search(query, session)
    }

    async fn try_llm_plan(
        &self,
        query: &str,
        session: Option<&str>,
        mode: QueryPlanMode,
    ) -> Option<QueryPlan> {
        let provider = chat_provider_from_env();
        if provider.mode() == ProcessingMode::Degraded {
            return None;
        }

        let session_context = session.unwrap_or("None");
        let prompt = format!(
            r#"You are a query router for an Agent Context Engine.
Analyze the user query and generate exact search queries for three different context spaces:
- `resource` (documents, codes, files)
- `memory` (long-term facts, preferences, past events)
- `skill` (workflows, tools, scripts)

User Query: {query}
Session Context: {session_context}

Rules:
1. Extract the most precise search keywords (ignore conversational filler like "please", "find").
2. Only return queries for contexts that make sense (e.g., if asking to "run a build", target `skill` and possibly `resource`).
3. Priority 1 goes to the most directly relevant context, Priority 2 and 3 for supplementary contexts.

Return JSON only:
{{
  "queries": [
    {{
      "context_type": "resource|memory|skill",
      "query": "precise search keywords",
      "intent": "short intent phrase",
      "priority": 1
    }}
  ]
}}"#
        );

        let response = provider.complete(&prompt).await?;
        let json_str = strip_code_fences(&response);
        let value: Value = serde_json::from_str(json_str).ok()?;
        let queries = value.get("queries")?.as_array()?;

        let mut typed_queries = Vec::new();
        for q in queries {
            let context_type = q.get("context_type")?.as_str()?.to_owned();
            let query_text = q.get("query")?.as_str()?.to_owned();
            let intent = q
                .get("intent")
                .and_then(|v| v.as_str())
                .unwrap_or("general")
                .to_owned();
            let priority = q.get("priority").and_then(|v| v.as_u64()).unwrap_or(2) as u8;

            if !query_text.trim().is_empty() {
                typed_queries.push(PlannedQuery {
                    query: query_text,
                    context_type,
                    intent,
                    priority,
                    source: "llm_planner".to_owned(),
                });
            }
        }

        if typed_queries.is_empty() {
            return None;
        }

        typed_queries.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.context_type.cmp(&right.context_type))
                .then_with(|| left.query.cmp(&right.query))
        });
        typed_queries.dedup_by(|left, right| {
            left.context_type == right.context_type && left.query == right.query
        });

        Some(QueryPlan {
            mode,
            typed_queries,
            skip_reason: None,
        })
    }

    fn deterministic_plan_search(&self, query: &str, session: Option<&str>) -> QueryPlan {
        let trimmed = trimmed_query(query);
        let session_terms = session.map(session_terms).unwrap_or_default();
        let mut typed_queries = Vec::new();
        let mut push =
            |context_type: &str, query_text: String, intent: &str, priority: u8, source: &str| {
                if query_text.trim().is_empty() {
                    return;
                }
                typed_queries.push(PlannedQuery {
                    query: query_text,
                    context_type: context_type.to_owned(),
                    intent: intent.to_owned(),
                    priority,
                    source: source.to_owned(),
                });
            };

        push(
            "resource",
            query.to_owned(),
            "reference_lookup",
            2,
            "raw_query",
        );
        push("memory", query.to_owned(), "memory_recall", 3, "raw_query");

        if query_needs_skill(query) {
            push(
                "skill",
                trimmed.clone().unwrap_or_else(|| query.to_owned()),
                "workflow_execution",
                1,
                "intent_heuristic",
            );
        }

        if let Some(trimmed) = trimmed.clone().filter(|trimmed| trimmed != query) {
            push(
                "resource",
                trimmed.clone(),
                "reference_lookup",
                2,
                "trimmed_query",
            );
            if query_needs_skill(query) {
                push("skill", trimmed, "workflow_execution", 1, "trimmed_query");
            }
        }

        if !session_terms.is_empty() {
            let augmented = format!(
                "{} {}",
                trimmed.clone().unwrap_or_else(|| query.to_owned()),
                session_terms.join(" ")
            )
            .trim()
            .to_owned();
            if !augmented.is_empty() && augmented != query {
                push(
                    "resource",
                    augmented.clone(),
                    "session_refinement",
                    2,
                    "session_terms",
                );
                if query_needs_skill(query) {
                    push("skill", augmented, "workflow_execution", 1, "session_terms");
                }
            }
        }

        typed_queries.sort_by(|left, right| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| left.context_type.cmp(&right.context_type))
                .then_with(|| left.query.cmp(&right.query))
                .then_with(|| left.source.cmp(&right.source))
        });
        typed_queries.dedup_by(|left, right| {
            left.context_type == right.context_type && left.query == right.query
        });

        QueryPlan {
            mode: QueryPlanMode::Search,
            typed_queries,
            skip_reason: None,
        }
    }
}

fn strip_code_fences(s: &str) -> &str {
    let s = s.trim();
    if let Some(inner) = s.strip_prefix("```json") {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim();
        }
    }
    if let Some(inner) = s.strip_prefix("```") {
        if let Some(inner) = inner.strip_suffix("```") {
            return inner.trim();
        }
    }
    s
}

fn base_queries(query: &str) -> Vec<TypedQuery> {
    ["resource", "memory", "skill"]
        .into_iter()
        .map(|context_type| TypedQuery {
            query: query.to_owned(),
            context_type: context_type.to_owned(),
        })
        .collect()
}

fn trimmed_query(query: &str) -> Option<String> {
    let trimmed = query
        .split_whitespace()
        .filter(|token| {
            let normalized = token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase();
            !matches!(normalized.as_str(), "help" | "me" | "find" | "please")
        })
        .collect::<Vec<_>>()
        .join(" ");
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn session_terms(session: &str) -> Vec<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "about", "for", "the", "to", "of", "in", "on", "with", "my", "our",
        "recent", "session",
    ];

    let mut terms = Vec::new();
    for token in session.split_whitespace() {
        let normalized = token
            .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
            .to_ascii_lowercase();
        if normalized.len() < 4
            || STOPWORDS.contains(&normalized.as_str())
            || terms.iter().any(|existing| existing == &normalized)
        {
            continue;
        }
        terms.push(normalized);
        if terms.len() == 4 {
            break;
        }
    }
    terms
}

fn is_non_retrieval_query(query: &str) -> bool {
    let normalized = query
        .trim()
        .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "thanks" | "thankyou" | "thank you" | "ok" | "okay" | "cool" | "great"
    )
}

fn query_needs_skill(query: &str) -> bool {
    const SKILL_HINTS: &[&str] = &[
        "create", "build", "generate", "draft", "write", "plan", "workflow", "run", "execute",
        "fix",
    ];

    query
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| !ch.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .any(|token| SKILL_HINTS.contains(&token.as_str()))
}
