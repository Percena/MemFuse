//! Intent module — query intent classification for predicate routing.
//!
//! Strategy (signal灯塔 philosophy):
//! When LLM is available, uses it to semantically classify the query intent
//! across the full 8-category taxonomy. When LLM is unavailable, falls back
//! to bilingual keyword matching.
//!
//! The intent result provides predicate prefixes that guide fact routing —
//! telling the agent "what kind of information to look for", not the exact answer.

use crate::llm::{LlmAssist, LlmIntent, build_intent_prompt, parse_llm_json};
use crate::{DEFAULT_GENERIC_FACT_LIMIT, FactEntry, contains_any};

/// Query intent classification result.
#[derive(Debug, Clone)]
pub struct IntentResult {
    /// Matched predicate prefixes (e.g., "location.", "work.", "entities.", "cases.").
    pub matched_predicates: Vec<String>,
    /// Whether the query requests detailed information.
    pub is_detail_query: bool,
    /// Whether the query is cross-thread (references past sessions).
    pub is_cross_thread: bool,
}

/// Classify query intent using LLM (semantic) or keyword matching (fallback).
pub async fn classify_intent(query: &str, llm: &LlmAssist) -> IntentResult {
    let keyword_result = classify_intent_keywords(query);
    // Try LLM classification first.
    if let Some(mut result) = try_llm_classify_intent(query, llm).await {
        for predicate in keyword_result.matched_predicates {
            if !result.matched_predicates.contains(&predicate) {
                result.matched_predicates.push(predicate);
            }
        }
        result.is_detail_query |= keyword_result.is_detail_query;
        result.is_cross_thread |= keyword_result.is_cross_thread;
        return result;
    }
    // Fall back to keyword matching.
    keyword_result
}

/// LLM-based intent classification.
async fn try_llm_classify_intent(query: &str, llm: &LlmAssist) -> Option<IntentResult> {
    if !llm.is_available() {
        return None;
    }

    let prompt = build_intent_prompt(query);
    let response = llm.complete(&prompt).await?;

    let parsed: LlmIntent = parse_llm_json(&response)?;

    // Validate: ensure predicate prefixes have valid format (category.sub or category.)
    let valid_predicates = parsed
        .predicate_prefixes
        .iter()
        .filter(|p| {
            let categories = [
                "profile",
                "preferences",
                "entities",
                "events",
                "cases",
                "patterns",
                "tools",
                "skills",
                "location",
                "identity",
                "health",
                "diet",
                "work",
                "language",
                "communication",
                "project",
                "procedure",
                "convention",
                "environment",
            ];
            categories.iter().any(|cat| p.starts_with(cat))
        })
        .cloned()
        .collect::<Vec<_>>();

    if valid_predicates.is_empty() {
        return None;
    }

    Some(IntentResult {
        matched_predicates: valid_predicates,
        is_detail_query: parsed.is_detail,
        is_cross_thread: parsed.is_cross_thread,
    })
}

/// Keyword-based intent classification (fallback when LLM is unavailable).
/// Bilingual keyword matching for intent classification.
/// Public so that downstream handlers can reuse the zero-cost cross-thread
/// and detail-query detection on latency-sensitive read paths.
pub fn classify_intent_keywords(query: &str) -> IntentResult {
    let lower = query.to_lowercase();
    let mut predicates: Vec<String> = Vec::new();

    // location.*
    if contains_any(
        &lower,
        &[
            "live", "moved", "location", "city", "住", "搬", "哪", "哪里", "住哪", "住在",
        ],
    ) {
        predicates.push("location.".to_owned());
    }
    // health.*
    if contains_any(
        &lower,
        &[
            "allergy",
            "allergies",
            "allergic",
            "过敏",
            "constraint",
            "限制",
        ],
    ) {
        predicates.push("health.".to_owned());
    }
    // work.* + profile.role
    if contains_any(
        &lower,
        &[
            "work",
            "job",
            "role",
            "company",
            "工作",
            "职业",
            "公司",
            "职位",
            "做什么",
            "什么工作",
            "工程师",
            "设计师",
            "产品经理",
        ],
    ) {
        predicates.push("work.".to_owned());
        predicates.push("profile.".to_owned());
    }
    // identity.* + profile.name
    if contains_any(&lower, &["name", "名字", "叫", "是谁", "什么名字", "who"]) {
        predicates.push("identity.".to_owned());
        predicates.push("profile.".to_owned());
    }
    if contains_any(&lower, &["pronoun", "pronouns"]) {
        predicates.push("identity.".to_owned());
    }
    // diet.* + preference.*
    if contains_any(
        &lower,
        &["food", "eat", "diet", "spicy", "吃", "饮食", "辣"],
    ) {
        predicates.push("diet.".to_owned());
        predicates.push("preference.".to_owned());
    }
    // preference.*
    if contains_any(
        &lower,
        &[
            "communicate",
            "communication style",
            "沟通",
            "交流",
            "表达方式",
        ],
    ) {
        predicates.push("preference.".to_owned());
    }
    // preference.coding_style / preferences.coding_style
    if contains_any(
        &lower,
        &[
            "coding",
            "programming",
            "language",
            "tech stack",
            "技术栈",
            "编程",
            "写代码",
            "用什么",
            "什么语言",
            "什么框架",
        ],
    ) {
        predicates.push("preferences.".to_owned());
        predicates.push("preference.".to_owned());
    }
    // language.* — "language" is ambiguous (spoken vs programming), so it also matches preferences above
    if contains_any(
        &lower,
        &[
            "language",
            "speak",
            "languages",
            "spoken",
            "会说",
            "语言",
            "说话",
            "说什么语言",
        ],
    ) {
        predicates.push("language.".to_owned());
    }
    // project.* + entities.project
    if contains_any(
        &lower,
        &[
            "project",
            "working on",
            "building",
            "developing",
            "项目",
            "开发",
            "开发的",
            "做的项目",
            "做什么项目",
        ],
    ) {
        predicates.push("project.".to_owned());
        predicates.push("entities.".to_owned());
    }
    // entities.architecture_decision — decisions, migrations, tech choices
    if contains_any(
        &lower,
        &[
            "decided",
            "decision",
            "决定",
            "选了",
            "架构",
            "迁移",
            "migrated",
            "switched",
            "database",
            "数据库",
            "db",
            "选了什么",
        ],
    ) {
        predicates.push("entities.".to_owned());
    }
    // cases.*
    if contains_any(
        &lower,
        &[
            "bug",
            "fix",
            "resolved",
            "问题",
            "修复",
            "解决",
            "performance",
            "性能",
            "优化",
        ],
    ) {
        predicates.push("cases.".to_owned());
    }
    // events.*
    if contains_any(&lower, &["event", "happened", "事件", "发生"]) {
        predicates.push("events.".to_owned());
    }
    // patterns.*
    if contains_any(&lower, &["pattern", "how to", "workflow", "模式", "流程"]) {
        predicates.push("patterns.".to_owned());
    }
    // tools.*
    if contains_any(&lower, &["tool", "framework", "library", "工具", "框架"]) {
        predicates.push("tools.".to_owned());
    }
    // skills.*
    if contains_any(&lower, &["skill", "technique", "技巧", "技能"]) {
        predicates.push("skills.".to_owned());
    }
    // procedure.* — build/test/deploy commands
    if contains_any(
        &lower,
        &[
            "build",
            "compile",
            "make",
            "test",
            "deploy",
            "run",
            "构建",
            "编译",
            "跑",
            "测试",
            "部署",
            "运行",
            "命令",
            "怎么跑",
            "怎么构建",
            "怎么测试",
            "怎么部署",
        ],
    ) {
        predicates.push("procedure.".to_owned());
    }
    // convention.* — tool/naming choices
    if contains_any(
        &lower,
        &[
            "convention",
            "style",
            "naming",
            "coding style",
            "规范",
            "约定",
            "命名",
            "习惯",
            "用什么",
            "不用",
            "偏好用",
        ],
    ) {
        predicates.push("convention.".to_owned());
        predicates.push("preference.".to_owned());
    }
    // environment.* — CI/runtime versions
    if contains_any(
        &lower,
        &[
            "environment",
            "runtime",
            "version",
            "ci",
            "pipeline",
            "platform",
            "环境",
            "版本",
            "CI",
            "流水线",
            "运行环境",
            "平台",
        ],
    ) {
        predicates.push("environment.".to_owned());
    }

    let is_detail = contains_any(
        &lower,
        &[
            "what exactly",
            "which day",
            "quote",
            "details",
            "last time",
            "原话",
            "哪天",
            "具体",
            "细节",
            "上次怎么说",
            "exactly",
        ],
    );
    let is_cross_thread = contains_any(
        &lower,
        &[
            "across threads",
            "across thread",
            "across sessions",
            "other threads",
            "other sessions",
            "cross-thread",
            "cross thread",
            "cross-session",
            "cross session",
            "across conversations",
            "跨线程",
            "跨会话",
            "其他线程",
            "其他会话",
            "last time",
            "previous session",
            "earlier",
            "before",
            "yesterday",
        ],
    );

    IntentResult {
        matched_predicates: predicates,
        is_detail_query: is_detail,
        is_cross_thread,
    }
}

/// Check if an intent matches a given predicate.
pub fn intent_matches_predicate(intent: &IntentResult, predicate: &str) -> bool {
    intent
        .matched_predicates
        .iter()
        .any(|prefix| predicate.starts_with(prefix) || predicate_alias_matches(prefix, predicate))
}

fn predicate_alias_matches(prefix: &str, predicate: &str) -> bool {
    match prefix {
        "location." => predicate == "profile.location",
        "profile." => predicate.starts_with("location.") || predicate.starts_with("identity."),
        "preference." => predicate.starts_with("preferences."),
        "preferences." => predicate.starts_with("preference."),
        "project." => predicate == "entities.project",
        "entities." => predicate == "project.active",
        "procedure." => predicate == "preference.coding_style",
        _ => false,
    }
}

/// Route facts for a query using intent classification.
///
/// If intent matches any facts, return those (sorted by confidence).
/// If no match, return up to DEFAULT_GENERIC_FACT_LIMIT facts as fallback.
pub async fn route_facts_for_query(
    facts: &[FactEntry],
    query: &str,
    llm: &LlmAssist,
) -> Vec<FactEntry> {
    let intent = classify_intent(query, llm).await;
    route_facts_for_intent(facts, query, &intent)
}

/// Route facts for a query using a precomputed intent result.
///
/// This keeps latency-sensitive read paths from re-running intent
/// classification after they already decided whether LLM assistance is allowed.
pub fn route_facts_for_intent(
    facts: &[FactEntry],
    query: &str,
    intent: &IntentResult,
) -> Vec<FactEntry> {
    if facts.is_empty() {
        return Vec::new();
    }

    let mut matched: Vec<FactEntry> = facts
        .iter()
        .filter(|f| intent_matches_predicate(intent, &f.predicate))
        .cloned()
        .collect();

    if !matched.is_empty() {
        sort_fact_entries(&mut matched);
        return matched;
    }

    // Fallback 1: basic keyword match against display_value
    let query_lower = query.to_lowercase();
    let query_tokens: Vec<&str> = query_lower.split_whitespace().collect();
    let mut keyword_matched: Vec<FactEntry> = facts
        .iter()
        .filter(|f| {
            let dv = f.display_value.to_lowercase();
            query_tokens.iter().any(|t| t.len() >= 2 && dv.contains(t))
        })
        .cloned()
        .collect();
    if !keyword_matched.is_empty() {
        sort_fact_entries(&mut keyword_matched);
        keyword_matched.truncate(DEFAULT_GENERIC_FACT_LIMIT);
        return keyword_matched;
    }

    // Fallback 2: generic facts sorted by confidence, capped
    let mut fallback: Vec<FactEntry> = facts.to_vec();
    sort_fact_entries(&mut fallback);
    fallback.truncate(DEFAULT_GENERIC_FACT_LIMIT);
    fallback
}

/// Sort fact entries by confidence (descending), then predicate (ascending), then fact_id.
fn sort_fact_entries(facts: &mut [FactEntry]) {
    facts.sort_by(|a, b| {
        if a.confidence != b.confidence {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        } else if a.predicate != b.predicate {
            a.predicate.cmp(&b.predicate)
        } else {
            a.fact_id.cmp(&b.fact_id)
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FactEntry;
    use mfs_test_util::env_isolated;

    fn make_llm() -> LlmAssist {
        LlmAssist::from_env()
    }

    #[tokio::test]
    async fn classify_location_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("What city do I live in?", &llm).await;
        // LLM may or may not be available; keyword fallback should work
        assert!(result.matched_predicates.contains(&"location.".to_owned()));
    }

    #[tokio::test]
    async fn classify_chinese_location_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("我住在哪里？", &llm).await;
        assert!(result.matched_predicates.contains(&"location.".to_owned()));
    }

    #[tokio::test]
    async fn classify_work_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("What's my job?", &llm).await;
        assert!(result.matched_predicates.contains(&"work.".to_owned()));
    }

    #[tokio::test]
    async fn classify_health_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("I have allergies to pollen", &llm).await;
        assert!(result.matched_predicates.contains(&"health.".to_owned()));
    }

    #[tokio::test]
    async fn classify_project_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("What project am I working on?", &llm).await;
        assert!(result.matched_predicates.contains(&"project.".to_owned()));
    }

    #[tokio::test]
    async fn classify_detail_query() {
        let llm = make_llm();
        let result = classify_intent("What exactly did I say last time?", &llm).await;
        assert!(result.is_detail_query);
        // keyword fallback should detect cross-thread
        assert!(result.is_cross_thread);
    }

    #[tokio::test]
    async fn classify_cross_thread() {
        let llm = make_llm();
        let result = classify_intent("Search across threads for my preferences", &llm).await;
        assert!(result.is_cross_thread);
    }

    #[test]
    fn test_intent_matches_predicate() {
        let intent = IntentResult {
            matched_predicates: vec!["location.".to_owned()],
            is_detail_query: false,
            is_cross_thread: false,
        };
        assert!(intent_matches_predicate(&intent, "location.current_city"));
        assert!(!intent_matches_predicate(&intent, "work.current_role"));
    }

    #[tokio::test]
    async fn route_facts_with_match() {
        let llm = make_llm();
        let facts = vec![
            FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "location.current_city".to_owned(),
                display_value: "Tokyo".to_owned(),
                confidence: 0.9,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "work.current_role".to_owned(),
                display_value: "Engineer".to_owned(),
                confidence: 0.8,
                staleness_note: None,
                valid_from: None,
            },
        ];
        let routed = route_facts_for_query(&facts, "Where do I live?", &llm).await;
        assert!(!routed.is_empty());
        assert_eq!(routed[0].predicate, "location.current_city");
    }

    #[tokio::test]
    async fn route_facts_fallback() {
        let llm = make_llm();
        let facts = vec![
            FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "work.current_role".to_owned(),
                display_value: "Engineer".to_owned(),
                confidence: 0.8,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "identity.name".to_owned(),
                display_value: "John".to_owned(),
                confidence: 0.9,
                staleness_note: None,
                valid_from: None,
            },
        ];
        let routed = route_facts_for_query(&facts, "weather today", &llm).await; // no match
        assert!(!routed.is_empty());
        assert!(routed.len() <= DEFAULT_GENERIC_FACT_LIMIT);
    }

    #[tokio::test]
    async fn route_facts_bridges_profile_and_procedure_aliases() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let facts = vec![
            FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "profile.location".to_owned(),
                display_value: "User currently lives in Tokyo".to_owned(),
                confidence: 0.9,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "preference.coding_style".to_owned(),
                display_value: "Run cargo test to run the tests".to_owned(),
                confidence: 0.8,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f3".to_owned(),
                predicate: "entities.project".to_owned(),
                display_value: "User is working on Project Lantern".to_owned(),
                confidence: 0.95,
                staleness_note: None,
                valid_from: None,
            },
        ];

        let routed = route_facts_for_query(
            &facts,
            "What city and test command should I remember?",
            &llm,
        )
        .await;
        let predicates = routed
            .iter()
            .map(|fact| fact.predicate.as_str())
            .collect::<Vec<_>>();

        assert!(
            predicates.contains(&"profile.location"),
            "routed predicates: {predicates:?}"
        );
        assert!(
            predicates.contains(&"preference.coding_style"),
            "routed predicates: {predicates:?}"
        );
    }

    #[test]
    fn route_facts_with_precomputed_keyword_intent() {
        let facts = vec![
            FactEntry {
                fact_id: "f1".to_owned(),
                predicate: "profile.location".to_owned(),
                display_value: "User currently lives in Tokyo".to_owned(),
                confidence: 0.9,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f2".to_owned(),
                predicate: "preference.coding_style".to_owned(),
                display_value: "Run cargo test to run the tests".to_owned(),
                confidence: 0.8,
                staleness_note: None,
                valid_from: None,
            },
            FactEntry {
                fact_id: "f3".to_owned(),
                predicate: "entities.project".to_owned(),
                display_value: "User is working on Project Lantern".to_owned(),
                confidence: 0.95,
                staleness_note: None,
                valid_from: None,
            },
        ];
        let intent = classify_intent_keywords("Where do I live, and how do I run tests?");

        let routed =
            route_facts_for_intent(&facts, "Where do I live, and how do I run tests?", &intent);
        let predicates = routed
            .iter()
            .map(|fact| fact.predicate.as_str())
            .collect::<Vec<_>>();

        assert!(
            predicates.contains(&"profile.location"),
            "routed predicates: {predicates:?}"
        );
        assert!(
            predicates.contains(&"preference.coding_style"),
            "routed predicates: {predicates:?}"
        );
        assert!(
            !predicates.contains(&"entities.project"),
            "routed predicates: {predicates:?}"
        );
    }

    #[tokio::test]
    async fn route_facts_empty() {
        let llm = make_llm();
        let routed = route_facts_for_query(&[], "anything", &llm).await;
        assert!(routed.is_empty());
    }

    // ── Procedural predicate intent tests (§4.2) ──

    #[tokio::test]
    async fn classify_procedure_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("How do I build this project?", &llm).await;
        assert!(result.matched_predicates.contains(&"procedure.".to_owned()));
    }

    #[tokio::test]
    async fn classify_procedure_intent_chinese() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("怎么构建项目？", &llm).await;
        assert!(result.matched_predicates.contains(&"procedure.".to_owned()));
    }

    #[tokio::test]
    async fn classify_convention_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("What naming convention do we use?", &llm).await;
        assert!(
            result
                .matched_predicates
                .contains(&"convention.".to_owned())
        );
    }

    #[tokio::test]
    async fn classify_convention_intent_chinese() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("命名规范是什么？", &llm).await;
        assert!(
            result
                .matched_predicates
                .contains(&"convention.".to_owned())
        );
    }

    #[tokio::test]
    async fn classify_environment_intent_keywords() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("What CI pipeline is this project on?", &llm).await;
        assert!(
            result
                .matched_predicates
                .contains(&"environment.".to_owned())
        );
    }

    #[tokio::test]
    async fn classify_environment_intent_chinese() {
        let _env_guard = env_isolated();
        let llm = make_llm();
        let result = classify_intent("运行环境是什么？", &llm).await;
        assert!(
            result
                .matched_predicates
                .contains(&"environment.".to_owned())
        );
    }
}
