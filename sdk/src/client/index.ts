/**
 * MemFuse Client Library — public exports
 */

export { MemFuseRuntimeClient } from './runtime-client.js';
export { MemFuseOpsClient } from './ops-client.js';
export { HttpClient, MemFuseHttpError } from './http.js';
export { runMemoryLifecycle } from './runtime.js';
export { renderContextSections, renderContextText } from './render.js';
export { BaseHostMemoryAdapter } from './adapter.js';
export { GenericMemoryAdapter } from './adapters/generic.js';
export { CodexMemoryAdapter } from './adapters/codex.js';
export { ClaudeCodeMemoryAdapter } from './adapters/claude-code.js';

export type { HostMemoryAdapter } from './adapter.js';
export type { HttpClientOptions, HttpRequestOptions, RetryOptions } from './http.js';

export type {
  AppendTurnRequest,
  AppendTurnResponse,
  SearchMemoriesRequest,
  SearchMemoriesResponse,
  SearchMemoryHit,
  ResolveContextRequest,
  ResolveContextResponse,
  ResolveUnifiedContextRequest,
  ResolveUnifiedContextResponse,
  ContextDetailRequest,
  ContextDetailResponse,
  EpisodeDetailResponse,
  RelevantResourceEntry,
  ResourceProvenanceArtifact,
  ContextDetailHandle,
  RegisterManagedResourceRequest,
  RegisterManagedResourceResponse,
  ListManagedResourcesResponse,
  ManagedResource,
  ResourceSnapshot,
  ResourceSyncJob,
  ResourceChangeEvent,
  ResourceSyncResult,
  ListResourceSyncJobsResponse,
  ListResourceChangeEventsResponse,
  MemoryViewPublishResult,
  ResourceExtractionResult,
  ThreadStateResponse,
  ThreadStateTurn,
  ConsolidationJobRequest,
  MemoryJobResponse,
  MetricsSnapshot,
  HealthStatusResponse,
  ReadyStatusResponse,
  SystemStatusResponse,
  ObserverStatusResponse,
  WatchServiceStatusResponse,
  JobSummaryResponse,
  CursorSummary,
  CursorSummaryResponse,
  ConsolidationCompletionEvent,
  MemoryEventSummary,
  AuditEventEntry,
  RequestOptions,
  ReplayThreadPreviewResponse,
  ReplayJobStatusResponse,
  RebuildThreadPreviewResponse,
  RebuildUserInspectionResponse,
  RebuildJobStatusResponse,
  StartTurnInput,
  StartTurnResult,
  PrepareReadInput,
  PrepareReadResult,
  FinishTurnInput,
  FinishTurnResult,
  RenderedContextSection,
  RunMemoryLifecycleInput,
  RunMemoryLifecycleResult,
  OverlayEntry,
  FactEntry,
  EpisodeSummary,
  SessionContinuityArtifact,
  CrossThreadBriefArtifact,
  HostMemoryAdapterHooks,
  DeleteSessionResponse,
  GetSessionResponse,
  ListTasksResponse,
} from './types.js';
