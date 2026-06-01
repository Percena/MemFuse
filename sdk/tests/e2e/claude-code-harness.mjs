export function mcpToolSucceeded(callResult) {
  return Boolean(callResult?.ok && !callResult?.result?.isError);
}

export function extractFactIdFromMcpResult(result) {
  const direct = result?.fact_id || result?.id;
  if (typeof direct === 'string' && direct.startsWith('fact_')) return direct;

  const text = collectText(result);
  const idLineMatch = text.match(/\bID:\s*(fact_[A-Za-z0-9_-]+)/);
  if (idLineMatch) return idLineMatch[1];

  const anyFactIdMatch = text.match(/\bfact_[A-Za-z0-9_-]+\b/);
  return anyFactIdMatch ? anyFactIdMatch[0] : '';
}

export function listMissingEvidence(evidence, keys) {
  return keys.filter((key) => {
    const value = evidence?.[key];
    if (value === true) return false;
    if (typeof value === 'string' && value.length > 0) return false;
    if (Array.isArray(value) && value.length > 0) return false;
    return true;
  });
}

export function failScenarioIfMissingEvidence(scenario, keys, failureClass) {
  const missing = listMissingEvidence(scenario.evidence, keys);
  if (missing.length === 0) return false;
  scenario.status = 'failed';
  scenario.failure_class = failureClass;
  scenario.evidence.missing_evidence = missing;
  return true;
}

export function createJsonRpcFrameParser(onMessage) {
  let buffer = Buffer.alloc(0);

  return {
    push(chunk) {
      buffer = Buffer.concat([buffer, Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)]);
      while (true) {
        const headerEnd = buffer.indexOf('\r\n\r\n');
        if (headerEnd === -1) {
          const lineEnd = buffer.indexOf('\n');
          if (lineEnd === -1) return;
          const line = buffer.subarray(0, lineEnd).toString('utf8').trim();
          buffer = buffer.subarray(lineEnd + 1);
          if (line.length > 0) onMessage(JSON.parse(line));
          continue;
        }

        const header = buffer.subarray(0, headerEnd).toString('ascii');
        const match = header.match(/Content-Length:\s*(\d+)/i);
        if (!match) {
          const lineEnd = buffer.indexOf('\n');
          if (lineEnd === -1) return;
          const line = buffer.subarray(0, lineEnd).toString('utf8').trim();
          buffer = buffer.subarray(lineEnd + 1);
          if (line.length > 0) onMessage(JSON.parse(line));
          continue;
        }

        const length = Number.parseInt(match[1], 10);
        const bodyStart = headerEnd + 4;
        const bodyEnd = bodyStart + length;
        if (buffer.length < bodyEnd) return;

        const body = buffer.subarray(bodyStart, bodyEnd).toString('utf8');
        buffer = buffer.subarray(bodyEnd);
        onMessage(JSON.parse(body));
      }
    },
  };
}

function collectText(value) {
  if (value == null) return '';
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(collectText).join('\n');
  if (typeof value === 'object') {
    return Object.values(value).map(collectText).join('\n');
  }
  return String(value);
}
