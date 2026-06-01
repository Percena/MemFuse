/**
 * MemFuse Hooks — exports for programmatic use
 */

export { adaptInput, detectPlatform, formatOutput, stripPrivate, sanitizeSecrets, sanitizeMemoryText, truncate, isDegradableError, readStdin, EXIT_OK, EXIT_FATAL, PATHS, callBackend, checkHealth, CanvasRouter, router, loadConfig } from './platform-utils.js';
export type { Platform, AdaptedInput } from './platform-utils.js';
