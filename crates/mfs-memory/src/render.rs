//! Render module — markdown rendering for memory context injection.
//!
//! Formats the memory context response into a structured markdown string
//! with three sections: [Current Facts], [Recent Updates], [Relevant History].
//!
//! Signal灯塔 enhancement: each item includes 📍 markers that tell the agent
//! **where** to find more detail, not just the raw value. This aligns with the
//! philosophy that MemFuse provides directional signals, not encyclopedic answers.

use crate::heuristics::render_heuristics_section;
use crate::{EpisodeSummary, FactEntry, MemoryContextResponse, OverlayEntry};

/// Render a memory context response as structured markdown with signal markers.
///
/// Output format (signal灯塔 enhanced):
/// ```markdown
/// ## MemFuse Memory Context
///
/// [Current Facts]
/// - ✓ User currently lives in Tokyo 📍 [location.current_city]
/// - ~ User works as engineer 📍 [work.current_role]
///
/// [Recent Updates]
/// - user: I moved to Tokyo
///
/// [Relevant History]
/// - Auth system discussion 📍 episode=ep1
/// ```
///
/// The 📍 markers provide directional signals:
/// - Facts: 📍 [predicate] — tells agent what category of info this is
/// - Episodes: 📍 episode=id — tells agent which episode to dig into for detail
///
/// Trailing whitespace is trimmed.
pub fn render_memory_injection(resp: &MemoryContextResponse) -> String {
    let mut output = String::new();

    // Header indicating MemFuse context (signal灯塔 philosophy)
    output.push_str("## MemFuse Memory Context\n\n");

    if !resp.sections.current_facts.is_empty() {
        output.push_str("[Current Facts]\n");
        for f in &resp.sections.current_facts {
            // Confidence marker: ✓ high (>=0.8), ~ moderate (>=0.5), ? low (<0.5)
            let confidence_marker = if f.confidence >= 0.8 {
                "✓"
            } else if f.confidence >= 0.5 {
                "~"
            } else {
                "?"
            };
            output.push_str("- ");
            output.push_str(confidence_marker);
            output.push(' ');
            output.push_str(&f.display_value);
            // 📍 signal: predicate prefix tells agent what category this fact belongs to
            output.push_str(" 📍 [");
            output.push_str(&f.predicate);
            output.push(']');
            // Append staleness note if present (§10.2.2)
            if let Some(note) = &f.staleness_note {
                output.push_str(" ⚠ ");
                output.push_str(note);
            }
            output.push('\n');
        }
        output.push('\n');
    }

    if !resp.sections.recent_updates.is_empty() {
        output.push_str("[Recent Updates]\n");
        for o in &resp.sections.recent_updates {
            output.push_str("- ");
            output.push_str(o.role.as_str());
            output.push_str(": ");
            output.push_str(&o.content);
            output.push('\n');
        }
        output.push('\n');
    }

    if !resp.sections.relevant_history.is_empty() {
        output.push_str("[Relevant History]\n");
        for e in &resp.sections.relevant_history {
            output.push_str("- ");
            output.push_str(&e.summary);
            // 📍 signal: episode ID lets agent call get_observations or timeline for detail
            output.push_str(" 📍 episode=");
            output.push_str(&e.episode_id);
            // §4.3 valence direction marker: show emotional polarity of episode
            if let Some(valence) = e.emotional_valence {
                if valence >= 0.3 {
                    output.push_str(" 🟢"); // positive valence
                } else if valence <= -0.3 {
                    output.push_str(" 🔴"); // negative valence
                }
            }
            // Recall indicator: shows how often this has been referenced
            if e.recall_count > 0 {
                output.push_str(" (recalled ");
                output.push_str(&e.recall_count.to_string());
                output.push_str("×)");
            }
            output.push('\n');
        }
        output.push('\n');
    }

    // Behavioral heuristics section (T2H Phase 1)
    if !resp.sections.behavioral_heuristics.is_empty() {
        let heuristics_md = render_heuristics_section(&resp.sections.behavioral_heuristics);
        output.push_str(&heuristics_md);
        output.push_str("\n\n");
    }

    // If detail_handles exist, hint at cross-thread information
    if !resp.detail_handles.is_empty() {
        output.push_str("[Cross-Thread References]\n");
        for handle in &resp.detail_handles {
            output.push_str("- 📍 ");
            output.push_str(handle);
            output.push_str(" — use timeline or get_observations to explore\n");
        }
        output.push('\n');
    }

    // Signal灯塔 footer: remind agent of proactive interaction
    if !resp.sections.current_facts.is_empty() || !resp.sections.relevant_history.is_empty() {
        output.push_str("💡 _MemFuse provides directional signals. For full detail, use search_memories → timeline → get_observations, or read the relevant resource files._\n");
    }

    // Trim trailing whitespace
    output.trim_end().to_owned()
}

/// Render only the facts section as a compact list with signal markers.
pub fn render_facts_section(facts: &[FactEntry]) -> String {
    if facts.is_empty() {
        return String::new();
    }
    let mut output = String::from("[Current Facts]\n");
    for f in facts {
        let confidence_marker = if f.confidence >= 0.8 {
            "✓"
        } else if f.confidence >= 0.5 {
            "~"
        } else {
            "?"
        };
        output.push_str("- ");
        output.push_str(confidence_marker);
        output.push(' ');
        output.push_str(&f.display_value);
        output.push_str(" 📍 [");
        output.push_str(&f.predicate);
        output.push(']');
        // Append staleness note if present (§10.2.2)
        if let Some(note) = &f.staleness_note {
            output.push_str(" ⚠ ");
            output.push_str(note);
        }
        output.push('\n');
    }
    output.trim_end().to_owned()
}

/// Render only the overlay section as a compact list.
pub fn render_overlay_section(overlay: &[OverlayEntry]) -> String {
    if overlay.is_empty() {
        return String::new();
    }
    let mut output = String::from("[Recent Updates]\n");
    for o in overlay {
        output.push_str("- ");
        output.push_str(o.role.as_str());
        output.push_str(": ");
        output.push_str(&o.content);
        output.push('\n');
    }
    output.trim_end().to_owned()
}

/// Render only the episodes section as a compact list with signal markers.
pub fn render_episodes_section(episodes: &[EpisodeSummary]) -> String {
    if episodes.is_empty() {
        return String::new();
    }
    let mut output = String::from("[Relevant History]\n");
    for e in episodes {
        output.push_str("- ");
        output.push_str(&e.summary);
        output.push_str(" 📍 episode=");
        output.push_str(&e.episode_id);
        // §4.3 valence direction marker
        if let Some(valence) = e.emotional_valence {
            if valence >= 0.3 {
                output.push_str(" 🟢");
            } else if valence <= -0.3 {
                output.push_str(" 🔴");
            }
        }
        if e.recall_count > 0 {
            output.push_str(" (recalled ");
            output.push_str(&e.recall_count.to_string());
            output.push_str("×)");
        }
        output.push('\n');
    }
    output.trim_end().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryContextArtifacts, MemoryContextSections, TurnRole};

    fn make_response() -> MemoryContextResponse {
        MemoryContextResponse {
            sections: MemoryContextSections {
                current_facts: vec![
                    FactEntry {
                        fact_id: "f1".to_owned(),
                        predicate: "location.current_city".to_owned(),
                        display_value: "User currently lives in Tokyo".to_owned(),
                        confidence: 0.9,
                        staleness_note: None,
                        valid_from: None,
                    },
                    FactEntry {
                        fact_id: "f2".to_owned(),
                        predicate: "identity.name".to_owned(),
                        display_value: "User's name is Alice".to_owned(),
                        confidence: 0.8,
                        staleness_note: None,
                        valid_from: None,
                    },
                ],
                recent_updates: vec![
                    OverlayEntry {
                        turn_id: "t1".to_owned(),
                        role: TurnRole::User,
                        content: "I moved to Tokyo".to_owned(),
                    },
                    OverlayEntry {
                        turn_id: "t2".to_owned(),
                        role: TurnRole::Assistant,
                        content: "Got it".to_owned(),
                    },
                ],
                relevant_history: vec![
                    EpisodeSummary {
                        episode_id: "ep1".to_owned(),
                        summary: "Auth system discussion".to_owned(),
                        salience: 0.5,
                        strength: 1.0,
                        recall_count: 0,
                        emotional_valence: None,
                        emotional_intensity: None,
                        context_tags_json: None,
                        embedding_json: None,
                        created_at: None,
                    },
                    EpisodeSummary {
                        episode_id: "ep2".to_owned(),
                        summary: "Rate limiting implementation".to_owned(),
                        salience: 0.6,
                        strength: 1.2,
                        recall_count: 3,
                        emotional_valence: None,
                        emotional_intensity: None,
                        context_tags_json: None,
                        embedding_json: None,
                        created_at: None,
                    },
                ],
                behavioral_heuristics: Vec::new(),
            },
            artifacts: MemoryContextArtifacts {
                cross_thread_briefs: Vec::new(),
            },
            detail_handles: Vec::new(),
        }
    }

    #[test]
    fn render_full_context() {
        let resp = make_response();
        let rendered = render_memory_injection(&resp);
        assert!(rendered.contains("## MemFuse Memory Context"));
        assert!(rendered.contains("[Current Facts]"));
        assert!(rendered.contains("[Recent Updates]"));
        assert!(rendered.contains("[Relevant History]"));
        // Signal markers
        assert!(rendered.contains("📍 [location.current_city]"));
        assert!(rendered.contains("📍 episode=ep1"));
        assert!(rendered.contains("✓ User currently lives in Tokyo"));
        assert!(rendered.contains("recalled 3×"));
        // Footer hint
        assert!(rendered.contains("directional signals"));
        // No trailing whitespace
        assert!(!rendered.ends_with('\n'));
    }

    #[test]
    fn render_empty_response() {
        let resp = MemoryContextResponse {
            sections: MemoryContextSections {
                current_facts: Vec::new(),
                recent_updates: Vec::new(),
                relevant_history: Vec::new(),
                behavioral_heuristics: Vec::new(),
            },
            artifacts: MemoryContextArtifacts {
                cross_thread_briefs: Vec::new(),
            },
            detail_handles: Vec::new(),
        };
        let rendered = render_memory_injection(&resp);
        // Empty response should still have header but no sections
        assert!(rendered.contains("## MemFuse Memory Context"));
    }

    #[test]
    fn render_only_facts() {
        let facts = vec![FactEntry {
            fact_id: "f1".to_owned(),
            predicate: "test".to_owned(),
            display_value: "Fact 1".to_owned(),
            confidence: 0.9,
            staleness_note: None,
            valid_from: None,
        }];
        let rendered = render_facts_section(&facts);
        assert!(rendered.contains("[Current Facts]"));
        assert!(rendered.contains("✓ Fact 1"));
        assert!(rendered.contains("📍 [test]"));
    }

    #[test]
    fn render_only_overlay() {
        let overlay = vec![OverlayEntry {
            turn_id: "t1".to_owned(),
            role: TurnRole::User,
            content: "Hello".to_owned(),
        }];
        let rendered = render_overlay_section(&overlay);
        assert!(rendered.contains("[Recent Updates]"));
        assert!(rendered.contains("- user: Hello"));
    }

    #[test]
    fn render_only_episodes() {
        let episodes = vec![EpisodeSummary {
            episode_id: "ep1".to_owned(),
            summary: "Summary 1".to_owned(),
            salience: 0.5,
            strength: 1.0,
            recall_count: 2,
            emotional_valence: None,
            emotional_intensity: None,
            context_tags_json: None,
            embedding_json: None,
            created_at: None,
        }];
        let rendered = render_episodes_section(&episodes);
        assert!(rendered.contains("[Relevant History]"));
        assert!(rendered.contains("📍 episode=ep1"));
        assert!(rendered.contains("recalled 2×"));
    }

    #[test]
    fn render_partial_sections() {
        let resp = MemoryContextResponse {
            sections: MemoryContextSections {
                current_facts: vec![FactEntry {
                    fact_id: "f1".to_owned(),
                    predicate: "test".to_owned(),
                    display_value: "Fact".to_owned(),
                    confidence: 0.9,
                    staleness_note: None,
                    valid_from: None,
                }],
                recent_updates: Vec::new(),
                relevant_history: Vec::new(),
                behavioral_heuristics: Vec::new(),
            },
            artifacts: MemoryContextArtifacts {
                cross_thread_briefs: Vec::new(),
            },
            detail_handles: Vec::new(),
        };
        let rendered = render_memory_injection(&resp);
        assert!(rendered.contains("[Current Facts]"));
        assert!(!rendered.contains("[Recent Updates]"));
        assert!(!rendered.contains("[Relevant History]"));
    }

    #[test]
    fn render_detail_handles() {
        let resp = MemoryContextResponse {
            sections: MemoryContextSections {
                current_facts: Vec::new(),
                recent_updates: Vec::new(),
                relevant_history: vec![EpisodeSummary {
                    episode_id: "ep1".to_owned(),
                    summary: "test".to_owned(),
                    salience: 0.5,
                    strength: 1.0,
                    recall_count: 0,
                    emotional_valence: None,
                    emotional_intensity: None,
                    context_tags_json: None,
                    embedding_json: None,
                    created_at: None,
                }],
                behavioral_heuristics: Vec::new(),
            },
            artifacts: MemoryContextArtifacts {
                cross_thread_briefs: Vec::new(),
            },
            detail_handles: vec!["ep-ctx-1".to_owned(), "ep-ctx-2".to_owned()],
        };
        let rendered = render_memory_injection(&resp);
        assert!(rendered.contains("[Cross-Thread References]"));
        assert!(rendered.contains("ep-ctx-1"));
    }

    #[test]
    fn render_staleness_note_in_injection() {
        let resp = MemoryContextResponse {
            sections: MemoryContextSections {
                current_facts: vec![FactEntry {
                    fact_id: "f1".to_owned(),
                    predicate: "location.current_city".to_owned(),
                    display_value: "User lives in Tokyo".to_owned(),
                    confidence: 0.8,
                    staleness_note: Some(
                        "recorded 5 days ago — verify before asserting".to_owned(),
                    ),
                    valid_from: None,
                }],
                recent_updates: Vec::new(),
                relevant_history: Vec::new(),
                behavioral_heuristics: Vec::new(),
            },
            artifacts: MemoryContextArtifacts {
                cross_thread_briefs: Vec::new(),
            },
            detail_handles: Vec::new(),
        };
        let rendered = render_memory_injection(&resp);
        assert!(rendered.contains("⚠ recorded 5 days ago"));
    }
}
