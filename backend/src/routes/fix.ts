import { Router } from 'express';
import { z } from 'zod';
import { AppError } from '../errors.js';
import { MODES } from '../prompt.js';
import type { FixService } from '../service.js';

const FixRequest = z.object({
  text: z.string(),
  mode: z.enum(MODES).default('grammar'),
});

export function fixRouter(service: FixService): Router {
  const router = Router();

  router.post('/fix', async (req, res) => {
    const parsed = FixRequest.safeParse(req.body);
    if (!parsed.success) {
      const detail = parsed.error.issues
        .map((issue) => `${issue.path.join('.') || 'body'}: ${issue.message}`)
        .join('; ');
      throw AppError.invalidRequest(detail);
    }

    const outcome = await service.fix(parsed.data.text, parsed.data.mode);
    res.json(outcome);
  });

  return router;
}
