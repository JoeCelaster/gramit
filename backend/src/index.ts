import { createApp } from './app.js';
import { loadConfigFromDotenv } from './config.js';
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
    hint: 'copy backend/.env.example to backend/.env and fill it in',
  });
}

const corrector = config.azure ? createAzureCorrector(config.azure, config.upstreamTimeoutMs) : null;

const service = createFixService({
  corrector,
  maxChars: config.maxChars,
  missingAzureVars: config.missingAzureVars,
});

const app = createApp({ config, service });

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
