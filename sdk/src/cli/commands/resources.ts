import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum, optBool,
} from '../types.js';
import {
  formatResourceAdded, formatResourceList, formatResourceOp,
  formatTaskStatus, formatTaskList,
} from '../output.js';

export function registerResourceCommands(register: RegisterFn): void {
  register('add-resource', async (args) => {
    const sourceKind = optStr(args, 'source-kind');
    const sourcePath = optStr(args, 'source-path');
    const url = optStr(args, 'url');
    const logicalName = optStr(args, 'logical-name');
    const branch = optStr(args, 'branch');
    const revision = optStr(args, 'revision');
    const fileName = optStr(args, 'file-name');
    const content = optStr(args, 'content');

    let request: Record<string, unknown>;

    if (fileName && content) {
      request = { file_name: fileName, content, logical_name: logicalName };
    } else if (sourceKind === 'git_url') {
      const gitUrl = url || sourcePath;
      if (!gitUrl) {
        console.error('Error: git_url requires --url or --source-path with a URL.');
        process.exit(1);
      }
      request = { source_kind: 'git_url', source_path: gitUrl, logical_name: logicalName, branch, revision };
    } else if (sourceKind === 'localfs' || sourceKind === 'git') {
      if (!sourcePath) {
        console.error(`Error: ${sourceKind} requires --source-path.`);
        process.exit(1);
      }
      request = { source_kind: sourceKind, source_path: sourcePath, logical_name: logicalName, branch, revision };
    } else {
      console.error('Error: add-resource requires either --file-name + --content (inline) or --source-kind + --source-path.');
      process.exit(1);
    }

    const result = await call('POST', '/resources', request, args) as Record<string, unknown>;
    output(formatResourceAdded(result, args.mode), args);
  });

  register('add-repo', async (args) => {
    const pathOrUrl = requirePositional(args, 'path-or-url', 0);
    const logicalName = optStr(args, 'logical-name');
    const branch = optStr(args, 'branch');
    const revision = optStr(args, 'revision');

    const isUrl = /^(https?:\/\/|git@|ssh:\/\/)/.test(pathOrUrl);
    const request = isUrl
      ? { source_kind: 'git_url', source_path: pathOrUrl, logical_name: logicalName, branch, revision }
      : { source_kind: 'git', source_path: pathOrUrl, logical_name: logicalName, branch, revision };

    const result = await call('POST', '/resources', request, args) as Record<string, unknown>;
    output(formatResourceAdded(result, args.mode), args);
  });

  register('add-inline', async (args) => {
    const fileName = requirePositional(args, 'file-name', 0);
    const content = requirePositional(args, 'content', 1);
    const logicalName = optStr(args, 'logical-name');

    const result = await call('POST', '/resources', {
      file_name: fileName, content, logical_name: logicalName,
    }, args) as Record<string, unknown>;
    output(formatResourceAdded(result, args.mode), args);
  });

  register('resources-list', async (args) => {
    const result = await call('GET', '/resources', null, args);
    output(formatResourceList(result, args.mode), args);
  });

  register('resource-refresh', async (args) => {
    const resourceId = requirePositional(args, 'resource-id', 0);
    const result = await call('POST', `/resources/${encodeURIComponent(resourceId)}/refresh`, null, args) as Record<string, unknown>;
    output(formatResourceOp('Refresh', result, args.mode), args);
  });

  register('resource-rebuild', async (args) => {
    const resourceId = requirePositional(args, 'resource-id', 0);
    const result = await call('POST', `/resources/${encodeURIComponent(resourceId)}/rebuild`, null, args) as Record<string, unknown>;
    output(formatResourceOp('Rebuild', result, args.mode), args);
  });

  register('task-status', async (args) => {
    const taskKey = requirePositional(args, 'task-key', 0);
    output(formatTaskStatus(await call('GET', `/tasks/${encodeURIComponent(taskKey)}`, null, args) as Record<string, unknown>, args.mode), args);
  });

  register('tasks-list', async (args) => {
    const limit = optNum(args, 'limit') ?? 20;
    const result = await call('GET', `/tasks?limit=${limit}`, null, args);
    output(formatTaskList(result, args.mode), args);
  });
}