use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct EpisodeTimelineQuery {
    direction: Option<String>,
    radius: Option<usize>,
}

pub(super) async fn get_episode_detail(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let episode = metadata
        .get_episode(&episode_id)
        .map_err(AppError::from_error)?
        .ok_or_else(|| {
            AppError(MfsError::NotFound {
                resource: format!("episode:{episode_id}"),
            })
        })?;

    let turns = episode_turns(&metadata, &episode).map_err(AppError::from_error)?;
    let facts: Vec<_> = metadata
        .get_active_facts(&episode.account_id, &episode.user_id)
        .map_err(AppError::from_error)?
        .into_iter()
        .filter(|fact| {
            fact.source_episode_ids_json
                .as_deref()
                .and_then(|j| serde_json::from_str::<Vec<String>>(j).ok())
                .is_some_and(|ids| ids.contains(&episode.episode_id))
        })
        .collect();

    Ok(Json(serde_json::json!({
        "episode_id": episode.episode_id,
        "session_id": episode.session_id,
        "resource_id": episode.resource_id,
        "summary": episode.summary,
        "salience_score": episode.salience_score,
        "strength_score": episode.strength_score,
        "emotional_valence": episode.emotional_valence,
        "emotional_intensity": episode.emotional_intensity,
        "context_tags_json": episode.context_tags_json,
        "created_at": episode.created_at,
        "facts": facts,
        "turns": turns,
    })))
}

pub(super) async fn get_episode_timeline(
    State(state): State<Arc<AppState>>,
    Path(episode_id): Path<String>,
    Query(query): Query<EpisodeTimelineQuery>,
) -> HandlerResult<Json<serde_json::Value>> {
    let metadata = state.metadata.clone();
    let anchor = metadata
        .get_episode(&episode_id)
        .map_err(AppError::from_error)?
        .ok_or_else(|| {
            AppError(MfsError::NotFound {
                resource: format!("episode:{episode_id}"),
            })
        })?;

    let mut episodes: Vec<_> = metadata
        .get_episodes_by_user(
            &anchor.account_id,
            &anchor.user_id,
            anchor.resource_id.as_deref(),
        )
        .map_err(AppError::from_error)?
        .into_iter()
        .filter(|episode| episode.session_id == anchor.session_id)
        .collect();
    episodes.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.episode_id.cmp(&right.episode_id))
    });

    let anchor_index = episodes
        .iter()
        .position(|episode| episode.episode_id == anchor.episode_id)
        .ok_or_else(|| {
            AppError(MfsError::NotFound {
                resource: format!("episode:{episode_id}"),
            })
        })?;

    let radius = query.radius.unwrap_or(3);
    let direction = query.direction.as_deref().unwrap_or("both");
    let start = match direction {
        "after" => anchor_index,
        "before" | "both" => anchor_index.saturating_sub(radius),
        _ => anchor_index.saturating_sub(radius),
    };
    let end_exclusive = match direction {
        "before" => (anchor_index + 1).min(episodes.len()),
        "after" | "both" => (anchor_index + radius + 1).min(episodes.len()),
        _ => (anchor_index + radius + 1).min(episodes.len()),
    };

    let window: Vec<_> = episodes[start..end_exclusive]
        .iter()
        .map(|episode| {
            serde_json::json!({
                "episode_id": episode.episode_id,
                "session_id": episode.session_id,
                "summary": episode.summary,
                "salience_score": episode.salience_score,
                "emotional_valence": episode.emotional_valence,
                "emotional_intensity": episode.emotional_intensity,
                "created_at": episode.created_at,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "anchor_episode_id": anchor.episode_id,
        "direction": direction,
        "radius": radius,
        "episodes": window,
        "count": end_exclusive.saturating_sub(start),
    })))
}

/// Citation feedback: increment recall_count for episodes and/or facts.
#[derive(Debug, Deserialize)]
pub(super) struct CiteMemoriesRequest {
    pub episode_ids: Option<Vec<String>>,
    pub fact_ids: Option<Vec<String>>,
}

pub(super) async fn cite_memories(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CiteMemoriesRequest>,
) -> HandlerResult<Json<serde_json::Value>> {
    let ep_ids = request.episode_ids.as_deref().unwrap_or(&[]);
    let fa_ids = request.fact_ids.as_deref().unwrap_or(&[]);

    if ep_ids.is_empty() && fa_ids.is_empty() {
        return Ok(Json(serde_json::json!({
            "cited_episodes": 0,
            "cited_facts": 0,
            "warning": "no IDs provided",
        })));
    }

    let metadata = state.metadata.clone();
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let account_id = state.config.account_id.clone();
    let user_id = state.config.user_id.clone();
    let mut cited_episodes = 0u32;
    let mut cited_facts = 0u32;
    let mut episode_errors: Vec<String> = Vec::new();
    let mut fact_errors: Vec<String> = Vec::new();

    for id in ep_ids {
        match metadata.increment_episode_recall(id, &now) {
            Ok(()) => {
                cited_episodes += 1;
                metadata
                    .append_access_log(id, "episode", &now, &account_id, &user_id)
                    .ok();
            }
            Err(e) => episode_errors.push(format!("{}: {}", id, e)),
        }
    }
    for id in fa_ids {
        match metadata.increment_fact_recall(id, &now) {
            Ok(()) => {
                cited_facts += 1;
                metadata
                    .append_access_log(id, "fact", &now, &account_id, &user_id)
                    .ok();
            }
            Err(e) => fact_errors.push(format!("{}: {}", id, e)),
        }
    }

    let has_errors = !episode_errors.is_empty() || !fact_errors.is_empty();
    let mut response = serde_json::json!({
        "cited_episodes": cited_episodes,
        "cited_facts": cited_facts,
    });
    if has_errors {
        response["errors"] = serde_json::json!({
            "episodes": episode_errors,
            "facts": fact_errors,
        });
    }

    Ok(Json(response))
}

fn episode_turns(
    metadata: &MetadataStore,
    episode: &mfs_metadata::EpisodeRow,
) -> Result<Vec<serde_json::Value>, MfsError> {
    let turns = metadata
        .get_turns_by_session(&episode.session_id)
        .map_err(|error| MfsError::Internal {
            message: error.to_string(),
        })?;
    let start_seq = episode.source_start_turn_id.as_deref().and_then(|turn_id| {
        turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .map(|turn| turn.turn_seq)
    });
    let end_seq = episode.source_end_turn_id.as_deref().and_then(|turn_id| {
        turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .map(|turn| turn.turn_seq)
    });

    let filtered: Vec<_> = turns
        .into_iter()
        .filter(|turn| match (start_seq, end_seq) {
            (Some(start), Some(end)) => turn.turn_seq >= start && turn.turn_seq <= end,
            _ => {
                episode
                    .source_start_turn_id
                    .as_deref()
                    .is_some_and(|turn_id| turn.turn_id == turn_id)
                    || episode
                        .source_end_turn_id
                        .as_deref()
                        .is_some_and(|turn_id| turn.turn_id == turn_id)
            }
        })
        .map(|turn| {
            serde_json::json!({
                "turn_id": turn.turn_id,
                "turn_seq": turn.turn_seq,
                "role": turn.role,
                "content": turn.content_text,
                "created_at": turn.created_at,
            })
        })
        .collect();
    Ok(filtered)
}
