/**
 * MemFuse Host Memory Adapter
 */

import type { MemFuseRuntimeClient } from './runtime-client.js';
import { renderContextSections, renderContextText } from './render.js';
import type {
  FinishTurnInput,
  FinishTurnResult,
  HostMemoryAdapterHooks,
  PrepareReadInput,
  PrepareReadResult,
  PreparedTurnResult,
  RenderedContextSection,
  ResolveContextResponse,
  StartTurnInput,
  StartTurnResult,
} from './types.js';

export interface HostMemoryAdapter {
  startTurn(input: StartTurnInput): Promise<StartTurnResult>;
  finishTurn(input: FinishTurnInput): Promise<FinishTurnResult>;
  prepareRead(input: PrepareReadInput): Promise<PrepareReadResult>;
}

export class BaseHostMemoryAdapter implements HostMemoryAdapter {
  protected readonly client: MemFuseRuntimeClient;
  private readonly hooks: HostMemoryAdapterHooks;

  constructor(client: MemFuseRuntimeClient, hooks: HostMemoryAdapterHooks = {}) {
    this.client = client;
    this.hooks = hooks;
  }

  async startTurn(input: StartTurnInput): Promise<StartTurnResult> {
    await this.hooks.onBeforeAppendUserTurn?.(input);
    const userTurn = await this.client.appendUserTurn(input.threadId, {
      content: input.userMessage,
      metadata: { resource_id: input.resourceId },
    }, input.appendUserTurnOptions);
    await this.hooks.onAfterAppendUserTurn?.(userTurn);

    const context = await this.client.resolveContext({
      user_id: input.userId,
      session_id: input.threadId,
      resource_id: input.resourceId,
      query: input.queryText,
      token_budget: input.budget,
    }, input.resolveContextOptions);
    await this.hooks.onAfterResolveContext?.(context);

    return { userTurn, context };
  }

  async finishTurn(input: FinishTurnInput): Promise<FinishTurnResult> {
    await this.hooks.onBeforeAppendAssistantTurn?.(input);
    const assistantTurn = await this.client.appendAssistantTurn(input.threadId, {
      content: input.assistantMessage,
      metadata: { resource_id: input.resourceId },
    }, input.appendAssistantTurnOptions);
    await this.hooks.onAfterAppendAssistantTurn?.(assistantTurn);

    return { assistantTurn };
  }

  async prepareTurn(input: StartTurnInput): Promise<PreparedTurnResult> {
    const started = await this.startTurn(input);
    return {
      ...started,
      renderedSections: this.renderContextSections(started.context),
      renderedText: this.renderContextText(started.context),
    };
  }

  async prepareRead(input: PrepareReadInput): Promise<PrepareReadResult> {
    const [search, facts] = await Promise.all([
      this.client.searchMemories({
        user_id: input.userId,
        query: input.filePath,
        limit: input.limit ?? 3,
      }, input.searchOptions),
      this.client.listFacts(input.userId, input.factsOptions),
    ]);

    const relatedFacts = facts.filter(fact => {
      const value = fact.display_value || '';
      // Match using path segments: split both into meaningful parts
      // and require at least one non-trivial segment overlap
      const fileSegments = input.filePath.split(/[/\\]/).filter(s => s.length > 2);
      const valueSegments = value.split(/[/\\:\s.]/).filter(s => s.length > 2);
      return fileSegments.some(seg => valueSegments.some(vSeg => vSeg === seg))
        || value.includes(input.filePath);
    }).slice(0, 3);

    return {
      filePath: input.filePath,
      relatedEpisodes: search.results,
      relatedFacts,
      renderedText: this.renderReadHint(input.filePath, search.results, relatedFacts),
    };
  }

  renderContextSections(context: ResolveContextResponse): RenderedContextSection[] {
    return renderContextSections(context);
  }

  renderContextText(context: ResolveContextResponse): string {
    return renderContextText(context);
  }

  protected renderReadHint(
    filePath: string,
    episodes: PrepareReadResult['relatedEpisodes'],
    facts: PrepareReadResult['relatedFacts'],
  ): string {
    const lines: string[] = [];

    if (episodes.length > 0) {
      lines.push(`MemFuse read hint for ${filePath}:`);
      for (const episode of episodes) {
        lines.push(`- ${episode.summary}${episode.episode_id ? ` [episode=${episode.episode_id}]` : ''}`);
      }
    }

    if (facts.length > 0) {
      if (lines.length === 0) {
        lines.push(`MemFuse read hint for ${filePath}:`);
      }
      for (const fact of facts) {
        const marker = fact.confidence >= 0.8 ? '✓' : '~';
        lines.push(`- ${marker} ${fact.display_value} [${fact.predicate}]`);
      }
    }

    if (lines.length === 0) {
      return '';
    }

    lines.push('Use get_observations or timeline for more detail.');
    return lines.join('\n');
  }
}
