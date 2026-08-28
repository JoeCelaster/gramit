// The deployment entrypoint. Vercel scans this file for an express import to decide it
// is a Node server, so the instance is created here rather than inside createApp.
import express from 'express';
import { loadConfigFromDotenv } from './config.js';
import { createApp } from './create-app.js';
import { createLinkReader } from './links.js';
import { createAzureCorrector } from './llm/azure.js';
import { log } from './logger.js';
import { VERSION } from './routes/health.js';
import { createFixService } from './service.js';

const config = loadConfigFromDotenv();

if (!config.azure) {
  // Loud, but not fatal: the server still binds so the daemon gets a clean NO_API_KEY
  // response and `gramit doctor` can say exactly what to fix.
  log.error('Azure OpenAI is not configured — /v1/fix will fail with NO_API_KEY', {
    missing: config.missingAzureVars.join(', '),
    hint: 'copy backend/.env.example to backend/.env and fill it in (or set the vars in the Vercel project)',
  });
}

const corrector = config.azure ? createAzureCorrector(config.azure, config.upstreamTimeoutMs) : null;

// Write mode reads the pages an instruction links to. A page that will not load is
// logged and dropped, never raised: the user asked for writing, not for a fetch.
const links = config.links.enabled
  ? createLinkReader(config.links, undefined, (url, reason) =>
      log.warn('could not read a linked page', { url, reason }),
    )
  : null;

const service = createFixService({
  corrector,
  maxChars: config.maxChars,
  missingAzureVars: config.missingAzureVars,
  links,
});

const app = createApp({ config, service }, express());

const server = app.listen(config.port, config.host, () => {
  log.info('gramit backend listening', {
    version: VERSION,
    url: `http://${config.host}:${config.port}`,
    model: config.azure?.deployment ?? 'none',
  });
});

for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, () => {
    log.info('shutting down', { signal });
    server.close(() => process.exit(0));
  });
}
