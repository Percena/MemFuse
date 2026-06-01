/**
 * @percena/memfuse — unified package entry point
 *
 * Provides MCP server, hooks, skills, and client library in one package.
 */

// MCP server
export { startServer } from './mcp/server.js';

// Hooks
export * from './hooks/index.js';

// Skills
export * from './skills/index.js';

// Shared
export { loadConfig } from './shared/config.js';
export type { MemFuseConfig } from './shared/config.js';
export { callServer, callServerWithUrl, callBackend, checkHealth, httpRequest, MemFuseNetworkError } from './shared/http.js';
export { CanvasRouter, isCanvasPath, isCanvasWritePath, latestCanvasSnapshotPath } from './shared/router.js';
export { extractErrorMessage, toArray } from './shared/utils.js';

// Client library
export * from './client/index.js';
