import { APIConnectionTimeoutError, APIError, AzureOpenAI } from 'openai';
import type { AzureConfig } from '../config.js';
import { AppError } from '../errors.js';
import { log } from '../logger.js';
import { sanitizeCorrection, systemPrompt, type Mode } from '../prompt.js';

export interface CorrectionResult {
  corrected: string;
  model: string;
}

export interface Corrector {
  fix(text: string, mode: Mode): Promise<CorrectionResult>;
}

/**
 * We don't know which parameters the target deployment accepts — `gpt-5.6-luna` is a
 * custom Azure deployment name, and newer models reject `temperature` or JSON mode
 * outright. So we walk down a ladder of request shapes, remembering the first one the
 * deployment accepts and using it for every later call.
 *
 * Note that no max-token cap is sent: a correction is roughly as long as its input, and
 * a cap risks truncating the user's text mid-paste (and the parameter's name differs
 * across model generations).
 */
interface Strategy {
  jsonMode: boolean;
  temperature: boolean;
}

const STRATEGIES: readonly Strategy[] = [
  { jsonMode: true, temperature: true },
  { jsonMode: true, temperature: false },
  { jsonMode: false, temperature: false },
];

const UNSUPPORTED_PARAM = /unsupported|not supported|unrecognized|unknown|invalid[_ ]?(?:parameter|value|request)|response_format|temperature/i;

function isUnsupportedParamError(err: unknown): boolean {
  if (!(err instanceof APIError)) return false;
  if (err.status !== 400) return false;
  return UNSUPPORTED_PARAM.test(err.message ?? '');
}

function toAppError(err: unknown, timeoutMs: number): AppError {
  if (err instanceof AppError) return err;

  if (err instanceof APIConnectionTimeoutError) return AppError.upstreamTimeout(timeoutMs);

  if (err instanceof APIError) {
    if (err.status === 401 || err.status === 403) {
      return AppError.noApiKey('Azure OpenAI rejected the credentials. Check AZURE_OPENAI_API_KEY.');
    }
    if (err.status === 429) return AppError.rateLimited();
    if (err.status === 404) {
      return AppError.upstream(
        'Azure OpenAI returned 404. Check AZURE_OPENAI_DEPLOYMENT and AZURE_OPENAI_API_VERSION.',
      );
    }
    return AppError.upstream(`Azure OpenAI error ${err.status ?? '?'}: ${err.message}`);
  }

  const message = err instanceof Error ? err.message : String(err);
  if (/timeout|aborted/i.test(message)) return AppError.upstreamTimeout(timeoutMs);
  return AppError.upstream(`Could not reach Azure OpenAI: ${message}`);
}

export function createAzureCorrector(config: AzureConfig, timeoutMs: number): Corrector {
  const client = new AzureOpenAI({
    endpoint: config.endpoint,
    apiKey: config.apiKey,
    apiVersion: config.apiVersion,
    deployment: config.deployment,
    timeout: timeoutMs,
    maxRetries: 1,
  });

  // Index of the first strategy this deployment accepted; sticky across calls.
  let strategyIndex = 0;

  async function callOnce(text: string, mode: Mode, strategy: Strategy): Promise<string> {
    const response = await client.chat.completions.create({
      model: config.deployment,
      messages: [
        { role: 'system', content: systemPrompt(strategy.jsonMode, mode) },
        { role: 'user', content: text },
      ],
      ...(strategy.temperature ? { temperature: 0 } : {}),
      // Only sent when configured; without it the model samples freshly every call.
      ...(config.seed !== null ? { seed: config.seed } : {}),
      ...(strategy.jsonMode ? { response_format: { type: 'json_object' as const } } : {}),
    });

    const content = response.choices[0]?.message?.content;
    if (typeof content !== 'string' || content.trim() === '') {
      const reason = response.choices[0]?.finish_reason ?? 'unknown';
      throw AppError.badUpstreamResponse(`Azure OpenAI returned no text (finish_reason=${reason}).`);
    }
    return content;
  }

  return {
    async fix(text: string, mode: Mode): Promise<CorrectionResult> {
      let lastError: unknown;

      for (let i = strategyIndex; i < STRATEGIES.length; i += 1) {
        const strategy = STRATEGIES[i]!;
        try {
          const raw = await callOnce(text, mode, strategy);
          if (i !== strategyIndex) {
            log.info('adopted fallback request shape for deployment', {
              deployment: config.deployment,
              jsonMode: strategy.jsonMode,
              temperature: strategy.temperature,
            });
            strategyIndex = i;
          }
          return { corrected: sanitizeCorrection(raw, text, mode), model: config.deployment };
        } catch (err) {
          lastError = err;
          if (!isUnsupportedParamError(err) || i === STRATEGIES.length - 1) break;
          log.warn('deployment rejected request shape, trying a simpler one', {
            deployment: config.deployment,
            jsonMode: strategy.jsonMode,
            temperature: strategy.temperature,
            error: err instanceof Error ? err.message : String(err),
          });
        }
      }

      throw toAppError(lastError, timeoutMs);
    },
  };
}
