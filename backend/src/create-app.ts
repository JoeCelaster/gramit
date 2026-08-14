// Named create-app rather than app on purpose: Vercel picks a deployment's server
// entrypoint by searching the build output for app.js before index.js, and would
// otherwise start this module — which exports a factory and never listens.
import express, { type NextFunction, type Request, type Response } from 'express';
import type { Config } from './config.js';
import { AppError, toErrorBody } from './errors.js';
import { log } from './logger.js';
import { fixRouter } from './routes/fix.js';
import { healthRouter } from './routes/health.js';
import type { FixService } from './service.js';

export interface AppOptions {
  config: Config;
  service: FixService;
}

function normalizeError(err: unknown): AppError {
  if (err instanceof AppError) return err;

  // express.json() rejections arrive as SyntaxError with a `type` tag.
  if (err && typeof err === 'object' && 'type' in err) {
    const type = (err as { type?: string }).type;
    if (type === 'entity.parse.failed') return AppError.invalidRequest('Request body is not valid JSON.');
    if (type === 'entity.too.large') return new AppError('TOO_LONG', 413, 'Request body is too large.');
  }

  return new AppError('INTERNAL', 500, err instanceof Error ? err.message : 'Unexpected error.');
}

/** `app` is a parameter so the entrypoint can own the express instance — Vercel decides
 *  a deployment is a Node server by finding an express import in the entrypoint file
 *  itself, and does not follow the import graph to get here. */
export function createApp({ config, service }: AppOptions, app: express.Express = express()): express.Express {
  app.disable('x-powered-by');
  // A correction request is text; 2mb is generous and keeps a runaway client cheap.
  app.use(express.json({ limit: '2mb' }));

  app.use(healthRouter(config));
  app.use('/v1', fixRouter(service));

  app.use((_req, _res, next) => {
    next(new AppError('NOT_FOUND', 404, 'No such endpoint.'));
  });

  app.use((err: unknown, req: Request, res: Response, _next: NextFunction) => {
    const appError = normalizeError(err);
    if (appError.status >= 500 && appError.code !== 'NOT_FOUND') {
      log.error('request failed', {
        method: req.method,
        path: req.path,
        code: appError.code,
        message: appError.message,
      });
    }
    res.status(appError.status).json(toErrorBody(appError));
  });

  return app;
}
