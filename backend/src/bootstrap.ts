import type { Express } from 'express';
import { createApp } from './app.js';
import { loadConfigFromDotenv, type Config } from './config.js';
import { createAzureCorrector } from './llm/azure.js';
import { log } from './logger.js';
import { createFixService } from './service.js';

export interface Bootstrapped {
  app: Express;
  config: Config;
}

/** Wires config → corrector → service → app without binding a port, so the long-lived
 *  server (`index.ts`) and the serverless handler (`api/index.js`) share one code path.
 *  A serverless function is handed a request, not a port, so `listen` has to stay out
 *  of here — calling it is what makes a Vercel deployment fail to invoke. */
export function bootstrap(): Bootstrapped {
  const config = loadConfigFromDotenv();

  if (!config.azure) {
    // Loud, but not fatal: the app still answers so the daemon gets a clean NO_API_KEY
    // response and `gramit doctor` can say exactly what to fix.
    log.error('Azure OpenAI is not configured — /v1/fix will fail with NO_API_KEY', {
      missing: config.missingAzureVars.join(', '),
      hint: 'copy backend/.env.example to backend/.env and fill it in (or set the vars in the Vercel project)',
    });
  }

  const corrector = config.azure ? createAzureCorrector(config.azure, config.upstreamTimeoutMs) : null;

  const service = createFixService({
    corrector,
    maxChars: config.maxChars,
    missingAzureVars: config.missingAzureVars,
  });

  return { app: createApp({ config, service }), config };
}
