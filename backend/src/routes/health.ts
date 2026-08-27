import { Router } from 'express';
import type { Config } from '../config.js';

export const VERSION = '0.2.1';

export function healthRouter(config: Config): Router {
  const router = Router();
  const startedAt = Date.now();

  router.get('/health', (_req, res) => {
    res.json({
      ok: true,
      version: VERSION,
      // `ok` means the process is up; `hasKey` is what tells `gramit doctor` whether a
      // fix can actually succeed.
      hasKey: config.azure !== null,
      model: config.azure?.deployment ?? null,
      missing: config.missingAzureVars,
      uptime_s: Math.round((Date.now() - startedAt) / 1000),
    });
  });

  return router;
}
