import { bootstrap } from './bootstrap.js';
import { log } from './logger.js';
import { VERSION } from './routes/health.js';

const { app, config } = bootstrap();

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
