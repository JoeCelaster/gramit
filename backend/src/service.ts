import { countChanges } from './diff.js';
import { AppError } from './errors.js';
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
}

export function createFixService(options: FixServiceOptions): FixService {
  const { corrector, maxChars, missingAzureVars = [], now = () => Date.now() } = options;

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

      const { corrected, model } = await corrector.fix(text, mode);
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
