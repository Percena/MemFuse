import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optNum, optStr,
} from '../types.js';
import {
  formatWatchList, formatWatchOp, formatWatchDaemonStatus,
} from '../output.js';

export function registerWatchCommands(register: RegisterFn): void {
  register('watches-list', async (args) => {
    const result = await call('GET', '/watches', null, args);
    output(formatWatchList(result, args.mode), args);
  });

  register('resource-watch', async (args) => {
    const resourceId = requirePositional(args, 'resource-id', 0);
    const interval = optNum(args, 'interval') ?? 300;
    const result = await call('POST',
      `/resources/${encodeURIComponent(resourceId)}/watch`,
      { interval_seconds: interval }, args) as Record<string, unknown>;
    output(formatWatchOp('Watch registered', resourceId, result, args.mode), args);
  });

  register('resource-watch-disable', async (args) => {
    const resourceId = requirePositional(args, 'resource-id', 0);
    const result = await call('POST',
      `/resources/${encodeURIComponent(resourceId)}/watch/disable`,
      null, args) as Record<string, unknown>;
    output(formatWatchOp('Watch disabled', resourceId, result, args.mode), args);
  });

  register('resource-watch-run', async (args) => {
    const resourceId = requirePositional(args, 'resource-id', 0);
    const result = await call('POST',
      `/resources/${encodeURIComponent(resourceId)}/watch/run`,
      null, args) as Record<string, unknown>;
    output(formatWatchOp('Watch run', resourceId, result, args.mode), args);
  });

  register('watch-run-due', async (args) => {
    const result = await call('POST', '/watches/run-due', null, args) as Record<string, unknown>;
    output(formatWatchOp('Run-due', '', result, args.mode), args);
  });

  register('watch-run-loop', async (args) => {
    const iterations = optNum(args, 'iterations') ?? 1;
    const sleepMs = optNum(args, 'sleep-ms') ?? 5000;
    const result = await call('POST', '/watches/run-loop',
      { iterations, sleep_ms: sleepMs }, args) as Record<string, unknown>;
    output(formatWatchOp('Run-loop', '', result, args.mode), args);
  });

  register('watch-daemon-start', async (args) => {
    const pollMs = optNum(args, 'poll-ms') ?? 30000;
    const result = await call('POST', '/watch-service/start',
      { poll_ms: pollMs }, args) as Record<string, unknown>;
    output(formatWatchDaemonStatus('start', result, args.mode), args);
  });

  register('watch-daemon-status', async (args) => {
    const result = await call('GET', '/watch-service/status', null, args) as Record<string, unknown>;
    output(formatWatchDaemonStatus('status', result, args.mode), args);
  });

  register('watch-daemon-stop', async (args) => {
    const result = await call('POST', '/watch-service/stop', null, args) as Record<string, unknown>;
    output(formatWatchDaemonStatus('stop', result, args.mode), args);
  });
}