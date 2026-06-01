import {
  CliArgs, RegisterFn, call, output, optStr, optBool,
} from '../types.js';
import { runSetup } from '../../setup/install.js';
import {
  formatSystemStatus, formatObserverStatus, formatHealth,
} from '../output.js';
import {
  installService,
  renderDoctor,
  renderServiceStatus,
  runServiceAction,
} from '../../service/manager.js';

export function registerSystemCommands(register: RegisterFn): void {
  register('system-status', async (args) => {
    const result = await call('GET', '/system/status', null, args) as Record<string, unknown>;
    output(formatSystemStatus(result, args.mode), args);
  });

  register('observer-status', async (args) => {
    const result = await call('GET', '/system/observer', null, args) as Record<string, unknown>;
    output(formatObserverStatus(result, args.mode), args);
  });

  register('health', async (args) => {
    const result = await call('GET', '/health', null, args) as Record<string, unknown>;
    output(formatHealth(result, args.config.serverUrl, args.mode), args);
  });

  register('ready', async (args) => {
    const result = await call('GET', '/ready', null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const status = String(result.status ?? '');
    const checks = result.checks as Record<string, unknown> | undefined;
    const lines = [`Status: ${status}`];
    if (checks) {
      for (const [k, v] of Object.entries(checks)) {
        const detail = typeof v === 'object' ? JSON.stringify(v) : String(v);
        lines.push(`  ${k}: ${detail}`);
      }
    }
    output(lines.join('\n'), args);
  });

  register('metrics', async (args) => {
    const result = await call('GET', '/metrics', null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    // Server returns Prometheus text format wrapped as { raw: "..." }
    if (typeof result === 'string') { output(result, args); return; }
    const raw = result.raw;
    if (typeof raw === 'string') { output(raw, args); return; }
    const lines = Object.entries(result).map(([k, v]) => `${k}: ${typeof v === 'object' ? JSON.stringify(v) : v}`);
    output(lines.join('\n'), args);
  });

  register('install', async (args) => {
    const skills = optBool(args, 'skills');
    const hooks = optBool(args, 'hooks');
    const noMcp = optBool(args, 'no-mcp');
    const platform = optStr(args, 'platform') || 'both';
    const projectDir = optStr(args, 'project-dir') || process.cwd();

    const setupArgs: string[] = [];
    setupArgs.push(`--platform=${platform}`);
    setupArgs.push(`--project-dir=${projectDir}`);
    if (args.config.userId !== 'default') setupArgs.push(`--user-id=${args.config.userId}`);
    setupArgs.push(`--server-url=${args.config.serverUrl}`);

    if (noMcp) {
      process.env['MEMFUSE_SKIP_MCP'] = '1';
    }

    if (skills === undefined && hooks === undefined) {
      console.error('Error: install requires at least --skills or --hooks. Usage: memfuse install --skills [--hooks] [--no-mcp]');
      process.exit(1);
    }

    if (skills && !hooks && noMcp) {
      process.env['MEMFUSE_INSTALL_MODE'] = 'skills-only';
    } else if (skills && hooks && noMcp) {
      process.env['MEMFUSE_INSTALL_MODE'] = 'skills-hooks';
    } else if (skills && hooks) {
      process.env['MEMFUSE_INSTALL_MODE'] = 'full';
    } else if (hooks && !skills) {
      process.env['MEMFUSE_INSTALL_MODE'] = 'hooks-only';
    } else {
      process.env['MEMFUSE_INSTALL_MODE'] = 'default';
    }

    await runSetup(setupArgs);
  });

  register('service', async (args) => {
    const action = args.positional[0] || 'status';
    if (action === 'status') {
      output(renderServiceStatus({
        scope: optStr(args, 'scope'),
        serverBin: optStr(args, 'server-bin'),
      }), args);
      return;
    }
    if (action === 'install') {
      output(installService({
        scope: optStr(args, 'scope'),
        serverBin: optStr(args, 'server-bin'),
      }), args);
      return;
    }
    if (action === 'doctor') {
      output(await renderDoctor({
        scope: optStr(args, 'scope'),
        serverBin: optStr(args, 'server-bin'),
      }), args);
      return;
    }
    if (['start', 'stop', 'restart', 'logs', 'uninstall'].includes(action)) {
      try {
        output(runServiceAction(action as 'start' | 'stop' | 'restart' | 'logs' | 'uninstall', {
          scope: optStr(args, 'scope'),
          dryRun: optBool(args, 'dry-run'),
          serverBin: optStr(args, 'server-bin'),
        }), args);
      } catch (err) {
        console.error(`Error: ${err instanceof Error ? err.message : String(err)}`);
        process.exit(1);
      }
      return;
    }
    console.error(
      `Error: unknown service action '${action}'. Use: status, install, start, stop, restart, logs, doctor, uninstall.`,
    );
    process.exit(1);
  });
}
