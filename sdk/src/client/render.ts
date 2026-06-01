/**
 * MemFuse Context Rendering
 */

import type {
  RenderedContextSection,
  ResolveContextResponse,
} from './types.js';

export function renderContextSections(
  context: ResolveContextResponse,
): RenderedContextSection[] {
  const sections: RenderedContextSection[] = [];

  if (context.sections.current_facts?.length) {
    sections.push({
      id: 'current_facts',
      title: 'Current Facts',
      lines: context.sections.current_facts.map(fact => fact.display_value),
    });
  }

  if (context.sections.recent_updates?.length) {
    sections.push({
      id: 'recent_updates',
      title: 'Recent Updates',
      lines: context.sections.recent_updates.map(update => `${update.role}: ${update.content}`),
    });
  }

  if (context.sections.relevant_history?.length) {
    sections.push({
      id: 'relevant_history',
      title: 'Relevant History',
      lines: context.sections.relevant_history.map(item => item.summary),
    });
  }

  return sections.filter(section => section.lines.length > 0);
}

export function renderContextText(context: ResolveContextResponse): string {
  return renderContextSections(context)
    .map(section => [`[${section.title}]`, ...section.lines.map(line => `- ${line}`)].join('\n'))
    .join('\n\n');
}