import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join } from 'node:path';
import { DEFAULT_BIND_ADDR, DEFAULT_SERVER_URL, loadConfig, memfuseHome } from '../shared/config.js';
import { httpRequest } from '../shared/http.js';

export type ServicePlatform = 'darwin' | 'linux' | 'manual';
export type ServiceScope = 'user' | 'system';
export type ServiceAction = 'start' | 'stop' | 'restart' | 'logs' | 'uninstall';

export interface ServiceOptions {
  scope?: string;
  dryRun?: boolean;
  serverBin?: string;
}

export interface ServiceLayout {
  platform: ServicePlatform;
  scope: ServiceScope;
  servicePath: string;
  configPath: string;
  dataDir: string;
  logDir: string;
  serverBin: string;
}

export function detectServicePlatform(): ServicePlatform {
  const forced = process.env.MEMFUSE_SERVICE_PLATFORM;
  if (forced === 'darwin' || forced === 'linux' || forced === 'manual') return forced;
  if (process.platform === 'darwin') return 'darwin';
  if (process.platform === 'linux') return 'linux';
  return 'manual';
}

export function serviceLayout(options: ServiceOptions = {}): ServiceLayout {
  const scope = normalizeScope(options.scope);
  const platform = detectServicePlatform();
  const home = process.env.HOME || homedir();
  const mfHome = memfuseHome();
  const configHome = process.env.XDG_CONFIG_HOME || join(home, '.config');
  const userConfigPath = join(configHome, 'memfuse', 'config.toml');
  const serverBin = options.serverBin || process.env.MEMFUSE_SERVER_BIN || 'memfuse-server';

  if (platform === 'darwin') {
    return {
      platform,
      scope,
      servicePath: join(home, 'Library', 'LaunchAgents', 'io.memfuse.server.plist'),
      configPath: join(mfHome, 'config.toml'),
      dataDir: join(mfHome, 'data'),
      logDir: join(mfHome, 'logs'),
      serverBin,
    };
  }

  if (scope === 'system') {
    return {
      platform,
      scope,
      servicePath: '/etc/systemd/system/memfuse.service',
      configPath: '/etc/memfuse/config.toml',
      dataDir: '/var/lib/memfuse',
      logDir: '/var/log/memfuse',
      serverBin,
    };
  }

  return {
    platform,
    scope,
    servicePath: join(configHome, 'systemd', 'user', 'memfuse.service'),
    configPath: userConfigPath,
    dataDir: join(mfHome, 'data'),
    logDir: join(mfHome, 'logs'),
    serverBin,
  };
}

export function installService(options: ServiceOptions = {}): string {
  const layout = serviceLayout(options);
  if (layout.platform === 'manual') {
    return 'MemFuse service install\n  supervisor: manual\n  status: native service install is not available for this platform yet';
  }

  mkdirSync(dirname(layout.servicePath), { recursive: true });
  mkdirSync(dirname(layout.configPath), { recursive: true });
  mkdirSync(layout.dataDir, { recursive: true });
  mkdirSync(layout.logDir, { recursive: true });
  if (!existsSync(layout.configPath)) {
    writeFileSync(layout.configPath, renderDefaultConfig(layout), 'utf-8');
  }
  writeFileSync(
    layout.servicePath,
    layout.platform === 'darwin' ? renderLaunchAgent(layout) : renderSystemdService(layout),
    'utf-8',
  );

  return [
    'MemFuse service installed',
    `  supervisor: ${supervisorName(layout.platform)}`,
    `  service: ${layout.servicePath}`,
    `  config: ${layout.configPath}`,
    `  data_dir: ${layout.dataDir}`,
    `  next: ${installNextCommand(layout)}`,
  ].join('\n');
}

export function renderSystemdService(layout: ServiceLayout): string {
  const userLines = layout.scope === 'system' ? 'User=memfuse\nGroup=memfuse\n' : '';
  const installTarget = layout.scope === 'system' ? 'multi-user.target' : 'default.target';
  return `[Unit]
Description=MemFuse local memory service
After=network-online.target

[Service]
Type=simple
${userLines}ExecStart=${layout.serverBin} --config ${layout.configPath}
Restart=on-failure
RestartSec=3
WorkingDirectory=${layout.scope === 'system' ? layout.dataDir : process.env.HOME || homedir()}
Environment=RUST_LOG=mfs_server=info
TimeoutStopSec=35

[Install]
WantedBy=${installTarget}
`;
}

export function renderLaunchAgent(layout: ServiceLayout): string {
  const stdout = join(layout.logDir, 'server.log');
  const stderr = join(layout.logDir, 'server.err.log');
  return `<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>io.memfuse.server</string>
  <key>ProgramArguments</key>
  <array>
    <string>${escapeXml(layout.serverBin)}</string>
    <string>--config</string>
    <string>${escapeXml(layout.configPath)}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${escapeXml(stdout)}</string>
  <key>StandardErrorPath</key>
  <string>${escapeXml(stderr)}</string>
</dict>
</plist>
`;
}

export function renderDefaultConfig(layout: ServiceLayout): string {
  const serverUrl = DEFAULT_SERVER_URL;
  return `[server]
profile = "development"
bind_addr = "${DEFAULT_BIND_ADDR}"
auth_mode = "dev"
allow_insecure_bind = false
shutdown_timeout_ms = 30000

[storage]
data_dir = "${escapeToml(layout.dataDir)}"
source_kind = "managed"
target_uri = "mfs://resources/localfs/docs"
canvas_separate_db = false

[identity]
account_id = "default"
user_id = "default"
agent_id = "default"

[client]
server_url = "${serverUrl}"
cloud_url = "${serverUrl}"
local_canvas_url = "${serverUrl}"
auth_token = ""
`;
}

export function renderServiceStatus(options: ServiceOptions = {}): string {
  const layout = serviceLayout(options);
  const configPath = effectiveConfigPath(layout);
  const command = serviceCommand('status', layout);
  const serverUrl = loadConfig(configPath).serverUrl;
  return [
    'MemFuse service',
    `  supervisor: ${supervisorName(layout.platform)}`,
    `  service: ${layout.servicePath}`,
    `  config: ${configPath}`,
    `  server_url: ${serverUrl}`,
    `  status: use \`${command.command} ${command.args.join(' ')}\``,
  ].join('\n');
}

export function runServiceAction(action: ServiceAction, options: ServiceOptions = {}): string {
  const layout = serviceLayout(options);
  if (layout.platform === 'manual') {
    return `MemFuse service ${action}\n  supervisor: manual\n  command: start memfuse-server manually with --config`;
  }
  const command = serviceCommand(action, layout);
  if (options.dryRun || process.env.MEMFUSE_SERVICE_DRY_RUN === '1') {
    return `MemFuse service ${action}\n  supervisor: ${supervisorName(layout.platform)}\n  command: ${command.command} ${command.args.join(' ')}`;
  }
  const result = spawnSync(command.command, command.args, { encoding: 'utf-8' });
  const output = [result.stdout, result.stderr].filter(Boolean).join('');
  if (result.status && result.status !== 0) {
    throw new Error(output.trim() || `${command.command} exited ${result.status}`);
  }
  if (action === 'uninstall' && existsSync(layout.servicePath)) {
    rmSync(layout.servicePath);
  }
  return output.trim() || `MemFuse service ${action} executed`;
}

export async function renderDoctor(options: ServiceOptions = {}): Promise<string> {
  const layout = serviceLayout(options);
  const configPath = effectiveConfigPath(layout);
  const config = loadConfig(configPath);
  const serverUrl = config.serverUrl;
  const dataDir = config.dataDir || layout.dataDir;
  const binaryOk = layout.serverBin === 'memfuse-server' ? 'unknown' : existsSync(layout.serverBin) ? 'ok' : 'missing';
  const configOk = existsSync(configPath) ? 'ok' : 'missing';
  const dataOk = existsSync(dataDir) ? 'ok' : 'missing';
  let ready = 'offline';
  try {
    const response = await httpRequest(serverUrl, 'GET', '/ready', null);
    ready = response.statusCode >= 200 && response.statusCode < 400 ? 'ok' : `http-${response.statusCode}`;
  } catch (_) {
    ready = 'offline';
  }
  return [
    'MemFuse service doctor',
    `  supervisor: ${supervisorName(layout.platform)}`,
    `  binary: ${binaryOk} (${layout.serverBin})`,
    `  config: ${configOk} (${configPath})`,
    `  data_dir: ${dataOk} (${dataDir})`,
    `  ready: ${ready} (${serverUrl}/ready)`,
  ].join('\n');
}

function serviceCommand(action: ServiceAction | 'status', layout: ServiceLayout): { command: string; args: string[] } {
  if (layout.platform === 'darwin') {
    const domain = `gui/${typeof process.getuid === 'function' ? process.getuid() : '$UID'}`;
    const label = `${domain}/io.memfuse.server`;
    if (action === 'start') return { command: 'launchctl', args: ['bootstrap', domain, layout.servicePath] };
    if (action === 'stop' || action === 'uninstall') return { command: 'launchctl', args: ['bootout', label] };
    if (action === 'restart') return { command: 'launchctl', args: ['kickstart', '-k', label] };
    if (action === 'logs') return { command: 'tail', args: ['-n', '100', join(layout.logDir, 'server.log')] };
    return { command: 'launchctl', args: ['print', label] };
  }
  const systemctl = layout.scope === 'system' ? ['systemctl'] : ['systemctl', '--user'];
  if (action === 'logs') {
    const args = layout.scope === 'system'
      ? ['-u', 'memfuse.service', '-n', '100', '--no-pager']
      : ['--user', '-u', 'memfuse.service', '-n', '100', '--no-pager'];
    return { command: 'journalctl', args };
  }
  if (action === 'uninstall') return { command: systemctl[0], args: [...systemctl.slice(1), 'disable', '--now', 'memfuse.service'] };
  if (action === 'status') return { command: systemctl[0], args: [...systemctl.slice(1), 'status', 'memfuse.service', '--no-pager'] };
  return { command: systemctl[0], args: [...systemctl.slice(1), action, 'memfuse.service'] };
}

function normalizeScope(scope?: string): ServiceScope {
  return scope === 'system' ? 'system' : 'user';
}

function effectiveConfigPath(layout: ServiceLayout): string {
  return process.env.MEMFUSE_CONFIG || layout.configPath;
}

function installNextCommand(layout: ServiceLayout): string {
  if (layout.platform === 'linux') {
    return layout.scope === 'system'
      ? 'systemctl daemon-reload && memfuse service start --scope system'
      : 'systemctl --user daemon-reload && memfuse service start';
  }
  return 'memfuse service start';
}

function supervisorName(platform: ServicePlatform): string {
  if (platform === 'darwin') return 'launchd';
  if (platform === 'linux') return 'systemd';
  return 'manual';
}

function escapeToml(value: string): string {
  return value.replace(/\\/g, '\\\\').replace(/"/g, '\\"');
}

function escapeXml(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
