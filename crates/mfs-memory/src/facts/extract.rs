//! Fact extraction — LLM-assisted + rule-based extraction logic.
//!
//! Extraction strategy (signal灯塔 philosophy):
//! 1. When LLM is available, use it to extract facts across the full 8-category
//!    taxonomy (profile, preferences, entities, events, cases, patterns, tools, skills).
//! 2. When LLM is unavailable or returns unparseable output, fall back to the
//!    45-rule regex system covering 21 predicates.

use regex::Regex;
use std::sync::LazyLock;
use tracing::{debug, warn};

use crate::llm::{
    LlmAssist, LlmFact, build_fact_extraction_prompt, build_fact_extraction_prompt_simple,
    parse_llm_json,
};
use crate::{ConversationTurn, FactAssertion, FactOperation, TurnRole};

// ─── Extraction Rules ──────────────────────────────────────────────────

struct ExtractionRule {
    pattern: Regex,
    predicate: String,
    value_type: String, // scalar, set, temporal
    operation: FactOperation,
    confidence: f64,
    capture_group: usize,
}

static EXTRACTION_RULES: LazyLock<Vec<ExtractionRule>> = LazyLock::new(build_extraction_rules);

/// Build the V1 extraction rules (ordered from most specific to least).
fn build_extraction_rules() -> Vec<ExtractionRule> {
    let rules: Vec<(&str, &str, &str, FactOperation, f64, usize)> = vec![
        // location.current_city — explicit past/current phrasing (Chinese)
        (
            r"(?:我以前住在[^\s,，。.!！?？]{2,20}[，,]?\s*现在住在\s*([^\s,，。.!！?？]{2,20}))",
            "location.current_city",
            "scalar",
            FactOperation::Update,
            0.90,
            1,
        ),
        // location.current_city — explicit past/current phrasing (English)
        (
            r"(?i)(?:i used to live in [^\s,，。.!！?？]{2,20}[,]?\s*now (?:live|am living) in\s*([^\s,，。.!！?？]{2,20}))",
            "location.current_city",
            "scalar",
            FactOperation::Update,
            0.90,
            1,
        ),
        // location.current_city — retract/update (Chinese)
        (
            r"(?:我(?:已经|已|刚|搬到|现在住在|现在在)\s*([^\s,，。.!！?？]{2,20}))",
            "location.current_city",
            "scalar",
            FactOperation::Update,
            0.85,
            1,
        ),
        // location.current_city — retract/update (English)
        (
            r"(?i)(?:i\s+(?:just\s+)?moved to|i\s+now\s+(?:live|am living)\s+in)\s*([^\s,，。.!！?？]{2,20})",
            "location.current_city",
            "scalar",
            FactOperation::Update,
            0.85,
            1,
        ),
        // location.current_city — assertion (Chinese)
        (
            r"(?:我(?:住在|在)\s*([^\s,，。.!！?？]{2,20}))",
            "location.current_city",
            "scalar",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // location.current_city — assertion (English)
        (
            r"(?i)(?:i\s+live(?:s)?\s+in)\s*([^\s,，。.!！?？]{2,20})",
            "location.current_city",
            "scalar",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // location.current_country — English
        (
            r"(?i)(?:from|born in|resident of|citizen of)\s+([A-Z][a-zA-Z\s]{2,25})",
            "location.current_country",
            "scalar",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // location.current_country — Chinese
        (
            r"(?:住在|来自|出生在|移民到|搬到)([^\s,，。.!！?？]{2,15})国",
            "location.current_country",
            "scalar",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // location.current_country — Chinese update
        (
            r"(?:现在住在|搬到|移居)([^\s,，。.!！?？]{2,15})国",
            "location.current_country",
            "scalar",
            FactOperation::Update,
            0.85,
            1,
        ),
        // identity.name (bilingual)
        (
            r"(?i)(?:我(?:叫|的名字是|名字叫)|my name is|i(?:'m| am) called)\s*([^\s,，。.!！?？]{1,20})",
            "identity.name",
            "scalar",
            FactOperation::Assert,
            0.90,
            1,
        ),
        // work.current_role
        (
            r"(?i)(?:我(?:是|做|担任)|i(?:'m| am)(?: a| an)?)\s*(工程师|程序员|设计师|产品经理|manager|engineer|developer|designer|teacher|doctor|lawyer|[^\s,，。.!！?？]{2,20}(?:师|员|官|长|者))",
            "work.current_role",
            "scalar",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // work.current_company
        (
            r"(?i)(?:我(?:在|就职于|供职于)|i work (?:at|for))\s*([^\s,，。.!！?？]{2,30})",
            "work.current_company",
            "scalar",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // health.allergy (Chinese)
        (
            r"(?:我对\s*([^\s,，。.!！?？]{1,20})过敏)",
            "health.allergy",
            "set",
            FactOperation::Assert,
            0.90,
            1,
        ),
        // health.allergy (English)
        (
            r"(?i)(?:i\s+am\s+allergic\s+to|i\s+have\s+(?:an?\s+)?allergy\s+to)\s*([^\s,，。.!！?？]{1,20})",
            "health.allergy",
            "set",
            FactOperation::Assert,
            0.90,
            1,
        ),
        // health.constraint (Chinese)
        (
            r"(?:我不能(?:吃|喝|用)\s*([^\s,，。.!！?？]{1,20}))",
            "health.constraint",
            "set",
            FactOperation::Assert,
            0.82,
            1,
        ),
        // health.constraint (English)
        (
            r"(?i)(?:i can't (?:eat|drink|use))\s*([^\s,，。.!！?？]{1,20})",
            "health.constraint",
            "set",
            FactOperation::Assert,
            0.82,
            1,
        ),
        // diet.spicy_preference — retract
        (
            r"(?i)(?:已经|已|不再|不吃|戒了|戒掉|stopped eating|no longer eat|can't eat|戒)(?:\s*吃)?\s*辣|(?:不能|不可以|不应该)\s*吃\s*辣",
            "diet.spicy_preference",
            "temporal",
            FactOperation::Retract,
            0.85,
            0,
        ),
        // diet.spicy_preference — assert
        (
            r"(?i)(?:我|i)\s*(?:喜欢|爱吃|love|like|enjoy)\s*(?:吃\s*)?(?:辣|spicy)",
            "diet.spicy_preference",
            "temporal",
            FactOperation::Assert,
            0.80,
            0,
        ),
        // preference.food
        (
            r"(?i)(?:我|i)\s*(?:喜欢吃|最爱吃|love eating|favorite food is)\s*([^\s,，。.!！?？]{1,20})",
            "preference.food",
            "set",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // preference.communication_style (Chinese)
        (
            r"(?:请用\s*([^\s,，。.!！?？]{1,10})(?:一点)?的方式和我沟通)",
            "preference.communication_style",
            "scalar",
            FactOperation::Assert,
            0.78,
            1,
        ),
        // preference.communication_style (English)
        (
            r"(?i)(?:please (?:be|communicate) (?:more )?(concise|direct|detailed) (?:with me)?)",
            "preference.communication_style",
            "scalar",
            FactOperation::Assert,
            0.78,
            1,
        ),
        // identity.pronouns
        (
            r"(?i)(?:my pronouns are|i use)\s*([a-z/]{2,20})",
            "identity.pronouns",
            "scalar",
            FactOperation::Assert,
            0.88,
            1,
        ),
        // language.spoken
        (
            r"(?i)(?:我|i)\s*(?:会说|speak|会)\s*([^\s,，。.!！?？]{2,20})(?:语|文|话|language)?",
            "language.spoken",
            "set",
            FactOperation::Assert,
            0.75,
            1,
        ),
        // project.active (Chinese)
        (
            r"(?:我(?:正在|在做|在开发|在搞|在写)\s*([^\s,，。.!！?？]{2,30})(?:项目|工程|系统)?)",
            "project.active",
            "set",
            FactOperation::Assert,
            0.70,
            1,
        ),
        // project.active (English)
        (
            r"(?i)(?:i(?:'m| am) (?:working on|building|developing))\s+([^\n,，。.!！?？]{2,50})",
            "project.active",
            "set",
            FactOperation::Assert,
            0.70,
            1,
        ),
        // ── Technical migration (entities.architecture_decision) — safety net ──
        // Chinese: "从 X 迁移到 Y" (extract Y as new value)
        (
            r"(?:从|由)\s*([^\s,，。.!！?？]{2,20})\s*(?:迁移|切换|换|迁到)(?:到|至|为)?\s*([^\s,，。.!！?？]{2,20})",
            "entities.architecture_decision",
            "scalar",
            FactOperation::Update,
            0.85,
            2,
        ),
        // English: "migrated from X to Y" / "switched from X to Y" (extract Y)
        (
            r"(?i)(?:migrated|switched|moved|changed|replaced)\s+(?:from\s+)?([^\s,，。.!！?？]{2,20})\s+(?:to|with)\s+([^\s,，。.!！?？]{2,20})",
            "entities.architecture_decision",
            "scalar",
            FactOperation::Update,
            0.85,
            2,
        ),
        // Chinese: "用 X 替代/替换 Y" (extract X as new value)
        (
            r"(?:用|使用)\s*([^\s,，。.!！?？]{2,20})\s*(?:替代|替换|代替)\s*([^\s,，。.!！?？]{2,20})",
            "entities.architecture_decision",
            "scalar",
            FactOperation::Update,
            0.85,
            1,
        ),
        // Chinese: "升级到 X"
        (
            r"(?:升级到|升级为|更新到|更新为)\s*([^\s,，。.!！?？]{2,20})",
            "entities.architecture_decision",
            "scalar",
            FactOperation::Update,
            0.82,
            1,
        ),
        // English: "switched/migrated/moved to X" (single-destination, no "from")
        (
            r"(?i)(?:switched|migrated|moved)\s+to\s+([^\s,，。.!！?？]{2,20})",
            "entities.architecture_decision",
            "scalar",
            FactOperation::Update,
            0.82,
            1,
        ),
        // ── Procedural memory (procedure/convention/environment) — §4.2 ──
        // procedure.build_command — how to build this project
        (
            r"(?i)(?:build(?:s|ing)?\s+(?:this|the|my)\s+(?:project|repo|repository|codebase|app)\s+(?:with|using|by\s+running)\s+([^\s,，。.!！?？]{2,30}))",
            "procedure.build_command",
            "scalar",
            FactOperation::Assert,
            0.82,
            1,
        ),
        // procedure.build_command — "run X to build" / "build with X"
        (
            r"(?i)(?:run|use|execute)\s+([^\s,，。.!！?？]+(?:\s+[^\s,，。.!！?？]+)?)\s+to\s+(?:build|compile|make)",
            "procedure.build_command",
            "scalar",
            FactOperation::Assert,
            0.80,
            1,
        ),
        // procedure.build_command (Chinese) — "用 X 构建/编译项目"
        (
            r"(?:用\s*([^\s,，。.!！?？]{2,30})\s*(?:构建|编译|打包|跑|运行)\s*(?:项目|工程|代码|程序|app))",
            "procedure.build_command",
            "scalar",
            FactOperation::Assert,
            0.80,
            1,
        ),
        // procedure.test_command — how to run tests
        (
            r"(?i)(?:run|use|execute)\s+([^\s,，。.!！?？]+(?:\s+[^\s,，。.!！?？]+)?)\s+to\s+(?:run\s+)?(?:the\s+)?(?:tests?|unit\s+tests?|integration\s+tests?)",
            "procedure.test_command",
            "scalar",
            FactOperation::Assert,
            0.80,
            1,
        ),
        // procedure.test_command (Chinese) — "用 X 跑测试"
        (
            r"(?:(?:跑|运行|执行|用)\s*([^\s,，。.!！?？]{2,30})\s*(?:测试|跑测试|跑一下测试|跑单元测试|跑集成测试))",
            "procedure.test_command",
            "scalar",
            FactOperation::Assert,
            0.80,
            1,
        ),
        // procedure.deploy_step — deployment sequence
        (
            r"(?i)(?:deploy(?:ing|s)?\s+(?:to|this|the|staging|production|prod)\s+[^.]*?(?:first|then|before|after|need\s+to|run)\s+([^\s,，。.!！?？]{2,40}))",
            "procedure.deploy_step",
            "scalar",
            FactOperation::Assert,
            0.78,
            1,
        ),
        // procedure.deploy_step (Chinese) — "部署到 X 先/需要跑 Y"
        (
            r"(?:部署[^.]*?(?:先|需要|要|得先)\s*(?:跑|运行|执行)\s*([^\s,，。.!！?？]{2,40}))",
            "procedure.deploy_step",
            "scalar",
            FactOperation::Assert,
            0.78,
            1,
        ),
        // convention.tool — "use X not Y" / "prefer X over Y for ..."
        (
            r"(?i)(?:use|prefer|always\s+use|should\s+use|we\s+use|this\s+project\s+uses)\s+([^\s,，。.!！?？]{2,20})\s+(?:not|instead\s+of|over|rather\s+than|don't\s+use)\s+([^\s,，。.!！?？]{2,20})",
            "convention.tool",
            "scalar",
            FactOperation::Update,
            0.85,
            1,
        ),
        // convention.tool (Chinese) — "用 X 不用 Y" / "这个项目用 X"
        (
            r"(?:这个(?:项目|工程|仓库|代码库)用|我们(?:用|使用|偏好))\s*([^\s,，。.!！?？]{2,20})(?:不用|而不是|而不是用|别用|不用)\s*([^\s,，。.!！?？]{2,20})?",
            "convention.tool",
            "scalar",
            FactOperation::Update,
            0.85,
            1,
        ),
        // convention.naming — naming conventions
        (
            r"(?i)(?:naming\s+convention|convention\s+is|we\s+(?:name|call|use)\s+\w+\s+(?:PascalCase|camelCase|snake_case|kebab-case|UPPER_CASE|lowercase))\s*(?:for\s+)?(\w+Case|[a-z_]+-case)",
            "convention.naming",
            "scalar",
            FactOperation::Assert,
            0.80,
            1,
        ),
        // convention.naming (Chinese) — 命名规范
        (
            r"(?:(?:命名|名字|名称)(?:规范|规则|约定|习惯)|组件用|变量用|函数用|类名用)\s*(大驼峰|小驼峰|PascalCase|camelCase|蛇形|snake_case|kebab-case|下划线|连字符)",
            "convention.naming",
            "scalar",
            FactOperation::Assert,
            0.80,
            1,
        ),
        // environment.ci — CI/CD platform identification
        (
            r"(?i)(?:ci(?:/cd)?\s+(?:is|runs\s+on|uses|configured\s+in|pipeline\s+in)\s+([^\s,，。.!！?？]{2,40}))",
            "environment.ci",
            "scalar",
            FactOperation::Assert,
            0.78,
            1,
        ),
        // environment.ci (Chinese) — CI 平台
        (
            r"(?:CI(?:/CD)?(?:用的是|跑在|配置在|流水线在)\s*([^\s,，。.!！?？]{2,40}))",
            "environment.ci",
            "scalar",
            FactOperation::Assert,
            0.78,
            1,
        ),
        // environment.runtime — runtime/language version
        (
            r"(?i)(?:(?:this|my|our)\s+(?:project|repo|app|codebase)\s+(?:runs|needs|requires|uses)\s+(?:node|python|rust|go|java|ruby)\s+([^\s,，。.!！?？]{2,15}))",
            "environment.runtime",
            "scalar",
            FactOperation::Assert,
            0.82,
            1,
        ),
        // environment.runtime (Chinese) — 运行环境版本
        (
            r"(?:(?:这个|我的|我们的)(?:项目|工程|代码库)(?:需要|要求|用的是|跑在)\s*(?:Node|Python|Rust|Go|Java|Ruby)\s*([^\s,，。.!！?？]{0,15}))",
            "environment.runtime",
            "scalar",
            FactOperation::Assert,
            0.82,
            1,
        ),
    ];

    rules
        .into_iter()
        .filter_map(
            |(pattern, predicate, value_type, operation, confidence, cg)| {
                Regex::new(pattern).ok().map(|re| ExtractionRule {
                    pattern: re,
                    predicate: predicate.to_owned(),
                    value_type: value_type.to_owned(),
                    operation,
                    confidence,
                    capture_group: cg,
                })
            },
        )
        .collect()
}

/// Intermediate extraction result before conversion to FactAssertion.
struct ExtractedAssertion {
    subject: String,
    predicate: String,
    raw_value: String,
    value_type: String,
    operation: FactOperation,
    confidence: f64,
    valid_from: Option<String>,
}

/// Extract facts from conversation turns.
///
/// Strategy: LLM first (full 8-category taxonomy), regex fallback (21 predicates).
/// Only user turns are processed; assistant turns are skipped.
pub async fn extract_facts(turns: &[ConversationTurn], llm: &LlmAssist) -> Vec<FactAssertion> {
    // Try LLM extraction first (full prompt, then simplified prompt on empty result).
    if let Some(assertions) = try_llm_extract_facts(turns, llm).await {
        if !assertions.is_empty() {
            debug!(count = assertions.len(), "Using LLM-extracted facts");
            return assertions;
        }
    }
    // Fall back to deterministic regex rules.
    let regex_facts = extract_facts_regex(turns);
    debug!(
        count = regex_facts.len(),
        "Using regex-extracted facts (LLM fallback)"
    );
    regex_facts
}

/// Extract facts using LLM across the full 8-category taxonomy.
///
/// Attempts extraction with the full prompt first. If the LLM returns an empty
/// facts array, retries once with a simplified prompt that focuses on the most
/// common extraction patterns (decisions, changes, preferences).
async fn try_llm_extract_facts(
    turns: &[ConversationTurn],
    llm: &LlmAssist,
) -> Option<Vec<FactAssertion>> {
    if !llm.is_available() {
        debug!("LLM not available for fact extraction, will fall back to regex");
        return None;
    }

    let user_turns: Vec<&ConversationTurn> =
        turns.iter().filter(|t| t.role == TurnRole::User).collect();
    if user_turns.is_empty() {
        return None;
    }

    let turns_text = user_turns
        .iter()
        .map(|t| format!("[{}]: {}", t.role.as_str(), t.content_text))
        .collect::<Vec<_>>()
        .join("\n");

    // First attempt: full prompt with examples and detailed guidance.
    let prompt = build_fact_extraction_prompt(&turns_text);
    if let Some(assertions) = try_extract_with_prompt(&prompt, llm, &user_turns).await {
        if !assertions.is_empty() {
            return Some(assertions);
        }
        debug!("Full prompt returned empty facts, retrying with simplified prompt");
    }

    // Second attempt: simplified prompt (shorter, more direct, focuses on common patterns).
    let simple_prompt = build_fact_extraction_prompt_simple(&turns_text);
    if let Some(assertions) = try_extract_with_prompt(&simple_prompt, llm, &user_turns).await {
        if !assertions.is_empty() {
            debug!(
                count = assertions.len(),
                "Simplified prompt extracted facts"
            );
            return Some(assertions);
        }
        debug!("Simplified prompt also returned empty facts");
    }

    None
}

/// Core extraction logic shared by full and simplified prompt paths.
async fn try_extract_with_prompt(
    prompt: &str,
    llm: &LlmAssist,
    user_turns: &[&ConversationTurn],
) -> Option<Vec<FactAssertion>> {
    let response = match llm.complete(prompt).await {
        Some(r) => r,
        None => {
            warn!("LLM fact extraction call returned None (API failure or timeout)");
            return None;
        }
    };

    debug!(
        response_len = response.len(),
        "LLM fact extraction response received"
    );

    let parsed: serde_json::Value = match parse_llm_json(&response) {
        Some(v) => v,
        None => {
            let preview_boundary = response.floor_char_boundary(200);
            warn!(
                response_preview = %&response[..preview_boundary],
                "Failed to parse LLM fact extraction response as JSON"
            );
            return None;
        }
    };

    let facts_arr = match parsed.get("facts").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => {
            warn!("LLM fact extraction response missing 'facts' array");
            return None;
        }
    };

    let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut assertions = Vec::new();
    for fact_val in facts_arr {
        let fact: LlmFact = match serde_json::from_value(fact_val.clone()) {
            Ok(f) => f,
            Err(e) => {
                warn!(error = %e, "Failed to deserialize individual LLM fact");
                continue;
            }
        };

        // Validate: skip empty values
        if fact.value.is_empty() {
            continue;
        }

        let operation = match fact.operation.as_str() {
            "update" => FactOperation::Update,
            "retract" => FactOperation::Retract,
            _ => FactOperation::Assert,
        };

        let valid_from = Some(now_str.clone());

        assertions.push(FactAssertion {
            assertion_id: format!("ast_{}", uuid::Uuid::new_v4()),
            user_id: user_turns[0].user_id.clone(),
            subject: "user".to_owned(),
            predicate: fact.predicate.clone(),
            raw_value_text: if operation == FactOperation::Retract {
                "retracted".to_owned()
            } else {
                fact.value.clone()
            },
            value_type: fact.value_type.clone(),
            operation,
            confidence: fact.confidence.clamp(0.0, 1.0),
            valid_from,
            valid_to: None,
            source_turn_id: Some(user_turns[0].turn_id.clone()),
            source_episode_ids: None,
            extractor_version: "v2-llm".to_owned(),
        });
    }

    Some(assertions)
}

/// Deterministic regex-based extraction (fallback when LLM is unavailable).
fn extract_facts_regex(turns: &[ConversationTurn]) -> Vec<FactAssertion> {
    let rules = &*EXTRACTION_RULES;
    let mut assertions: Vec<FactAssertion> = Vec::new();

    for t in turns {
        if t.role != TurnRole::User {
            continue;
        }
        let extracted = extract_from_text(&t.content_text, rules);
        for ea in extracted {
            assertions.push(assertion_to_model(&ea, &t.user_id, &t.turn_id));
        }
    }

    assertions
}

/// Extract facts from resource text (non-turn extraction flow, regex only).
pub fn extract_facts_from_text(text: &str, user_id: &str) -> Vec<FactAssertion> {
    let rules = &*EXTRACTION_RULES;
    let extracted = extract_from_text(text, rules);
    extracted
        .iter()
        .map(|ea| assertion_to_model(ea, user_id, ""))
        .collect()
}

/// Apply all extraction rules to a single text.
fn extract_from_text(text: &str, rules: &[ExtractionRule]) -> Vec<ExtractedAssertion> {
    let mut results: Vec<ExtractedAssertion> = Vec::new();
    let now_str = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    for rule in rules {
        if let Some(caps) = rule.pattern.captures(text) {
            let value = if rule.capture_group == 0 {
                // Group 0 = full match (used for retract patterns)
                caps[0].trim().to_owned()
            } else {
                caps.get(rule.capture_group)
                    .map(|m| m.as_str().trim().to_owned())
                    .unwrap_or_default()
            };
            if value.is_empty() {
                continue;
            }

            let valid_from = Some(now_str.clone());

            results.push(ExtractedAssertion {
                subject: "user".to_owned(),
                predicate: rule.predicate.clone(),
                raw_value: value,
                value_type: rule.value_type.clone(),
                operation: rule.operation,
                confidence: rule.confidence,
                valid_from,
            });
        }
    }

    results
}

/// Convert an extracted assertion to a domain FactAssertion.
fn assertion_to_model(
    ea: &ExtractedAssertion,
    user_id: &str,
    source_turn_id: &str,
) -> FactAssertion {
    let raw_value = if ea.operation == FactOperation::Retract {
        "retracted".to_owned()
    } else {
        ea.raw_value.clone()
    };

    FactAssertion {
        assertion_id: format!("ast_{}", uuid::Uuid::new_v4()),
        user_id: user_id.to_owned(),
        subject: ea.subject.clone(),
        predicate: ea.predicate.clone(),
        raw_value_text: raw_value,
        value_type: ea.value_type.clone(),
        operation: ea.operation,
        confidence: ea.confidence,
        valid_from: ea.valid_from.clone(),
        valid_to: None,
        source_turn_id: if source_turn_id.is_empty() {
            None
        } else {
            Some(source_turn_id.to_owned())
        },
        source_episode_ids: None,
        extractor_version: "v1-rules".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mfs_test_util::env_isolated;

    fn make_turn(role: TurnRole, content: &str) -> ConversationTurn {
        ConversationTurn {
            turn_id: "turn-1".to_owned(),
            turn_seq: 1,
            session_id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            role,
            content_text: content.to_owned(),
            token_count: content.len() / 4,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        }
    }

    fn make_llm() -> LlmAssist {
        LlmAssist::from_env()
    }

    #[tokio::test]
    async fn extract_location_update_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "我现在住在东京")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "location.current_city");
        assert_eq!(facts[0].operation, FactOperation::Update);
    }

    #[tokio::test]
    async fn extract_location_update_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "I just moved to Paris")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "location.current_city");
        assert_eq!(facts[0].operation, FactOperation::Update);
    }

    #[tokio::test]
    async fn extract_city_update_does_not_also_create_country() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "I now live in Tokyo")];
        let facts = extract_facts(&turns, &make_llm()).await;

        assert!(facts.iter().any(|fact| {
            fact.predicate == "location.current_city" && fact.raw_value_text == "Tokyo"
        }));
        assert!(
            !facts
                .iter()
                .any(|fact| fact.predicate == "location.current_country"),
            "facts: {facts:?}"
        );
    }

    #[tokio::test]
    async fn extract_allergy() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "I am allergic to peanuts")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "health.allergy");
        assert_eq!(facts[0].value_type, "set");
    }

    #[tokio::test]
    async fn extract_allergy_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "我对花生过敏")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "health.allergy");
    }

    #[tokio::test]
    async fn extract_spicy_retract() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "我不再吃辣了")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty(), "Expected facts from spicy retract");
        let retract_found = facts.iter().any(|f| {
            f.predicate == "diet.spicy_preference" && f.operation == FactOperation::Retract
        });
        assert!(
            retract_found,
            "Expected diet.spicy_preference retract, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_spicy_assert() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "我喜欢吃辣")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let spicy_found = facts.iter().any(|f| f.predicate == "diet.spicy_preference");
        assert!(
            spicy_found,
            "Expected diet.spicy_preference, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_name() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "My name is Alice")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "identity.name");
        assert_eq!(facts[0].confidence, 0.90);
    }

    #[tokio::test]
    async fn skip_assistant_turns() {
        let turns = vec![make_turn(TurnRole::Assistant, "I live in New York")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(facts.is_empty());
    }

    #[tokio::test]
    async fn case_preservation() {
        let turns = vec![make_turn(TurnRole::User, "My name is Alice")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].raw_value_text, "Alice");
    }

    #[tokio::test]
    async fn health_constraint_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "我不能吃海鲜")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "health.constraint");
    }

    #[tokio::test]
    async fn communication_style_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "请用简洁一点的方式和我沟通")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "preference.communication_style");
    }

    #[tokio::test]
    async fn pronouns() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "My pronouns are she/her")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "identity.pronouns");
        assert_eq!(facts[0].confidence, 0.88);
    }

    #[tokio::test]
    async fn language_spoken() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "I speak Japanese")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "language.spoken");
    }

    #[tokio::test]
    async fn project_active_chinese() {
        let turns = vec![make_turn(TurnRole::User, "我正在开发一个web系统")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        assert_eq!(facts[0].predicate, "project.active");
    }

    #[tokio::test]
    async fn project_active_english_preserves_multi_word_project_name() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "I am working on Project Lantern")];
        let facts = extract_facts(&turns, &make_llm()).await;
        let project = facts
            .iter()
            .find(|fact| fact.predicate == "project.active")
            .expect("expected project.active fact");

        assert_eq!(project.raw_value_text, "Project Lantern");
    }

    #[tokio::test]
    async fn extract_migration_update_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "从 REST 迁移到 GraphQL")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty(), "Expected facts from Chinese migration");
        let migration = facts
            .iter()
            .find(|f| f.predicate == "entities.architecture_decision");
        assert!(
            migration.is_some(),
            "Expected entities.architecture_decision, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
        assert_eq!(migration.unwrap().raw_value_text, "GraphQL");
        assert_eq!(migration.unwrap().operation, FactOperation::Update);
    }

    #[tokio::test]
    async fn extract_migration_update_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(
            TurnRole::User,
            "We migrated from PostgreSQL to SQLite",
        )];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty(), "Expected facts from English migration");
        let migration = facts
            .iter()
            .find(|f| f.predicate == "entities.architecture_decision");
        assert!(
            migration.is_some(),
            "Expected entities.architecture_decision"
        );
        assert_eq!(migration.unwrap().raw_value_text, "SQLite");
        assert_eq!(migration.unwrap().operation, FactOperation::Update);
    }

    #[tokio::test]
    async fn extract_replace_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "用 GraphQL 替代 REST")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty(), "Expected facts from Chinese replacement");
        let replacement = facts
            .iter()
            .find(|f| f.predicate == "entities.architecture_decision");
        assert!(
            replacement.is_some(),
            "Expected entities.architecture_decision"
        );
        assert_eq!(replacement.unwrap().raw_value_text, "GraphQL");
    }

    #[tokio::test]
    async fn extract_switched_to() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "We switched to GraphQL")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(
            !facts.is_empty(),
            "Expected facts from English single-destination"
        );
        let migration = facts
            .iter()
            .find(|f| f.predicate == "entities.architecture_decision");
        assert!(
            migration.is_some(),
            "Expected entities.architecture_decision"
        );
        assert_eq!(migration.unwrap().raw_value_text, "GraphQL");
        assert_eq!(migration.unwrap().operation, FactOperation::Update);
    }

    #[tokio::test]
    async fn extract_migrated_to() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "We migrated to SQLite")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty(), "Expected facts from migrated-to pattern");
        let migration = facts
            .iter()
            .find(|f| f.predicate == "entities.architecture_decision");
        assert!(
            migration.is_some(),
            "Expected entities.architecture_decision"
        );
        assert_eq!(migration.unwrap().raw_value_text, "SQLite");
    }

    #[tokio::test]
    async fn extract_build_command_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(
            TurnRole::User,
            "build this project with cargo build",
        )];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty(), "Expected facts from build command");
        let build = facts
            .iter()
            .find(|f| f.predicate == "procedure.build_command");
        assert!(
            build.is_some(),
            "Expected procedure.build_command, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_build_command_run() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "run cargo build to build")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let build = facts
            .iter()
            .find(|f| f.predicate == "procedure.build_command");
        assert!(build.is_some(), "Expected procedure.build_command");
    }

    #[tokio::test]
    async fn extract_build_command_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "用cargo构建项目")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(
            !facts.is_empty(),
            "Expected facts from Chinese build command"
        );
        let build = facts
            .iter()
            .find(|f| f.predicate == "procedure.build_command");
        assert!(
            build.is_some(),
            "Expected procedure.build_command, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_test_command_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "run cargo test to run the tests")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let test_cmd = facts
            .iter()
            .find(|f| f.predicate == "procedure.test_command");
        assert!(
            test_cmd.is_some(),
            "Expected procedure.test_command, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_test_command_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "用cargo跑测试")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(
            !facts.is_empty(),
            "Expected facts from Chinese test command"
        );
        let test_cmd = facts
            .iter()
            .find(|f| f.predicate == "procedure.test_command");
        assert!(test_cmd.is_some(), "Expected procedure.test_command");
    }

    #[tokio::test]
    async fn extract_convention_tool_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "we use React not Angular")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let conv = facts.iter().find(|f| f.predicate == "convention.tool");
        assert!(
            conv.is_some(),
            "Expected convention.tool, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
        assert_eq!(conv.unwrap().operation, FactOperation::Update);
    }

    #[tokio::test]
    async fn extract_convention_naming_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "naming convention is snake_case")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let naming = facts.iter().find(|f| f.predicate == "convention.naming");
        assert!(
            naming.is_some(),
            "Expected convention.naming, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_convention_naming_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "命名规范大驼峰")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let naming = facts.iter().find(|f| f.predicate == "convention.naming");
        assert!(
            naming.is_some(),
            "Expected convention.naming, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_environment_ci_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "CI runs on GitHub Actions")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let ci = facts.iter().find(|f| f.predicate == "environment.ci");
        assert!(
            ci.is_some(),
            "Expected environment.ci, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn extract_environment_ci_chinese() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "CI用的是GitHub Actions")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let ci = facts.iter().find(|f| f.predicate == "environment.ci");
        assert!(ci.is_some(), "Expected environment.ci");
    }

    #[tokio::test]
    async fn extract_environment_runtime_english() {
        let _env_guard = env_isolated();
        let turns = vec![make_turn(TurnRole::User, "this project needs Node 22")];
        let facts = extract_facts(&turns, &make_llm()).await;
        assert!(!facts.is_empty());
        let runtime = facts.iter().find(|f| f.predicate == "environment.runtime");
        assert!(
            runtime.is_some(),
            "Expected environment.runtime, got: {:?}",
            facts.iter().map(|f| &f.predicate).collect::<Vec<_>>()
        );
    }
}
