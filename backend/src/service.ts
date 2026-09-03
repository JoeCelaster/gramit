import { countChanges } from './diff.js';
import { AppError } from './errors.js';
import { renderLinkContext, type LinkReader } from './links.js';
import type { Corrector } from './llm/azure.js';
import type { Mode } from './prompt.js';

export interface FixOutcome {
  corrected: string;
  changed: boolean;
  changes: number;
  model: string;
  latency_ms: number;
}


export interface FixService {
  fix(text: string, mode: Mode): Promise<FixOutcome>;
}

export interface FixServiceOptions {
  /** null when Azure isn't configured — every call then fails with NO_API_KEY. */
  corrector: Corrector | null;
  maxChars: number;
  missingAzureVars?: string[];
  now?: () => number;
  /** Reads the pages a write instruction links to. null disables the feature. */
  links?: LinkReader | null;
}

export function createFixService(options: FixServiceOptions): FixService {
  const { corrector, maxChars, missingAzureVars = [], now = () => Date.now(), links = null } = options;

  return {
    async fix(text: string, mode: Mode): Promise<FixOutcome> {
      if (text.trim() === '') throw AppError.emptyText();
      if (text.length > maxChars) throw AppError.tooLong(text.length, maxChars);
      if (!corrector) {
        throw AppError.noApiKey(
          `Azure OpenAI is not configured on the backend. Missing: ${missingAzureVars.join(', ') || 'unknown'}.`,
        );
      }

      const started = now();

      // Only write mode uses a link. The other modes transform the selection in front
      // of them, so fetching a URL they happen to contain would cost a round trip and
      // leak where the user's text points, for nothing — and in prompt mode the URL is
      // context for whichever model the rewritten prompt is sent to, not for this one.
      const context =
        mode === 'write' && links ? renderLinkContext(await links.read(text)) : null;

      const { corrected, model } = await corrector.fix(text, mode, context ?? undefined);
      const changes = countChanges(text, corrected);

      return {
        corrected,
        changed: corrected !== text,
        changes,
        model,
        latency_ms: now() - started,
      };
    },
  };
}
