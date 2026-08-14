// Vercel serverless entrypoint. Every route in vercel.json rewrites here, and this
// module hands the request straight to the Express app — no `listen`, because the
// platform owns the socket.
//
// It imports the compiled output rather than `src/`, so the deployed code is exactly
// what `npm run build` type-checks and what `npm start` runs locally. vercel.json's
// buildCommand makes sure dist/ exists before this function is packaged.
import { bootstrap } from '../dist/bootstrap.js';

// Built once per cold start and reused across invocations on the same instance.
const { app } = bootstrap();

export default function handler(req, res) {
  // A rewrite normally preserves the original path, so the app sees `/health` as
  // written. If the `/api` destination ever leaks through instead, strip it rather
  // than 404 on a request that was routed correctly.
  if (req.url === '/api') req.url = '/';
  else if (req.url?.startsWith('/api/')) req.url = req.url.slice('/api'.length);

  return app(req, res);
}
