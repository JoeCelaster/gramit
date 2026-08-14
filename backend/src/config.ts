import { config as loadDotenv } from 'dotenv';

export interface AzureConfig {
  endpoint: string;
  apiKey: string;
  deployment: string;
  apiVersion: string;
  /** Optional fixed sampling seed. Unset means the model samples freshly each call.
   *  This deployment rejects `temperature` and `top_p`, so a seed is the only way to
   *  make the same text correct the same way twice. */
  seed: number | null;
}

export interface Config {
  host: string;
  port: number;
  /** null when the Azure env vars are incomplete — the server still starts so
   *  `gramit doctor` gets a clean NO_API_KEY instead of a connection refused. */
  azure: AzureConfig | null;
  /** Names of the Azure env vars that were missing, for the startup log and /health. */
  missingAzureVars: string[];
  maxChars: number;
  upstreamTimeoutMs: number;
}

const AZURE_VARS = [
  'AZURE_OPENAI_ENDPOINT',
  'AZURE_OPENAI_API_KEY',
  'AZURE_OPENAI_DEPLOYMENT',
  'AZURE_OPENAI_API_VERSION',
] as const;

function intFromEnv(name: string, fallback: number): number {
  const raw = process.env[name];
  if (raw === undefined || raw.trim() === '') return fallback;
  const parsed = Number.parseInt(raw, 10);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    throw new Error(`${name} must be a positive integer, got ${JSON.stringify(raw)}`);
  }
  return parsed;
}

export function loadConfig(env: NodeJS.ProcessEnv = process.env): Config {
  const missingAzureVars = AZURE_VARS.filter((name) => !env[name]?.trim());

  const azure: AzureConfig | null =
    missingAzureVars.length === 0
      ? {
          // Trailing slashes are a common copy-paste artifact and break URL joining.
          endpoint: env.AZURE_OPENAI_ENDPOINT!.trim().replace(/\/+$/, ''),
          apiKey: env.AZURE_OPENAI_API_KEY!.trim(),
          deployment: env.AZURE_OPENAI_DEPLOYMENT!.trim(),
          apiVersion: env.AZURE_OPENAI_API_VERSION!.trim(),
          seed: env.AZURE_OPENAI_SEED?.trim() ? intFromEnv('AZURE_OPENAI_SEED', 0) : null,
        }
      : null;

  return {
    host: env.HOST?.trim() || '127.0.0.1',
    port: intFromEnv('PORT', 8787),
    azure,
    missingAzureVars: [...missingAzureVars],
    maxChars: intFromEnv('MAX_CHARS', 25_000),
    upstreamTimeoutMs: intFromEnv('UPSTREAM_TIMEOUT_MS', 30_000),
  };
}

/** Reads `.env` into process.env, then builds the config. Call once at startup. */
export function loadConfigFromDotenv(): Config {
  loadDotenv({ quiet: true });
  return loadConfig();
}
