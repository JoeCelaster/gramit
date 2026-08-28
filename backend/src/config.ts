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

export interface LinkConfig {
  /** Off means write mode ignores URLs entirely — no outbound request is ever made. */
  enabled: boolean;
  timeoutMs: number;
  maxBytes: number;
  maxChars: number;
  maxLinks: number;
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
  /** How write mode reads the pages an instruction links to. */
  links: LinkConfig;
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
    // Loopback everywhere by default: this process holds the API key and must not be
    // reachable from the LAN. Vercel is the exception — it runs the app as a real server
    // and proxies to it, so binding loopback there makes every request time out.
    host: env.HOST?.trim() || (env.VERCEL ? '0.0.0.0' : '127.0.0.1'),
    port: intFromEnv('PORT', 8787),
    azure,
    missingAzureVars: [...missingAzureVars],
    maxChars: intFromEnv('MAX_CHARS', 25_000),
    upstreamTimeoutMs: intFromEnv('UPSTREAM_TIMEOUT_MS', 30_000),
    links: {
      // On by default: a link in a write instruction is there to be used, and a fix
      // that silently ignored it would invent what the page said instead.
      enabled: (env.LINK_FETCH ?? 'on').trim().toLowerCase() !== 'off',
      // Short, because it is spent before the model is even called and the user is
      // watching a hotkey. A slow page is dropped rather than allowed to add seconds.
      timeoutMs: intFromEnv('LINK_TIMEOUT_MS', 6_000),
      maxBytes: intFromEnv('LINK_MAX_BYTES', 1_500_000),
      maxChars: intFromEnv('LINK_MAX_CHARS', 6_000),
      maxLinks: intFromEnv('LINK_MAX_LINKS', 3),
    },
  };
}

/** Reads `.env` into process.env, then builds the config. Call once at startup. */
export function loadConfigFromDotenv(): Config {
  loadDotenv({ quiet: true });
  return loadConfig();
}
