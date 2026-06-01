/**
 * MemFuse SDK Types
 *
 * Canonical types for the MemFuse SDK client library.
 * Handle type uses "mfs_uri" (MemFuse URI scheme).
 */

export type TurnRole = 'user' | 'assistant';

export interface AppendTurnRequest {
  user_id: string;
  resource_id?: string;
  message: {
    role: TurnRole;
    content_text: string;
    token_count?: number;
  };
  request_metadata?: Record<string, unknown>;
}

export interface AppendTurnResponse {
  ok: boolean;
  session_id: string;
}

export interface SearchMemoriesRequest {
  user_id?: string;
  session_id?: string;
  query: string;
  limit?: number;
}

export interface SearchMemoryHit {
  episode_id: string;
  summary: string;
  score?: number;
  salience_score?: number;
  created_at?: string;
}

export interface SearchMemoriesResponse {
  results: SearchMemoryHit[];
  total: number;
}

export interface ResolveContextRequest {
  user_id?: string;
  session_id: string;
  resource_id?: string;
  query: string;
  token_budget?: number;
}

export interface OverlayEntry {
  turn_id: string;
  role: string;
  content: string;
}

export interface FactEntry {
  fact_id: string;
  predicate: string;
  display_value: string;
  confidence: number;
}

export interface EpisodeSummary {
  episode_id: string;
  summary: string;
  salience: number;
  emotional_valence?: number;
  emotional_intensity?: number;
  context_tags_json?: string;
  created_at?: string;
}

export interface SessionContinuityArtifact {
  scope?: string;
  state?: string;
  summary?: string;
  recent_turn_ids?: string[];
  anchor_episode_ids?: string[];
}

export interface CrossThreadBriefArtifact {
  scope?: string;
  scope_id?: string;
  summary?: string;
  anchor_episode_ids?: string[];
  source_thread_ids?: string[];
}

export interface ResolveContextResponse {
  sections: {
    recent_updates?: OverlayEntry[];
    current_facts?: FactEntry[];
    relevant_history?: EpisodeSummary[];
  };
  artifacts: {
    session_continuity?: SessionContinuityArtifact;
    cross_thread_briefs?: CrossThreadBriefArtifact[];
  };
  detail_handles?: string[];
  rendered_markdown?: string;
  debug_metadata?: {
    query?: string;
    token_budget?: number;
    recent_updates_count?: number;
    current_facts_count?: number;
    relevant_history_count?: number;
    detail_handle_count?: number;
  };
}

export interface ResolveUnifiedContextRequest {
  user_id?: string;
  session_id: string;
  resource_id?: string;
  resource_scopes?: string[];
  planes?: string[];
  target_uris?: string[];
  query: string;
  token_budget?: number;
}

export interface RelevantResourceEntry {
  uri: string;
  root_uri?: string;
  context_type?: string;
  level: number;
  score: number;
  excerpt?: string;
  retrieval_plane?: string;
  match_reason?: string;
  resource_id?: string;
  snapshot_id?: string;
}

export interface ResourceProvenanceArtifact {
  uri: string;
  root_uri?: string;
  projection_view_id?: string;
  workspace_path?: string;
  source_kind?: string;
  source_identifier?: string;
  snapshot_id?: string;
  audit_event_types?: string[];
}

/** Handle type for context detail references (uses mfs_uri scheme) */
export interface ContextDetailHandle {
  type: 'episode' | 'mfs_uri';
  plane: 'memory' | 'resource';
  episode_id?: string;
  uri?: string;
  resource_id?: string;
  snapshot_id?: string;
}

export interface ResolveUnifiedContextResponse {
  sections: {
    recent_updates?: OverlayEntry[];
    current_facts?: FactEntry[];
    relevant_history?: EpisodeSummary[];
    relevant_resources?: RelevantResourceEntry[];
  };
  artifacts: {
    session_continuity?: SessionContinuityArtifact;
    cross_thread_briefs?: CrossThreadBriefArtifact[];
    resource_provenance?: ResourceProvenanceArtifact[];
    retrieval_trace_id?: string;
  };
  detail_handles?: ContextDetailHandle[];
  rendered_markdown?: string;
  debug_metadata?: {
    query?: string;
    token_budget?: number;
    recent_updates_count?: number;
    current_facts_count?: number;
    relevant_history_count?: number;
    relevant_resources_count?: number;
    detail_handle_count?: number;
  };
}

export interface RegisterManagedResourceRequest {
  user_id: string;
  logical_name?: string;
  source_kind?: string;
  source_identifier?: string;
  file_name?: string;
  content?: string;
  policy?: Record<string, unknown>;
}

export interface ManagedResource {
  resource_id: string;
  user_id: string;
  logical_name: string;
  source_kind: string;
  source_identifier: string;
  root_uri: string;
  source_resource_id?: string;
  source_host?: string;
  source_namespace?: string;
  source_repo?: string;
  source_ref?: string;
  status: string;
  policy_json?: unknown;
  created_at?: string;
  updated_at?: string;
}

export interface ResourceSnapshot {
  snapshot_id: string;
  resource_id: string;
  user_id: string;
  source_snapshot_id: string;
  root_uri: string;
  manifest_digest?: string | null;
  changed_uri_count: number;
  created_at?: string;
}

export interface RegisterManagedResourceResponse {
  resource: ManagedResource;
  snapshot?: ResourceSnapshot;
  task_key?: string;
}

export interface ListManagedResourcesResponse {
  count: number;
  resources: ManagedResource[];
}

export interface ResourceSyncJob {
  job_id: string;
  resource_id: string;
  user_id: string;
  operation: string;
  status: string;
  payload_json?: unknown;
  error_text?: string | null;
  scheduled_at?: string;
  finished_at?: string | null;
}

export interface ResourceChangeEvent {
  event_id: string;
  resource_id: string;
  user_id: string;
  uri: string;
  change_type: string;
  content_digest?: string | null;
  snapshot_id?: string | null;
  processed_at?: string | null;
  created_at?: string;
}

export interface ResourceSyncResult {
  sync_job: ResourceSyncJob;
  snapshot?: ResourceSnapshot;
  change_events?: ResourceChangeEvent[];
}

export interface ListResourceSyncJobsResponse {
  count: number;
  jobs: ResourceSyncJob[];
}

export interface ListResourceChangeEventsResponse {
  count: number;
  events: ResourceChangeEvent[];
}

export interface MemoryViewPublishResult {
  user_id: string;
  published_uris: string[];
}

export interface ResourceExtractionResult {
  resource_id: string;
  processed_events: number;
  assertion_count: number;
  published_uris?: string[];
}

export interface ThreadStateTurn {
  thread_id: string;
  user_id: string;
  resource_id?: string;
  turn_id: string;
  turn_seq: number;
  role: string;
  content_text: string;
  token_count: number;
  created_at?: string;
  ingested_at?: string;
}

export interface ThreadStateResponse {
  session: {
    thread_id: string;
    user_id: string;
    resource_id?: string;
    latest_turn_id?: string;
    latest_turn_seq?: number;
    updated_at?: string;
  };
  recent_turns?: ThreadStateTurn[];
}

export interface EpisodeDetailResponse {
  episode_id: string;
  session_id?: string;
  resource_id?: string;
  summary?: string;
  salience_score?: number;
  strength_score?: number;
  created_at?: string;
  facts?: Array<Record<string, unknown>>;
  turns?: Array<Record<string, unknown>>;
}

export interface ContextDetailRequest {
  user_id: string;
  handle: ContextDetailHandle;
  include_content?: boolean;
}

export interface ResourceDetailResponse {
  uri: string;
  resource_id?: string;
  snapshot_id?: string;
  abstract_text?: string;
  overview_text?: string;
  content?: string;
}

export interface ContextDetailResponse {
  handle: ContextDetailHandle;
  episode?: Record<string, unknown>;
  detail?: Array<Record<string, unknown>>;
  resource?: ResourceDetailResponse;
}

export interface ConsolidationJobRequest {
  scope_type: 'thread' | 'resource' | 'user';
  scope_id: string;
  range_start_turn_id?: string;
  range_end_turn_id?: string;
}

export interface MemoryJobResponse {
  job_id: string;
  job_type: string;
  scope_type: string;
  scope_id: string;
  range_start_turn_id?: string;
  range_end_turn_id?: string;
  dedupe_key?: string;
  status: string;
  retry_count?: number;
  scheduled_at?: string;
  finished_at?: string | null;
  error_text?: string | null;
}

export interface DeleteSessionResponse {
  session_id: string;
  deleted: boolean;
}

export interface GetSessionResponse {
  session_id: string;
  user_id: string;
  thread_id?: string;
  latest_turn_id?: string;
  latest_turn_seq?: number;
  updated_at?: string;
}

export interface ListTasksResponse {
  count: number;
  tasks: MemoryJobResponse[];
}

export interface MetricsSnapshot {
  process_role: string;
  counters: Record<string, number>;
  gauges: Record<string, number>;
  updated_at?: string;
}

export interface HealthStatusResponse {
  status: string;
  version?: string;
  summary_provider?: string;
  embedding_provider?: string;
}

export interface ReadyStatusResponse {
  status: string;
  checks?: Record<string, unknown>;
}

export interface SystemTaskStateCounts {
  total: number;
  pending: number;
  running: number;
  completed: number;
  failed: number;
}

export interface SystemResourceStatusCounts {
  total: number;
  ready: number;
  processing: number;
  failed: number;
}

export interface RetrievalCacheStatus {
  entries: number;
  builds: number;
  hits: number;
  invalidations: number;
}

export interface SystemStatusResponse {
  workspace_root: string;
  resources: SystemResourceStatusCounts;
  metadata_tasks: SystemTaskStateCounts;
  session_tasks: SystemTaskStateCounts;
  snapshots_total: number;
  runtime?: {
    status?: string;
    retrieval_cache?: RetrievalCacheStatus;
  };
}

export interface ObserverStatusResponse {
  runtime: {
    summary_provider?: string;
    embedding_provider?: string;
    rerank_provider?: string;
    retrieval_cache?: RetrievalCacheStatus;
    [key: string]: unknown;
  };
  semantic: {
    total_documents: number;
    resource_documents: number;
    memory_documents: number;
    skill_documents: number;
    embedding_dimension: number;
  };
}

export interface WatchServiceStatusResponse {
  running: boolean;
  poll_ms: number;
  started_at_ms?: number | null;
  stopped_at_ms?: number | null;
  last_tick_at_ms?: number | null;
  total_ticks: number;
  total_runs: number;
  last_run_count: number;
}

export interface JobSummaryResponse {
  status_counts: Record<string, number>;
  scope_counts: Record<string, number>;
  oldest_queued_at?: string;
  oldest_running_lease_expires_at?: string;
}

export interface CursorSummary {
  cursor_id: string;
  user_id: string;
  scope_type: string;
  scope_id: string;
  last_consolidated_turn_id: string;
  last_consolidated_at?: string;
  updated_at?: string;
}

/** Completion event attached to a consolidation/rebuild job. */
export interface ConsolidationCompletionEvent {
  range_start_turn_id: string;
  range_end_turn_id: string;
  episode_count: number;
  assertion_count: number;
  fact_count: number;
  turn_count: number;
}

/** Aggregated event counts from the audit trail. */
export interface MemoryEventSummary {
  total_events: number;
  event_type_counts: Record<string, number>;
  oldest_event_at?: string;
  newest_event_at?: string;
}

/** Audit trail entry from the metadata store. */
export interface AuditEventEntry {
  id: number;
  event_type: string;
  user_id: string;
  agent_id?: string;
  subject_uri?: string;
  actor?: string;
  details_json?: string;
  recorded_at: string;
}

export interface CursorSummaryResponse {
  count: number;
  cursors: CursorSummary[];
}

export interface ReplayThreadPreviewResponse {
  thread: {
    thread_id: string;
    user_id: string;
    turn_count: number;
    cursor?: CursorSummary | null;
  };
  derived_state: {
    episode_count: number;
    latest_episode_end_turn_id?: string;
  };
  replay_scope: {
    scope_type: string;
    replay_mode: string;
    note: string;
  };
}

export interface ReplayJobStatusResponse {
  job_id: string;
  status: string;
  scope_type: string;
  scope_id: string;
  scheduled_at?: string;
  finished_at?: string | null;
  completion_event?: ConsolidationCompletionEvent | null;
  retry_count?: number;
}

export interface RebuildThreadPreviewResponse {
  thread: {
    thread_id: string;
    user_id: string;
    turn_count: number;
    status: string;
    cursor?: CursorSummary | null;
  };
  derived_state: {
    episode_count: number;
    active_fact_count_for_user: number;
  };
  recent_events?: MemoryJobResponse[];
  recent_failures?: {
    thread_jobs?: MemoryJobResponse[];
    user_jobs?: MemoryJobResponse[];
  };
  execute_scope?: {
    user_id: string;
    note: string;
  };
}

export interface RebuildUserInspectionResponse {
  user_id: string;
  derived_state: {
    session_count: number;
    active_fact_count: number;
    assertion_count: number;
    episode_count: number;
    thread_cursor_count: number;
  };
  event_summary: MemoryEventSummary;
  latest_rebuild?: MemoryJobResponse | null;
  recent_events?: {
    memory_context?: AuditEventEntry[];
    consolidation?: MemoryJobResponse[];
    cursor_advancement?: CursorSummary[];
    rebuild?: MemoryJobResponse[];
  };
}

export interface RebuildJobStatusResponse {
  job_id: string;
  status: string;
  scope_type: string;
  scope_id: string;
  scheduled_at?: string;
  finished_at?: string | null;
  completion_event?: ConsolidationCompletionEvent | null;
  retry_count?: number;
  compare_summary?: {
    before?: {
      session_count?: number;
      fact_count?: number;
      assertion_count?: number;
      episode_count?: number;
      thread_cursor_count?: number;
    };
    after?: {
      session_count?: number;
      fact_count?: number;
      assertion_count?: number;
      episode_count?: number;
    };
    delta?: {
      fact_count?: number;
      assertion_count?: number;
      episode_count?: number;
    };
  };
}

export interface RunMemoryLifecycleInput {
  threadId: string;
  userId: string;
  userMessage: string;
  queryText: string;
  assistantMessage: string;
  resourceId?: string;
  budget?: number;
  userTurnOptions?: RequestOptions;
  resolveContextOptions?: RequestOptions;
  assistantTurnOptions?: RequestOptions;
}

export interface RunMemoryLifecycleResult {
  userTurn: AppendTurnResponse;
  context: ResolveContextResponse;
  assistantTurn: AppendTurnResponse;
}

export interface StartTurnInput {
  threadId: string;
  userId: string;
  userMessage: string;
  queryText: string;
  resourceId?: string;
  budget?: number;
  appendUserTurnOptions?: RequestOptions;
  resolveContextOptions?: RequestOptions;
}

export interface StartTurnResult {
  userTurn: AppendTurnResponse;
  context: ResolveContextResponse;
}

export interface PrepareReadInput {
  threadId: string;
  userId: string;
  filePath: string;
  limit?: number;
  searchOptions?: RequestOptions;
  factsOptions?: RequestOptions;
}

export interface PrepareReadResult {
  filePath: string;
  relatedEpisodes: SearchMemoryHit[];
  relatedFacts: FactEntry[];
  renderedText: string;
}

export interface PreparedTurnResult extends StartTurnResult {
  renderedSections: RenderedContextSection[];
  renderedText: string;
}

export interface FinishTurnInput {
  threadId: string;
  userId: string;
  assistantMessage: string;
  resourceId?: string;
  appendAssistantTurnOptions?: RequestOptions;
}

export interface FinishTurnResult {
  assistantTurn: AppendTurnResponse;
}

export interface RequestOptions {
  headers?: Record<string, string>;
  traceId?: string;
  idempotencyKey?: string;
  signal?: AbortSignal;
}

export interface HostMemoryAdapterHooks {
  onBeforeAppendUserTurn?(input: StartTurnInput): void | Promise<void>;
  onAfterAppendUserTurn?(result: AppendTurnResponse): void | Promise<void>;
  onAfterResolveContext?(result: ResolveContextResponse): void | Promise<void>;
  onBeforeAppendAssistantTurn?(input: FinishTurnInput): void | Promise<void>;
  onAfterAppendAssistantTurn?(result: AppendTurnResponse): void | Promise<void>;
}

export interface RenderedContextSection {
  id: 'current_facts' | 'recent_updates' | 'relevant_history';
  title: string;
  lines: string[];
}
