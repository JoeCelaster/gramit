// A stand-in for the real backend, for working on the daemon without Azure credentials.
//
//   node backend/dev-stub.mjs        # listens on 127.0.0.1:8787
//
// It applies a few canned corrections rather than calling a model, so the daemon's
// capture → correct → paste loop can be exercised end to end and the result is
// predictable enough to assert on.

import http from 'node:http';

const PORT = Number(process.env.PORT ?? 8787);
const HOST = process.env.HOST ?? '127.0.0.1';

const RULES = [
  [/\bhe go\b/g, 'he goes'],
  [/\bshe go\b/g, 'she goes'],
  [/\bthey goes\b/g, 'they go'],
  [/\bi has\b/gi, 'I have'],
  [/\bdont\b/g, "don't"],
  [/\bcant\b/g, "can't"],
  [/\bwont\b/g, "won't"],
  [/\bteh\b/g, 'the'],
  [/\brecieve\b/g, 'receive'],
  [/\bbuyed\b/g, 'bought'],
  [/\ba apple\b/g, 'an apple'],
  [/\bi\b/g, 'I'],
];

function correct(text) {
  let out = text;
  for (const [pattern, replacement] of RULES) out = out.replace(pattern, replacement);

  // Capitalise the first letter and make sure the sentence is terminated, which makes
  // a successful paste obvious at a glance.
  out = out.replace(/^(\s*)([a-z])/, (_, space, letter) => space + letter.toUpperCase());
  if (/[a-zA-Z0-9]$/.test(out.trimEnd())) {
    const trailing = out.slice(out.trimEnd().length);
    out = `${out.trimEnd()}.${trailing}`;
  }
  return out;
}

function countChanges(before, after) {
  if (before === after) return 0;
  const a = before.split(/\s+/);
  const b = after.split(/\s+/);
  let changes = Math.abs(a.length - b.length);
  for (let i = 0; i < Math.min(a.length, b.length); i += 1) {
    if (a[i] !== b[i]) changes += 1;
  }
  return Math.max(changes, 1);
}

const server = http.createServer((req, res) => {
  const send = (status, body) => {
    res.writeHead(status, { 'content-type': 'application/json' });
    res.end(JSON.stringify(body));
  };

  if (req.method === 'GET' && req.url === '/health') {
    return send(200, {
      ok: true,
      version: 'dev-stub',
      hasKey: true,
      model: 'dev-stub',
      missing: [],
    });
  }

  if (req.method === 'POST' && req.url === '/v1/fix') {
    let raw = '';
    req.on('data', (chunk) => {
      raw += chunk;
    });
    req.on('end', () => {
      let text;
      try {
        ({ text } = JSON.parse(raw));
      } catch {
        return send(400, {
          error: { code: 'INVALID_REQUEST', message: 'Body is not valid JSON.', retryable: false },
        });
      }

      if (typeof text !== 'string' || text.trim() === '') {
        return send(400, {
          error: { code: 'EMPTY_TEXT', message: 'No text to correct.', retryable: false },
        });
      }

      const corrected = correct(text);
      console.log(`fix: ${JSON.stringify(text)} -> ${JSON.stringify(corrected)}`);
      return send(200, {
        corrected,
        changed: corrected !== text,
        changes: countChanges(text, corrected),
        model: 'dev-stub',
        latency_ms: 1,
        cached: false,
      });
    });
    return undefined;
  }

  return send(404, {
    error: { code: 'NOT_FOUND', message: 'No such endpoint.', retryable: false },
  });
});

server.listen(PORT, HOST, () => {
  console.log(`gramit dev stub listening on http://${HOST}:${PORT}`);
  console.log('This applies canned corrections. It does NOT call a model.');
});
