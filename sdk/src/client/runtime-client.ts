/**
 * MemFuse Runtime Client
 *
 * Paths aligned with Rust mfs-server Axum routes (no /mcp, /v1, /v2 prefixes).
 */

import { HttpClient, type HttpClientOptions } from './http.js';
import type {
  EpisodeDetailResponse,
  ListManagedResourcesResponse,
  RegisterManagedResourceRequest,
  RegisterManagedResourceResponse,
  SearchMemoriesRequest,
  SearchMemoriesResponse,
  RequestOptions,
  FactEntry,
  ResolveContextRequest,
  ResolveContextResponse,
  ResolveUnifiedContextRequest,
  ResolveUnifiedContextResponse,
  ResourceSyncResult,
} from './types.js';

export class MemFuseRuntimeClient {
  protected readonly http: HttpClient;

  constructor(options: HttpClientOptions) {
    this.http = new HttpClient(options);
  }

  // ─── Session Messages ──────────────────────────────────────────────
  // Rust server: POST /sessions/{sessionId}/messages
  // Both user and assistant turns use the same endpoint with role in body.

  appendUserTurn(
    sessionId: string,
    request: { content: string; metadata?: Record<string, unknown> },
    options?: RequestOptions,
  ): Promise<{ ok: boolean; session_id: string }> {
    return this.http.post<{ ok: boolean; session_id: string }>(
      `/sessions/${encodeURIComponent(sessionId)}/messages`,
      { role: 'user', content: request.content, ...request.metadata },
      options,
    );
  }

  appendAssistantTurn(
    sessionId: string,
    request: { content: string; metadata?: Record<string, unknown> },
    options?: RequestOptions,
  ): Promise<{ ok: boolean; session_id: string }> {
    return this.http.post<{ ok: boolean; session_id: string }>(
      `/sessions/${encodeURIComponent(sessionId)}/messages`,
      { role: 'assistant', content: request.content, ...request.metadata },
      options,
    );
  }

  // ─── Context Resolution ────────────────────────────────────────────
  // Rust server: POST /context/resolve (unified, no /v1 or /v2 prefix)

  resolveContext(request: ResolveContextRequest, options?: RequestOptions): Promise<ResolveContextResponse> {
    return this.http
      .post<Record<string, unknown>>('/context/resolve', request, options)
      .then(normalizeResolveContextResponse);
  }

  resolveUnifiedContext(
    request: ResolveUnifiedContextRequest,
    options?: RequestOptions,
  ): Promise<ResolveUnifiedContextResponse> {
    return this.http
      .post<Record<string, unknown>>('/context/resolve', request, options)
      .then(normalizeResolveContextResponse as (value: Record<string, unknown>) => ResolveUnifiedContextResponse);
  }

  getEpisode(episodeId: string, options?: RequestOptions): Promise<EpisodeDetailResponse> {
    return this.http.get<EpisodeDetailResponse>(
      `/episodes/${encodeURIComponent(episodeId)}`,
      options,
    );
  }

  searchMemories(
    request: SearchMemoriesRequest,
    options?: RequestOptions,
  ): Promise<SearchMemoriesResponse> {
    return this.http
      .post<Record<string, unknown>>('/v1/memory:search', request, options)
      .then(normalizeSearchMemoriesResponse);
  }

  listFacts(userId: string, options?: RequestOptions): Promise<FactEntry[]> {
    return this.http
      .get<Record<string, unknown>>(`/facts?user_id=${encodeURIComponent(userId)}`, options)
      .then(raw => {
        const facts = Array.isArray(raw.facts) ? raw.facts as Record<string, unknown>[] : [];
        return facts.map(fact => ({
          fact_id: String(fact.fact_id || ''),
          predicate: String(fact.predicate || ''),
          display_value: String(fact.display_value || ''),
          confidence: Number(fact.confidence || 0),
        }));
      });
  }

  // ─── Resources ─────────────────────────────────────────────────────
  // Rust server: /resources (no /v1 prefix, colon→slash separator)

  registerResource(
    request: RegisterManagedResourceRequest,
    options?: RequestOptions,
  ): Promise<RegisterManagedResourceResponse> {
    return this.http.post<RegisterManagedResourceResponse>('/resources', request, options);
  }

  listResources(
    userId: string,
    limit?: number,
    options?: RequestOptions,
  ): Promise<ListManagedResourcesResponse> {
    const params = new URLSearchParams({ user_id: userId });
    if (limit !== undefined) params.set('limit', String(limit));
    return this.http.get<ListManagedResourcesResponse>(`/resources?${params.toString()}`, options);
  }

  refreshResource(
    resourceId: string,
    userId: string,
    options?: RequestOptions,
  ): Promise<ResourceSyncResult> {
    return this.http.post<ResourceSyncResult>(
      `/resources/${encodeURIComponent(resourceId)}/refresh?user_id=${encodeURIComponent(userId)}`,
      {},
      options,
    );
  }

  rebuildResource(
    resourceId: string,
    userId: string,
    options?: RequestOptions,
  ): Promise<ResourceSyncResult> {
    return this.http.post<ResourceSyncResult>(
      `/resources/${encodeURIComponent(resourceId)}/rebuild?user_id=${encodeURIComponent(userId)}`,
      {},
      options,
    );
  }

  runResourceWatch(
    resourceId: string,
    userId: string,
    options?: RequestOptions,
  ): Promise<ResourceSyncResult> {
    return this.http.post<ResourceSyncResult>(
      `/resources/${encodeURIComponent(resourceId)}/watch/run?user_id=${encodeURIComponent(userId)}`,
      {},
      options,
    );
  }
}

function normalizeResolveContextResponse(raw: Record<string, unknown>): ResolveContextResponse {
  const sections = raw.sections as ResolveContextResponse['sections'] ?? { recent_updates: [], current_facts: [], relevant_history: [] };
  const artifacts = raw.artifacts as ResolveContextResponse['artifacts'] ?? {};
  const detailHandles = Array.isArray(raw.detail_handles) ? raw.detail_handles as string[] : [];

  return {
    sections,
    artifacts,
    detail_handles: detailHandles,
    rendered_markdown: typeof raw.rendered_markdown === 'string' ? raw.rendered_markdown : undefined,
    debug_metadata: {
      query: typeof raw.query === 'string' ? raw.query : undefined,
      token_budget: typeof raw.token_budget === 'number' ? raw.token_budget : undefined,
      recent_updates_count: sections.recent_updates?.length ?? 0,
      current_facts_count: sections.current_facts?.length ?? 0,
      relevant_history_count: sections.relevant_history?.length ?? 0,
      detail_handle_count: detailHandles.length,
    },
  };
}

function normalizeSearchMemoriesResponse(raw: Record<string, unknown>): SearchMemoriesResponse {
  const results = Array.isArray(raw.results) ? raw.results as Record<string, unknown>[] : [];
  return {
    results: results.map(item => ({
      episode_id: String(item.episode_id || item.id || ''),
      summary: String(item.summary || item.content || ''),
      score: item.score != null ? Number(item.score) : undefined,
      salience_score: item.salience_score != null ? Number(item.salience_score) : undefined,
      created_at: item.created_at != null ? String(item.created_at) : undefined,
    })),
    total: typeof raw.total === 'number' ? raw.total : results.length,
  };
}
