// Privacy helpers shared by hooks and MCP tools.

export function stripPrivate(content: string): string {
  if (!content || typeof content !== 'string') return content || '';
  let output = '';
  let depth = 0;
  let i = 0;
  const lower = content.toLowerCase();
  const open = '<private>';
  const close = '</private>';

  while (i < content.length) {
    if (lower.startsWith(open, i)) {
      depth += 1;
      i += open.length;
      continue;
    }
    if (lower.startsWith(close, i)) {
      if (depth > 0) depth -= 1;
      i += close.length;
      continue;
    }
    if (depth === 0) output += content[i];
    i += 1;
  }

  return output;
}

export function sanitizeSecrets(content: string): string {
  if (!content || typeof content !== 'string') return content || '';
  return content
    .replace(/sk-[A-Za-z0-9_-]{8,}/g, 'sk-[REDACTED]')
    .replace(/Bearer [A-Za-z0-9_.-]{8,}/g, 'Bearer [REDACTED]')
    .replace(/ghp_[A-Za-z0-9]{8,}/g, 'ghp_[REDACTED]')
    .replace(/cr_[A-Za-z0-9]{8,}/g, 'cr_[REDACTED]');
}

export function sanitizeMemoryText(content: string): string {
  return sanitizeSecrets(stripPrivate(content));
}
