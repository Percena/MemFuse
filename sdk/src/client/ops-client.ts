/**
 * MemFuse Ops Client
 *
 * Internal/operations-facing client for task tracking and session management.
 * Paths aligned with Rust mfs-server Axum routes (no /mcp, /v1, /v2 prefixes).
 */

import { HttpClient, type HttpClientOptions } from './http.js';
import type {
  DeleteSessionResponse,
  GetSessionResponse,
  HealthStatusResponse,
  ListTasksResponse,
  ObserverStatusResponse,
  RequestOptions,
  ReadyStatusResponse,
  SystemStatusResponse,
  WatchServiceStatusResponse,
} from './types.js';

export class MemFuseOpsClient {
  private readonly http: HttpClient;

  constructor(options: HttpClientOptions) {
    this.http = new HttpClient(options);
  }

  // ─── Service Health / Operability ─────────────────────────────────

  getHealth(options?: RequestOptions): Promise<HealthStatusResponse> {
    return this.http.get<HealthStatusResponse>('/health', options);
  }

  getReady(options?: RequestOptions): Promise<ReadyStatusResponse> {
    return this.http.get<ReadyStatusResponse>('/ready', options);
  }

  getSystemStatus(options?: RequestOptions): Promise<SystemStatusResponse> {
    return this.http.get<SystemStatusResponse>('/system/status', options);
  }

  getObserverStatus(options?: RequestOptions): Promise<ObserverStatusResponse> {
    return this.http.get<ObserverStatusResponse>('/system/observer', options);
  }

  getWatchServiceStatus(options?: RequestOptions): Promise<WatchServiceStatusResponse> {
    return this.http.get<WatchServiceStatusResponse>('/watch-service/status', options);
  }

  // ─── Task Tracking ─────────────────────────────────────────────────
  // Rust server: GET /tasks/{taskId}, GET /tasks/{taskId}/wait

  getTaskStatus(taskId: string, options?: RequestOptions): Promise<Record<string, unknown>> {
    return this.http.get<Record<string, unknown>>(
      `/tasks/${encodeURIComponent(taskId)}`,
      options,
    );
  }

  waitTask(taskId: string, options?: RequestOptions): Promise<Record<string, unknown>> {
    return this.http.get<Record<string, unknown>>(
      `/tasks/${encodeURIComponent(taskId)}/wait`,
      options,
    );
  }

  listTasks(limit?: number, options?: RequestOptions): Promise<ListTasksResponse> {
    const params = new URLSearchParams();
    if (limit !== undefined) params.set('limit', String(limit));
    const qs = params.toString();
    return this.http.get<ListTasksResponse>(`/tasks${qs ? `?${qs}` : ''}`, options);
  }

  // ─── Session Commit ────────────────────────────────────────────────
  // Rust server: POST /sessions/{sessionId}/commit

  commitSession(sessionId: string, options?: RequestOptions): Promise<{ archive_uri: string; task_id: string | null }> {
    return this.http.post<{ archive_uri: string; task_id: string | null }>(
      `/sessions/${encodeURIComponent(sessionId)}/commit`,
      {},
      options,
    );
  }

  // ─── Session Management ────────────────────────────────────────────
  // Rust server: GET /sessions, POST /sessions, DELETE /sessions/{id}

  createSession(
    accountId: string,
    userId: string,
    agentId: string,
    options?: RequestOptions,
  ): Promise<{ session_id: string }> {
    return this.http.post<{ session_id: string }>('/sessions', {
      account_id: accountId, user_id: userId, agent_id: agentId,
    }, options);
  }

  listSessions(
    accountId: string,
    userId: string,
    agentId: string,
    options?: RequestOptions,
  ): Promise<Record<string, unknown>> {
    const params = new URLSearchParams({
      account_id: accountId, user_id: userId, agent_id: agentId,
    });
    return this.http.get<Record<string, unknown>>(`/sessions?${params.toString()}`, options);
  }

  deleteSession(sessionId: string, options?: RequestOptions): Promise<DeleteSessionResponse> {
    return this.http.delete<DeleteSessionResponse>(
      `/sessions/${encodeURIComponent(sessionId)}`,
      options,
    );
  }

  getSession(sessionId: string, options?: RequestOptions): Promise<GetSessionResponse> {
    return this.http.get<GetSessionResponse>(
      `/sessions/${encodeURIComponent(sessionId)}`,
      options,
    );
  }
}
