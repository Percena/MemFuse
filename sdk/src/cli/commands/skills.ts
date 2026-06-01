import {
  RegisterFn, call, output, requirePositional,
} from '../types.js';
import {
  formatSkillsList, formatSkillAdded,
} from '../output.js';

export function registerSkillCommands(register: RegisterFn): void {
  register('skills-list', async (args) => {
    const result = await call('GET', '/skills', null, args);
    output(formatSkillsList(result, args.mode), args);
  });

  register('add-skill', async (args) => {
    const path = requirePositional(args, 'path', 0);
    const result = await call('POST', '/skills', { path }, args) as Record<string, unknown>;
    output(formatSkillAdded(result, args.mode), args);
  });
}
