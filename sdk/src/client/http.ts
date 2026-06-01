/**
 * MemFuse SDK HTTP Client
 *
 * Fetch-based HTTP client with retry support for programmatic SDK use.
 * B1: supports CanvasRouter for multi-endpoint routing (canvas → local, other → cloud).
 */

import { CanvasRouter } from '../shared/router.js';

export class MemFuseHttpError extends Error {
  readonly status: number;
  readonly body: unknown;
  readonly retryable: boolean;

  constructor(status: number, body: unknown, retryable = false) {
    super(`MemFuse request failed with status ${status}`);
    this.name = 'MemFuseHttpError';
    this.status = status;
    this.body = body;
    this.retryable = retryable;
  }
}

export interface HttpClientOptions {
  baseUrl: string;
  fetchImpl?: typeof fetch;
  defaultHeaders?: Record<string, string>;
  retry?: RetryOptions;
  /** Optional CanvasRouter for multi-endpoint routing.
   *  When provided, Canvas paths route to localCanvasUrl, others to cloudUrl.
   *  When omitted, all requests go to baseUrl (backward compatible). */
  router?: CanvasRouter;
}

export interface HttpRequestOptions {
  headers?: Record<string, string>;
  traceId?: string;
  idempotencyKey?: string;
  signal?: AbortSignal;
}

export interface RetryOptions {
  maxAttempts?: number;
  backoffMs?: number;
}

export class HttpClient {
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;
  private readonly defaultHeaders: Record<string, string>;
  private readonly retry: Required<RetryOptions>;
  private readonly router?: CanvasRouter;

  constructor(options: HttpClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, '');
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.defaultHeaders = options.defaultHeaders ?? {};
    this.router = options.router;
    this.retry = {
      maxAttempts: options.retry?.maxAttempts ?? 3,
      backoffMs: options.retry?.backoffMs ?? 250,
    };
  }

  async get<T>(path: string, options?: HttpRequestOptions): Promise<T> {
    return this.request<T>(path, {
      method: 'GET',
      headers: options?.headers,
      traceId: options?.traceId,
      idempotencyKey: options?.idempotencyKey,
      signal: options?.signal,
    });
  }

  async post<T>(path: string, body: unknown, options?: HttpRequestOptions): Promise<T> {
    return this.request<T>(path, {
      method: 'POST',
      body,
      headers: options?.headers,
      traceId: options?.traceId,
      idempotencyKey: options?.idempotencyKey,
      signal: options?.signal,
    });
  }

  async put<T>(path: string, body: unknown, options?: HttpRequestOptions): Promise<T> {
    return this.request<T>(path, {
      method: 'PUT',
      body,
      headers: options?.headers,
      traceId: options?.traceId,
      idempotencyKey: options?.idempotencyKey,
      signal: options?.signal,
    });
  }

  async delete<T>(path: string, options?: HttpRequestOptions): Promise<T> {
    return this.request<T>(path, {
      method: 'DELETE',
      headers: options?.headers,
      traceId: options?.traceId,
      idempotencyKey: options?.idempotencyKey,
      signal: options?.signal,
    });
  }

  private async request<T>(
    path: string,
    options: { method: string; body?: unknown; headers?: Record<string, string>; traceId?: string; idempotencyKey?: string; signal?: AbortSignal },
  ): Promise<T> {
    const headers: Record<string, string> = {
      ...this.defaultHeaders,
      ...options.headers,
    };
    headers['Content-Type'] = 'application/json';
    if (options.traceId) headers['x-trace-id'] = options.traceId;
    if (options.idempotencyKey) headers['idempotency-key'] = options.idempotencyKey;

    let lastError: unknown;
    for (let attempt = 1; attempt <= this.retry.maxAttempts; attempt++) {
      try {
        // Resolve URL: with router → route by path type; without → use baseUrl
        const effectiveUrl = this.router
          ? this.router.resolveBackend(path).url
          : this.baseUrl;
        const response = await this.fetchImpl(`${effectiveUrl}${path}`, {
          method: options.method,
          headers,
          body: options.body === undefined ? undefined : JSON.stringify(options.body),
          signal: options.signal,
        });

        const text = await response.text();
        const responseBody = text ? safeParseJSON(text) : null;
        if (!response.ok) {
          const retryable = isRetryableStatus(response.status);
          const error = new MemFuseHttpError(response.status, responseBody, retryable);
          if (!retryable || attempt === this.retry.maxAttempts) throw error;
          lastError = error;
          await sleep(this.retry.backoffMs, attempt);
          continue;
        }
        return responseBody as T;
      } catch (error) {
        if (!isRetryableError(error) || attempt === this.retry.maxAttempts) throw error;
        lastError = error;
        await sleep(this.retry.backoffMs, attempt);
      }
    }
    throw lastError;
  }
}

function safeParseJSON(text: string): unknown {
  try { return JSON.parse(text); }
  catch { return text; }
}

function isRetryableStatus(status: number): boolean {
  return status === 408 || status === 429 || (status >= 500 && status <= 599);
}

function isRetryableError(error: unknown): boolean {
  if (error instanceof MemFuseHttpError) return error.retryable;
  return error instanceof TypeError;
}

async function sleep(backoffMs: number, attempt: number): Promise<void> {
  if (backoffMs <= 0) return;
  await new Promise(resolve => setTimeout(resolve, backoffMs * attempt));
}