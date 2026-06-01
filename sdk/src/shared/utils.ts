/**
 * MemFuse SDK — Shared utility functions
 */

/**
 * Extract a human-readable error message from an error field.
 *
 * Handles both the old flat format (`"error": "message"`) and the new
 * nested format (`"error": {"category": "...", "message": "...", "retryable": bool}`).
 */
export function extractErrorMessage(error: unknown): string {
  if (error == null) return '';
  if (typeof error === 'string') return error;
  if (typeof error === 'object' && error !== null) {
    const obj = error as Record<string, unknown>;
    // Nested format: { category, message, retryable }
    if (obj.message && typeof obj.message === 'string') {
      const category = typeof obj.category === 'string' ? obj.category : '';
      return category ? `${category}: ${obj.message}` : obj.message;
    }
  }
  return String(error);
}

/** Normalize array-like API payload fields. Non-arrays become an empty array. */
export function toArray<T = Record<string, unknown>>(val: unknown): T[] {
  if (Array.isArray(val)) return val as T[];
  return [];
}
