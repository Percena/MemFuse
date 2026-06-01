mod hierarchical;
mod jina_reranker;
mod planner;
mod query_plan;
mod rerank;
mod rerank_ext;
mod trajectory;

use std::collections::{HashSet, VecDeque};
use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::path::Path;
#[cfg(feature = "test-support")]
use std::path::PathBuf;

use mfs_index::{
    IndexError, IndexedDocument, SearchHit, SearchIndex, SqliteFtsIndex, SqliteSemanticIndex,
};
use mfs_metadata::MetadataStore;
use mfs_semantic::{EmbeddingProvider, embedding_provider_from_env};
use mfs_types::IdentityContext;
use mfs_workspace::FsError;
#[cfg(feature = "test-support")]
use mfs_workspace::WorkspaceFs;
use serde::Serialize;

use crate::hierarchical::{RankedLayeredHit, collapse_layered_hits, rank_layered_hits};
pub use jina_reranker::JinaReranker;
pub use planner::{QueryPlanner, TypedQuery};
pub use query_plan::{PlannedQuery, QueryPlan, QueryPlanMode};
use rerank::{DeterministicReranker, Reranker};
pub use rerank_ext::{RerankScore, RerankerExt};
pub use trajectory::{RetrievalStep, RetrievalTrajectory};

/// Select a reranker provider based on `MEMFUSE_RERANK_PROVIDER` env var.
/// Priority: jina (if env=jina and Jina API key configured) → deterministic (default).
pub fn reranker_from_env() -> Box<dyn Reranker> {
    match rerank_provider_name_from_env().as_str() {
        "jina" => JinaReranker::from_env()
            .map(|p| Box::new(p) as Box<dyn Reranker>)
            .unwrap_or_else(|| Box::new(DeterministicReranker)),
        _ => Box::new(DeterministicReranker),
    }
}

/// Read `MEMFUSE_RERANK_PROVIDER` env var to determine rerank provider.
/// Falls back to "jina" if `MEMFUSE_JINA_API_KEY` is set (auto-detect),
/// otherwise defaults to "deterministic".
pub fn rerank_provider_name_from_env() -> String {
    env_value(&["MEMFUSE_RERANK_PROVIDER"])
        .unwrap_or_else(|| {
            // Auto-detect: if Jina API key is available, use Jina reranker
            if env_value(&["MEMFUSE_JINA_API_KEY"]).is_some() {
                "jina".to_owned()
            } else {
                "deterministic".to_owned()
            }
        })
        .to_ascii_lowercase()
}

fn env_value(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    })
}

#[derive(Debug, Clone, Copy)]
pub struct RetrievalSettings {
    pub level_weights: [u16; 3],
    pub max_drill_depth: usize,
    pub phase_one_limit: usize,
    pub phase_two_limit: usize,
    pub max_drill_targets_per_level: usize,
    pub max_total_resources: usize,
    pub enable_rerank: bool,
    pub enable_llm_planner: bool,
}

impl Default for RetrievalSettings {
    fn default() -> Self {
        Self {
            level_weights: [0, 100, 200],
            max_drill_depth: 3,
            phase_one_limit: 10,
            phase_two_limit: 10,
            max_drill_targets_per_level: 5,
            max_total_resources: 16,
            enable_rerank: false,
            enable_llm_planner: false,
        }
    }
}

pub struct RetrievalEngine {
    planner: QueryPlanner,
    lexical_index: SqliteFtsIndex,
    semantic_index: Option<SqliteSemanticIndex>,
    semantic_embedder: Option<Box<dyn EmbeddingProvider>>,
    metadata: Option<MetadataStore>,
    identity: Option<IdentityContext>,
    settings: RetrievalSettings,
}

struct SearchBatch {
    hits: Vec<SearchHit>,
    plane: &'static str,
}

impl RetrievalEngine {
    #[cfg(feature = "test-support")]
    pub async fn for_tests() -> Result<Self, RetrievalError> {
        Self::for_tests_with_settings(RetrievalSettings::default()).await
    }

    #[cfg(feature = "test-support")]
    pub async fn for_tests_with_settings(
        settings: RetrievalSettings,
    ) -> Result<Self, RetrievalError> {
        let fixture_root = test_fixture_path();
        let identity = IdentityContext::new("acme", "alice", "coding-agent");
        let fs = WorkspaceFs::from_fixture(
            identity.account_id(),
            identity.user_id(),
            identity.agent_id(),
            fixture_root.to_str().expect("fixture path utf-8"),
        )
        .await
        .map_err(RetrievalError::Fs)?;
        Self::from_workspace_with_settings(
            fs.workspace_root(),
            &identity,
            fs.projection_root(),
            fs.projection_uri(),
            settings,
        )
        .await
    }

    pub async fn from_projection(
        root: &Path,
        projection_uri: &str,
    ) -> Result<Self, RetrievalError> {
        Self::from_projection_with_settings(root, projection_uri, RetrievalSettings::default())
            .await
    }

    pub async fn from_projection_with_settings(
        root: &Path,
        projection_uri: &str,
        settings: RetrievalSettings,
    ) -> Result<Self, RetrievalError> {
        let index = SqliteFtsIndex::open_in_memory().map_err(RetrievalError::Index)?;
        seed_index_root(&index, root, projection_uri, "resource").await?;

        Ok(Self {
            planner: QueryPlanner::new(settings.enable_llm_planner),
            lexical_index: index,
            semantic_index: None,
            semantic_embedder: None,
            metadata: None,
            identity: None,
            settings,
        })
    }

    pub async fn from_workspace(
        workspace_root: &Path,
        identity: &IdentityContext,
        projection_root: &Path,
        projection_uri: &str,
    ) -> Result<Self, RetrievalError> {
        Self::from_workspace_with_settings(
            workspace_root,
            identity,
            projection_root,
            projection_uri,
            RetrievalSettings::default(),
        )
        .await
    }

    pub async fn from_workspace_with_settings(
        workspace_root: &Path,
        identity: &IdentityContext,
        projection_root: &Path,
        projection_uri: &str,
        settings: RetrievalSettings,
    ) -> Result<Self, RetrievalError> {
        let index = SqliteFtsIndex::open_in_memory().map_err(RetrievalError::Index)?;
        seed_index_root(&index, projection_root, projection_uri, "resource").await?;
        seed_optional_index_root(
            &index,
            &workspace_root
                .join("tenants")
                .join(identity.account_id())
                .join(identity.user_id())
                .join("user")
                .join("memories"),
            "mfs://user/memories",
            "memory",
        )
        .await?;
        seed_optional_index_root(
            &index,
            &workspace_root
                .join("tenants")
                .join(identity.account_id())
                .join(identity.user_id())
                .join("agent")
                .join(identity.agent_space_name())
                .join("memories"),
            "mfs://agent/memories",
            "memory",
        )
        .await?;
        seed_optional_index_root(
            &index,
            &workspace_root
                .join("tenants")
                .join(identity.account_id())
                .join(identity.user_id())
                .join("agent")
                .join(identity.agent_space_name())
                .join("skills"),
            "mfs://agent/skills",
            "skill",
        )
        .await?;

        let semantic_index_path = workspace_root.join("_system").join("semantic.sqlite");
        let semantic_index = if semantic_index_path.exists() {
            Some(
                SqliteSemanticIndex::open_at(&semantic_index_path)
                    .map_err(RetrievalError::Index)?,
            )
        } else {
            None
        };
        let semantic_embedder = match &semantic_index {
            Some(index) => Some(embedding_provider_from_env(
                index
                    .embedding_dimension()
                    .map_err(RetrievalError::Index)?
                    .unwrap_or(8),
            )),
            None => None,
        };
        let metadata_path = workspace_root.join("_system").join("metadata.sqlite");
        let metadata = if metadata_path.exists() {
            Some(MetadataStore::open_at(&metadata_path, false).map_err(RetrievalError::Metadata)?)
        } else {
            None
        };

        Ok(Self {
            planner: QueryPlanner::new(settings.enable_llm_planner),
            lexical_index: index,
            semantic_index,
            semantic_embedder,
            metadata,
            identity: Some(identity.clone()),
            settings,
        })
    }

    pub async fn find(
        &self,
        query: &str,
        target: Option<&str>,
    ) -> Result<SearchResult, RetrievalError> {
        self.run(query, target, None).await
    }

    pub async fn search(
        &self,
        query: &str,
        target: Option<&str>,
        session: Option<&str>,
    ) -> Result<SearchResult, RetrievalError> {
        self.run(query, target, session).await
    }

    pub async fn grep(
        &self,
        query: &str,
        target: Option<&str>,
        limit: Option<usize>,
    ) -> Result<SearchResult, RetrievalError> {
        let mut trajectory = RetrievalTrajectory::default();
        trajectory.record("grep_query", query);
        if let Some(target) = target {
            trajectory.record("target", target);
        }

        let effective_limit = limit.unwrap_or(10);
        let hits = self
            .lexical_index
            .grep_literal(
                query,
                target,
                Some(&[2, 1, 0]),
                Some(&["resource"]),
                effective_limit,
            )
            .map_err(RetrievalError::Index)?;
        let resources = hits
            .into_iter()
            .map(|hit| self.context_from_hit(hit, "literal grep match"))
            .collect();

        Ok(SearchResult {
            query_plan: self.planner.plan_find(query).await,
            typed_queries: vec![TypedQuery {
                query: query.to_owned(),
                context_type: "resource".to_owned(),
            }],
            trajectory,
            resources,
            memories: Vec::new(),
            skills: Vec::new(),
        })
    }

    async fn run(
        &self,
        query: &str,
        target: Option<&str>,
        session: Option<&str>,
    ) -> Result<SearchResult, RetrievalError> {
        let query_plan = if session.is_some() {
            self.planner.plan_search(query, session).await
        } else {
            self.planner.plan_find(query).await
        };
        let mut typed_queries: Vec<TypedQuery> = query_plan
            .typed_queries
            .iter()
            .map(|typed_query| TypedQuery {
                query: typed_query.query.clone(),
                context_type: typed_query.context_type.clone(),
            })
            .collect();
        typed_queries
            .sort_by_key(|typed_query| context_priority(target, &typed_query.context_type));
        let mut trajectory = RetrievalTrajectory::default();
        trajectory.record("query", query);
        if let Some(target) = target {
            trajectory.record("target", target);
        }
        trajectory.record("query_plan_mode", query_plan.mode.as_str());
        if let Some(skip_reason) = &query_plan.skip_reason {
            trajectory.record("query_plan_skip_reason", skip_reason);
        }

        let mut resources = Vec::new();
        let mut memories = Vec::new();
        let mut skills = Vec::new();

        let mut budget_exhausted = false;

        if typed_queries.is_empty() {
            return Ok(SearchResult {
                query_plan,
                typed_queries,
                trajectory,
                resources,
                memories,
                skills,
            });
        }

        'typed_queries: for typed_query in &typed_queries {
            trajectory.record(
                "typed_query",
                &format!("{}:{}", typed_query.context_type, typed_query.query),
            );
            let target_scope = scope_for_context(target, &typed_query.context_type);
            let root_scope = target_scope.unwrap_or("");
            let context_types = [typed_query.context_type.as_str()];
            let phase_one = self
                .search_hits(
                    &typed_query.query,
                    target_scope,
                    Some(&[0, 1]),
                    Some(&context_types),
                    self.settings.phase_one_limit,
                )
                .await?;
            trajectory.record("retrieval_plane", phase_one.plane);
            let ranked_directories = collapse_layered_hits(
                phase_one
                    .hits
                    .into_iter()
                    .filter(|hit| {
                        root_scope.is_empty()
                            || hit.uri == root_scope
                            || is_direct_child_scope(root_scope, &hit.uri)
                    })
                    .collect(),
                self.settings.level_weights,
            );
            let ranked_directories = self
                .rerank_ranked_hits(query, ranked_directories, &mut trajectory)
                .await;
            trajectory.record("phase_one_hits", &ranked_directories.len().to_string());

            let mut pending_directories = VecDeque::new();
            let mut visited_directories = HashSet::new();

            for hit in &ranked_directories {
                push_ranked_context_if_missing(
                    &typed_query.context_type,
                    &mut resources,
                    &mut memories,
                    &mut skills,
                    hit.clone(),
                    phase_one.plane,
                    if self.settings.enable_rerank {
                        "rerank promoted directory hit"
                    } else {
                        "directory retrieval hit"
                    },
                );
                if total_len(&resources, &memories, &skills) >= self.settings.max_total_resources {
                    record_convergence_stop(&mut trajectory, "resource_budget");
                    break 'typed_queries;
                }
                if visited_directories.insert(hit.hit.uri.clone()) {
                    pending_directories.push_back((hit.hit.uri.clone(), 1));
                }
            }

            let mut leaf_hits: Vec<(SearchHit, &'static str)> = Vec::new();

            if pending_directories.is_empty() {
                let direct_leaf_hits = self
                    .search_hits(
                        &typed_query.query,
                        target_scope,
                        Some(&[2]),
                        Some(&context_types),
                        self.settings.phase_two_limit,
                    )
                    .await?;
                leaf_hits = direct_leaf_hits
                    .hits
                    .into_iter()
                    .map(|hit| (hit, direct_leaf_hits.plane))
                    .collect();
            } else {
                while let Some((drill_target, depth)) = pending_directories.pop_front() {
                    record_unique_step(&mut trajectory, "drill_target", &drill_target);
                    let nested_directory_hits = self
                        .search_hits(
                            &typed_query.query,
                            Some(&drill_target),
                            Some(&[0, 1]),
                            Some(&context_types),
                            self.settings.phase_one_limit,
                        )
                        .await?;
                    let ranked_nested_directories = collapse_layered_hits(
                        nested_directory_hits
                            .hits
                            .into_iter()
                            .filter(|hit| hit.uri != drill_target)
                            .filter(|hit| is_direct_child_scope(&drill_target, &hit.uri))
                            .collect(),
                        self.settings.level_weights,
                    );
                    let nested_plane = nested_directory_hits.plane;
                    let ranked_nested_directories = self
                        .rerank_ranked_hits(query, ranked_nested_directories, &mut trajectory)
                        .await;

                    for nested_hit in ranked_nested_directories
                        .into_iter()
                        .take(self.settings.max_drill_targets_per_level)
                    {
                        push_ranked_context_if_missing(
                            &typed_query.context_type,
                            &mut resources,
                            &mut memories,
                            &mut skills,
                            nested_hit.clone(),
                            nested_plane,
                            if self.settings.enable_rerank {
                                "rerank promoted directory hit"
                            } else {
                                "directory retrieval hit"
                            },
                        );
                        if total_len(&resources, &memories, &skills)
                            >= self.settings.max_total_resources
                        {
                            record_convergence_stop(&mut trajectory, "resource_budget");
                            budget_exhausted = true;
                            break;
                        }
                        if depth < self.settings.max_drill_depth
                            && visited_directories.insert(nested_hit.hit.uri.clone())
                        {
                            pending_directories.push_back((nested_hit.hit.uri.clone(), depth + 1));
                        }
                        let deeper_leaf_hits = self
                            .search_hits(
                                &typed_query.query,
                                Some(&nested_hit.hit.uri),
                                Some(&[2]),
                                Some(&context_types),
                                self.settings.phase_two_limit,
                            )
                            .await?;
                        leaf_hits.extend(
                            deeper_leaf_hits
                                .hits
                                .into_iter()
                                .map(|hit| (hit, deeper_leaf_hits.plane)),
                        );
                    }

                    if budget_exhausted {
                        break;
                    }

                    let scoped_hits = self
                        .search_hits(
                            &typed_query.query,
                            Some(&drill_target),
                            Some(&[2]),
                            Some(&context_types),
                            self.settings.phase_two_limit,
                        )
                        .await?;
                    leaf_hits.extend(
                        scoped_hits
                            .hits
                            .into_iter()
                            .map(|hit| (hit, scoped_hits.plane)),
                    );
                }
            }

            let mut leaf_planes = std::collections::HashMap::new();
            let ranked_leaves = rank_layered_hits(
                leaf_hits
                    .into_iter()
                    .map(|(hit, plane)| {
                        leaf_planes.insert(hit.uri.clone(), plane);
                        hit
                    })
                    .collect(),
                self.settings.level_weights,
            );
            let ranked_leaves = self
                .rerank_search_hits(query, ranked_leaves, &mut trajectory)
                .await;
            trajectory.record("phase_two_hits", &ranked_leaves.len().to_string());

            for hit in ranked_leaves {
                push_context_if_missing(
                    &typed_query.context_type,
                    &mut resources,
                    &mut memories,
                    &mut skills,
                    leaf_planes.get(&hit.uri).copied().unwrap_or("unknown"),
                    hit,
                    if self.settings.enable_rerank {
                        "rerank promoted leaf hit"
                    } else {
                        "leaf retrieval hit"
                    },
                );
                if total_len(&resources, &memories, &skills) >= self.settings.max_total_resources {
                    record_convergence_stop(&mut trajectory, "resource_budget");
                    budget_exhausted = true;
                    break;
                }
            }

            if budget_exhausted {
                break;
            }
        }

        self.enrich_with_relations(&mut resources, &mut memories, &mut skills, &mut trajectory);

        resources.sort_by(|left, right| {
            resource_sort_key(left, self.settings.level_weights)
                .cmp(&resource_sort_key(right, self.settings.level_weights))
                .then_with(|| left.score.total_cmp(&right.score))
                .then_with(|| left.uri.cmp(&right.uri))
        });
        memories.sort_by(|left, right| {
            resource_sort_key(left, self.settings.level_weights)
                .cmp(&resource_sort_key(right, self.settings.level_weights))
                .then_with(|| left.score.total_cmp(&right.score))
                .then_with(|| left.uri.cmp(&right.uri))
        });
        skills.sort_by(|left, right| {
            resource_sort_key(left, self.settings.level_weights)
                .cmp(&resource_sort_key(right, self.settings.level_weights))
                .then_with(|| left.score.total_cmp(&right.score))
                .then_with(|| left.uri.cmp(&right.uri))
        });

        if self.settings.enable_rerank {
            mark_rerank_reason(&mut resources);
            mark_rerank_reason(&mut memories);
            mark_rerank_reason(&mut skills);
        }

        self.attach_provenance(&mut resources);
        self.attach_provenance(&mut memories);
        self.attach_provenance(&mut skills);

        Ok(SearchResult {
            query_plan,
            typed_queries,
            trajectory,
            resources,
            memories,
            skills,
        })
    }

    async fn search_hits(
        &self,
        query: &str,
        target_prefix: Option<&str>,
        levels: Option<&[u8]>,
        context_types: Option<&[&str]>,
        limit: usize,
    ) -> Result<SearchBatch, RetrievalError> {
        if self.should_use_semantic(context_types) {
            let embedder = self.semantic_embedder.as_ref().expect("semantic embedder");
            let query_embedding = embedder.embed_text(query).await;
            let projection_filters = self.semantic_projection_filters(context_types);
            let projection_refs = projection_filters
                .as_ref()
                .map(|filters| filters.iter().map(String::as_str).collect::<Vec<_>>());
            let hits = self
                .semantic_index
                .as_ref()
                .expect("semantic index")
                .semantic_search(
                    &query_embedding,
                    query,
                    projection_refs.as_deref(),
                    target_prefix,
                    levels,
                    context_types,
                    limit,
                )
                .map_err(RetrievalError::Index)?;
            let hits = hits
                .into_iter()
                .map(|mut hit| {
                    hit.score = -hit.score;
                    hit
                })
                .collect::<Vec<_>>();
            if !hits.is_empty() {
                return Ok(SearchBatch {
                    hits,
                    plane: "semantic",
                });
            }
            let lexical_hits = self
                .lexical_index
                .search_with_filters(query, target_prefix, levels, context_types, limit)
                .map_err(RetrievalError::Index)?;
            return Ok(SearchBatch {
                hits: lexical_hits,
                plane: "semantic->lexical",
            });
        }

        let hits = self
            .lexical_index
            .search_with_filters(query, target_prefix, levels, context_types, limit)
            .map_err(RetrievalError::Index)?;
        Ok(SearchBatch {
            hits,
            plane: "lexical",
        })
    }

    fn should_use_semantic(&self, context_types: Option<&[&str]>) -> bool {
        self.semantic_index.is_some()
            && self.semantic_embedder.is_some()
            && context_types.map(|types| !types.is_empty()).unwrap_or(true)
    }

    fn semantic_projection_filters(&self, context_types: Option<&[&str]>) -> Option<Vec<String>> {
        let identity = self.identity.as_ref()?;
        let context_types = context_types?;
        let mut filters = Vec::new();

        for context_type in context_types {
            match *context_type {
                "resource" => filters.push(format!(
                    "tenant:{}:{}:resources",
                    identity.account_id(),
                    identity.user_id()
                )),
                "memory" => {
                    filters.push(format!(
                        "tenant:{}:{}:user",
                        identity.account_id(),
                        identity.user_id()
                    ));
                    filters.push(format!(
                        "tenant:{}:{}:agent:{}",
                        identity.account_id(),
                        identity.user_id(),
                        identity.agent_space_name()
                    ));
                }
                "skill" => filters.push(format!(
                    "tenant:{}:{}:agent:{}",
                    identity.account_id(),
                    identity.user_id(),
                    identity.agent_space_name()
                )),
                _ => {}
            }
        }

        if filters.is_empty() {
            None
        } else {
            filters.sort();
            filters.dedup();
            Some(filters)
        }
    }

    fn context_from_hit(&self, hit: SearchHit, match_reason: &str) -> MatchedContext {
        let provenance = self.hit_provenance(&hit);
        MatchedContext {
            matched_levels: vec![hit.level],
            uri: hit.uri,
            context_type: hit.context_type,
            level: hit.level,
            score: hit.score,
            excerpt: hit.excerpt,
            retrieval_plane: "lexical_grep".to_owned(),
            match_reason: match_reason.to_owned(),
            provenance,
        }
    }

    async fn rerank_ranked_hits(
        &self,
        query: &str,
        hits: Vec<RankedLayeredHit>,
        trajectory: &mut RetrievalTrajectory,
    ) -> Vec<RankedLayeredHit> {
        if !self.settings.enable_rerank {
            return hits;
        }
        trajectory.record("rerank", &format!("ranked_hits={}", hits.len()));
        if hits.len() < 2 {
            return hits;
        }
        DeterministicReranker.rerank_ranked_hits(query, hits).await
    }

    async fn rerank_search_hits(
        &self,
        query: &str,
        hits: Vec<SearchHit>,
        trajectory: &mut RetrievalTrajectory,
    ) -> Vec<SearchHit> {
        if !self.settings.enable_rerank {
            return hits;
        }
        trajectory.record("rerank", &format!("leaf_hits={}", hits.len()));
        if hits.len() < 2 {
            return hits;
        }
        DeterministicReranker.rerank_search_hits(query, hits).await
    }

    fn hit_provenance(&self, hit: &SearchHit) -> Option<MatchedProvenance> {
        let projection_view_id = self.projection_view_id_for_uri(&hit.uri)?;
        let entry = self
            .metadata
            .as_ref()?
            .get_path_entry(&projection_view_id, &hit.uri)
            .ok()??;
        let audit_events = self
            .metadata
            .as_ref()?
            .list_audit_for_subject(
                self.identity.as_ref()?.account_id(),
                self.identity.as_ref()?.user_id(),
                &hit.uri,
                5,
            )
            .ok()?
            .into_iter()
            .map(|record| MatchedAuditEvent {
                event_type: record.event_type,
                recorded_at: record.recorded_at,
            })
            .collect();
        Some(MatchedProvenance {
            projection_view_id,
            workspace_path: entry.workspace_path,
            content_kind: entry.content_kind,
            language: entry.language,
            resource_id: entry
                .repo_root_uri
                .as_deref()
                .and_then(|root_uri| {
                    self.metadata
                        .as_ref()?
                        .get_resource_source_by_root_uri(
                            self.identity.as_ref()?.account_id(),
                            self.identity.as_ref()?.user_id(),
                            root_uri,
                        )
                        .ok()?
                })
                .map(|resource| resource.resource_id),
            source_kind: entry.source_kind,
            source_identifier: entry.source_identifier,
            source_snapshot_id: entry.source_snapshot_id,
            audit_events,
        })
    }

    fn attach_provenance(&self, contexts: &mut [MatchedContext]) {
        for context in contexts {
            if context.provenance.is_some() {
                continue;
            }
            let hit = SearchHit {
                uri: context.uri.clone(),
                context_type: context.context_type.clone(),
                level: context.level,
                score: context.score,
                excerpt: context.excerpt.clone(),
            };
            context.provenance = self.hit_provenance(&hit);
        }
    }

    fn enrich_with_relations(
        &self,
        resources: &mut Vec<MatchedContext>,
        memories: &mut Vec<MatchedContext>,
        skills: &mut Vec<MatchedContext>,
        trajectory: &mut RetrievalTrajectory,
    ) {
        let (Some(metadata), Some(identity)) = (self.metadata.as_ref(), self.identity.as_ref())
        else {
            return;
        };

        let seed_uris = resources
            .iter()
            .chain(memories.iter())
            .chain(skills.iter())
            .map(|context| context.uri.clone())
            .collect::<Vec<_>>();
        let mut known_uris = seed_uris.iter().cloned().collect::<HashSet<_>>();
        let mut added = 0usize;

        for uri in seed_uris {
            let Ok(relations) =
                metadata.list_relations(identity.account_id(), identity.user_id(), &uri, 8)
            else {
                continue;
            };
            for relation in relations {
                let peer_uri = if relation.from_uri == uri {
                    relation.to_uri
                } else {
                    relation.from_uri
                };
                if !known_uris.insert(peer_uri.clone()) {
                    continue;
                }

                let context_type = relation_context_type(&peer_uri);
                let level = relation_level_for_uri(&peer_uri);
                let excerpt = self.relation_excerpt(&peer_uri);
                let context = MatchedContext {
                    matched_levels: vec![level],
                    uri: peer_uri.clone(),
                    context_type: context_type.to_owned(),
                    level,
                    score: 10_000.0,
                    excerpt,
                    retrieval_plane: "relation".to_owned(),
                    match_reason: format!("related via {} from {}", relation.relation_type, uri),
                    provenance: None,
                };
                bucket_for_context(context_type, resources, memories, skills).push(context);
                added += 1;
            }
        }

        if added > 0 {
            trajectory.record("relation_enrichment", &added.to_string());
        }
    }

    fn relation_excerpt(&self, uri: &str) -> String {
        let Some(projection_view_id) = self.projection_view_id_for_uri(uri) else {
            return String::new();
        };
        let Some(metadata) = self.metadata.as_ref() else {
            return String::new();
        };
        let Ok(Some(entry)) = metadata.get_path_entry(&projection_view_id, uri) else {
            return String::new();
        };
        std::fs::read_to_string(&entry.workspace_path)
            .ok()
            .map(|body| excerpt(&body))
            .unwrap_or_default()
    }

    fn projection_view_id_for_uri(&self, uri: &str) -> Option<String> {
        let identity = self.identity.as_ref()?;
        if uri.starts_with("mfs://resources/") {
            return Some(format!(
                "tenant:{}:{}:resources",
                identity.account_id(),
                identity.user_id()
            ));
        }
        if uri.starts_with("mfs://user/") {
            return Some(format!(
                "tenant:{}:{}:user",
                identity.account_id(),
                identity.user_id()
            ));
        }
        if uri.starts_with("mfs://agent/") {
            return Some(format!(
                "tenant:{}:{}:agent:{}",
                identity.account_id(),
                identity.user_id(),
                identity.agent_space_name()
            ));
        }
        None
    }
}

#[cfg(feature = "test-support")]
fn test_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/localfs_docs")
}

fn push_context_if_missing(
    context_type: &str,
    resources: &mut Vec<MatchedContext>,
    memories: &mut Vec<MatchedContext>,
    skills: &mut Vec<MatchedContext>,
    retrieval_plane: &str,
    hit: SearchHit,
    match_reason: &str,
) {
    let bucket = bucket_for_context(context_type, resources, memories, skills);
    if !bucket
        .iter()
        .any(|existing: &MatchedContext| existing.uri == hit.uri)
    {
        let mut context = context_from_search_hit(hit);
        context.retrieval_plane = retrieval_plane.to_owned();
        context.match_reason = match_reason.to_owned();
        bucket.push(context);
    }
}

fn push_ranked_context_if_missing(
    context_type: &str,
    resources: &mut Vec<MatchedContext>,
    memories: &mut Vec<MatchedContext>,
    skills: &mut Vec<MatchedContext>,
    hit: RankedLayeredHit,
    retrieval_plane: &str,
    match_reason: &str,
) {
    let bucket = bucket_for_context(context_type, resources, memories, skills);
    if !bucket
        .iter()
        .any(|existing: &MatchedContext| existing.uri == hit.hit.uri)
    {
        let mut context = context_from_ranked_search_hit(hit);
        context.retrieval_plane = retrieval_plane.to_owned();
        context.match_reason = match_reason.to_owned();
        bucket.push(context);
    }
}

fn bucket_for_context<'a>(
    context_type: &str,
    resources: &'a mut Vec<MatchedContext>,
    memories: &'a mut Vec<MatchedContext>,
    skills: &'a mut Vec<MatchedContext>,
) -> &'a mut Vec<MatchedContext> {
    match context_type {
        "memory" => memories,
        "skill" => skills,
        _ => resources,
    }
}

fn total_len(
    resources: &[MatchedContext],
    memories: &[MatchedContext],
    skills: &[MatchedContext],
) -> usize {
    resources.len() + memories.len() + skills.len()
}

fn context_priority(target: Option<&str>, context_type: &str) -> u8 {
    match (target, context_type) {
        (Some(target), "resource") if target.starts_with("mfs://resources/") => 0,
        (Some(target), "memory")
            if target.starts_with("mfs://user/") || target.starts_with("mfs://agent/memories") =>
        {
            0
        }
        (Some(target), "skill") if target.starts_with("mfs://agent/skills") => 0,
        (_, "resource") => 1,
        (_, "memory") => 2,
        (_, "skill") => 3,
        _ => 4,
    }
}

fn scope_for_context<'a>(target: Option<&'a str>, context_type: &str) -> Option<&'a str> {
    match (context_type, target) {
        ("resource", target) => target,
        ("memory", Some(target))
            if target.starts_with("mfs://user/") || target.starts_with("mfs://agent/memories") =>
        {
            Some(target)
        }
        ("skill", Some(target)) if target.starts_with("mfs://agent/skills") => Some(target),
        _ => None,
    }
}

fn resource_sort_key(matched: &MatchedContext, level_weights: [u16; 3]) -> (u16, usize, u16) {
    (
        level_priority(matched.level, level_weights),
        usize::MAX - matched.matched_levels.len(),
        matched
            .matched_levels
            .iter()
            .map(|level| level_priority(*level, level_weights))
            .sum(),
    )
}

fn relation_context_type(uri: &str) -> &'static str {
    if uri.starts_with("mfs://agent/skills") {
        "skill"
    } else if uri.starts_with("mfs://user/") || uri.starts_with("mfs://agent/memories") {
        "memory"
    } else {
        "resource"
    }
}

fn relation_level_for_uri(uri: &str) -> u8 {
    if uri.ends_with('/') || !uri.rsplit('/').next().unwrap_or_default().contains('.') {
        1
    } else {
        2
    }
}

fn excerpt(body: &str) -> String {
    let collapsed = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= 120 {
        collapsed
    } else {
        format!("{}…", &collapsed[..120])
    }
}

fn level_priority(level: u8, level_weights: [u16; 3]) -> u16 {
    match level {
        0 => level_weights[0],
        1 => level_weights[1],
        _ => level_weights[2],
    }
}

fn record_convergence_stop(trajectory: &mut RetrievalTrajectory, reason: &str) {
    record_unique_step(trajectory, "convergence_stop", reason);
}

fn record_unique_step(trajectory: &mut RetrievalTrajectory, stage: &str, detail: &str) {
    if trajectory
        .steps
        .iter()
        .any(|step| step.stage == stage && step.detail == detail)
    {
        return;
    }

    trajectory.record(stage, detail);
}

fn mark_rerank_reason(contexts: &mut [MatchedContext]) {
    for context in contexts {
        if !context.match_reason.contains("rerank") {
            context.match_reason = format!("rerank reviewed {}", context.match_reason);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedContext {
    pub uri: String,
    pub context_type: String,
    pub level: u8,
    pub matched_levels: Vec<u8>,
    pub score: f64,
    pub excerpt: String,
    pub retrieval_plane: String,
    pub match_reason: String,
    pub provenance: Option<MatchedProvenance>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedProvenance {
    pub projection_view_id: String,
    pub workspace_path: String,
    pub content_kind: Option<String>,
    pub language: Option<String>,
    pub resource_id: Option<String>,
    pub source_kind: Option<String>,
    pub source_identifier: Option<String>,
    pub source_snapshot_id: Option<String>,
    pub audit_events: Vec<MatchedAuditEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MatchedAuditEvent {
    pub event_type: String,
    pub recorded_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    pub query_plan: QueryPlan,
    pub typed_queries: Vec<TypedQuery>,
    pub trajectory: RetrievalTrajectory,
    pub resources: Vec<MatchedContext>,
    pub memories: Vec<MatchedContext>,
    pub skills: Vec<MatchedContext>,
}

#[derive(Debug)]
pub enum RetrievalError {
    Fs(FsError),
    Io(std::io::Error),
    Index(IndexError),
    Metadata(rusqlite::Error),
}

impl Display for RetrievalError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fs(source) => write!(f, "workspace error: {source}"),
            Self::Io(source) => write!(f, "io error: {source}"),
            Self::Index(source) => write!(f, "index error: {source}"),
            Self::Metadata(source) => write!(f, "metadata error: {source}"),
        }
    }
}

impl Error for RetrievalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Fs(source) => Some(source),
            Self::Io(source) => Some(source),
            Self::Index(source) => Some(source),
            Self::Metadata(source) => Some(source),
        }
    }
}

fn context_from_search_hit(hit: SearchHit) -> MatchedContext {
    MatchedContext {
        matched_levels: vec![hit.level],
        uri: hit.uri,
        context_type: hit.context_type,
        level: hit.level,
        score: hit.score,
        excerpt: hit.excerpt,
        retrieval_plane: "unknown".to_owned(),
        match_reason: "retrieval hit".to_owned(),
        provenance: None,
    }
}

fn context_from_ranked_search_hit(hit: RankedLayeredHit) -> MatchedContext {
    MatchedContext {
        matched_levels: hit.matched_levels,
        uri: hit.hit.uri,
        context_type: hit.hit.context_type,
        level: hit.hit.level,
        score: hit.hit.score,
        excerpt: hit.hit.excerpt,
        retrieval_plane: "unknown".to_owned(),
        match_reason: "retrieval hit".to_owned(),
        provenance: None,
    }
}

fn is_direct_child_scope(parent: &str, candidate: &str) -> bool {
    let parent = parent.trim_end_matches('/');
    let candidate = candidate.trim_end_matches('/');

    if parent.is_empty() {
        return false;
    }
    if candidate == parent || !candidate.starts_with(parent) {
        return false;
    }

    let suffix = candidate
        .strip_prefix(parent)
        .unwrap_or(candidate)
        .trim_start_matches('/');
    !suffix.is_empty() && !suffix.contains('/')
}

async fn seed_optional_index_root(
    index: &SqliteFtsIndex,
    root: &Path,
    base_uri: &str,
    context_type: &str,
) -> Result<(), RetrievalError> {
    if tokio::fs::try_exists(root)
        .await
        .map_err(RetrievalError::Io)?
    {
        seed_index_root(index, root, base_uri, context_type).await?;
    }
    Ok(())
}

async fn seed_index_root(
    index: &SqliteFtsIndex,
    root: &Path,
    base_uri: &str,
    context_type: &str,
) -> Result<(), RetrievalError> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(RetrievalError::Io)?;

        while let Some(entry) = entries.next_entry().await.map_err(RetrievalError::Io)? {
            let entry_path = entry.path();
            let file_type = entry.file_type().await.map_err(RetrievalError::Io)?;
            if file_type.is_dir() {
                stack.push(entry_path);
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().into_owned();
            let body = tokio::fs::read_to_string(&entry_path)
                .await
                .unwrap_or_default();
            let (uri, title, level) = match file_name.as_str() {
                ".abstract.md" => (
                    directory_uri_for_entry(root, base_uri, &entry_path),
                    ".abstract.md".to_owned(),
                    0,
                ),
                ".overview.md" => (
                    directory_uri_for_entry(root, base_uri, &entry_path),
                    ".overview.md".to_owned(),
                    1,
                ),
                _ => (path_to_uri(root, base_uri, &entry_path), file_name, 2),
            };

            index
                .index_document(&IndexedDocument {
                    uri,
                    context_type: context_type.to_owned(),
                    level,
                    title,
                    body,
                })
                .map_err(RetrievalError::Index)?;
        }
    }

    Ok(())
}

fn directory_uri_for_entry(root: &Path, projection_uri: &str, entry_path: &Path) -> String {
    let parent = entry_path.parent().unwrap_or(root);
    let rel = parent
        .strip_prefix(root)
        .unwrap_or(parent)
        .to_string_lossy()
        .replace('\\', "/");

    if rel.is_empty() {
        projection_uri.to_owned()
    } else {
        format!("{}/{}", projection_uri.trim_end_matches('/'), rel)
    }
}

fn path_to_uri(root: &Path, projection_uri: &str, entry_path: &Path) -> String {
    let rel = entry_path
        .strip_prefix(root)
        .unwrap_or(entry_path)
        .to_string_lossy()
        .replace('\\', "/");

    if rel.is_empty() {
        projection_uri.to_owned()
    } else {
        format!("{}/{}", projection_uri.trim_end_matches('/'), rel)
    }
}
