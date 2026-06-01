import { httpRequest } from '../shared/http.js';
import { OutputMode } from './output.js';
import { MemFuseConfig } from '../shared/config.js';

export class MemFuseHttpError extends Error {
  constructor(public statusCode: number, public method: string, public path: string) {
    super(`Server returned ${statusCode} for ${method} ${path}`);
    this.name = 'MemFuseHttpError';
  }
}

export class MemFuseNetworkError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'MemFuseNetworkError';
  }
}

export interface CliArgs {
  command: string;
  positional: string[];
  options: Record<string, unknown>;
  mode: OutputMode;
  config: MemFuseConfig;
  apiKey?: string;
  sessionExplicit?: boolean;
}

export type RegisterFn = (name: string, handler: (args: CliArgs) => Promise<void>) => void;

export function sessionId(args: CliArgs): string {
  return args.config.sessionId || 'default';
}

export function output(data: string, _args: CliArgs) {
  console.log(data);
}

export function requirePositional(args: CliArgs, name: string, index: number): string {
  const val = args.positional[index];
  if (!val) {
    console.error(`Error: ${args.command} requires ${name}.`);
    console.error(`Usage: memfuse ${args.command} <${name}>`);
    process.exit(1);
  }
  return val;
}

export function optStr(args: CliArgs, key: string): string | undefined {
  const val = args.options[key];
  return typeof val === 'string' ? val : undefined;
}

export function optNum(args: CliArgs, key: string): number | undefined {
  const val = args.options[key];
  if (typeof val === 'string') {
    const n = Number(val);
    return isNaN(n) ? undefined : n;
  }
  return typeof val === 'number' ? val : undefined;
}

export function optBool(args: CliArgs, key: string): boolean | undefined {
  const val = args.options[key];
  if (val === true || val === 'true') return true;
  if (val === false || val === 'false') return false;
  return undefined;
}

export function splitComma(val: string | undefined): string[] {
  if (!val) return [];
  return val.split(',').map(s => s.trim()).filter(s => s.length > 0);
}

export async function call(method: string, path: string, body: unknown, args: CliArgs): Promise<unknown> {
  try {
    const baseUrl = args.config.serverUrl;
    const apiKey = args.apiKey || process.env['MEMFUSE_API_KEY'];

    const result = await httpRequest(baseUrl, method, path, body, apiKey);
    if (result.statusCode >= 400) {
      throw new MemFuseHttpError(result.statusCode, method, path);
    }
    return result.body;
  } catch (err: unknown) {
    if (err instanceof MemFuseHttpError) {
      console.error(`Error: ${err.message}`);
      process.exit(1);
    }
    console.error(`Error: MemFuse server not reachable at ${args.config.serverUrl}. Is the server running?`);
    process.exit(2);
  }
}

export function buildContent(toolName: string, toolInput?: string, toolOutput?: string): string {
  const parts = [`Tool: ${toolName}`];
  if (toolInput) parts.push(`Input:\n${toolInput}`);
  if (toolOutput) parts.push(`Output:\n${toolOutput}`);
  return parts.join('\n');
}
