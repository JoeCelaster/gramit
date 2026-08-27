// A stand-in for the real backend, for working on the daemon without Azure credentials.
//
//   node backend/dev-stub.mjs        # listens on 127.0.0.1:8787
//
// It fakes the fix with a deterministic edit rather than calling a model, so the
// daemon's capture → fix → paste loop can be exercised end to end and the result is
// predictable enough to assert on. Both modes are served, so switching with
// `gramit mode` can be tested here too.
//
// In code mode:
//   code with a request comment  → the comment becomes a stub call, and it pastes
//   a bare request ("two sum")   → the whole selection becomes a stub function
//   code that asks for nothing   → returned unchanged, and it reports "no changes"
// In grammar mode a few canned corrections are applied instead.

import http from 'node:http';

const PORT = Number(process.env.PORT ?? 8787);
const HOST = process.env.HOST ?? '127.0.0.1';

// A comment-only line, in the shapes the languages we care about spell it. The
// captured group is the request the model would have been asked to carry out.
const COMMENT = /^([ \t]*)(?:\/\/+|#+|--|\*|\/\*)[ \t]*(.*?)[ \t]*(?:\*\/)?[ \t]*$/;

// Punctuation and keywords that prose does not have. Their absence is what separates
// a bare request ("Write Java code for two sum") from a block of code.
const LOOKS_LIKE_CODE = /[{};=()[\]]|\b(?:def|function|class|return|import|const|let|var)\b/;

/**
 * Fakes the rewrite with an edit that names what was asked for, so a paste is visible
 * at a glance and the same input always produces the same output.
 *
 * A request comment is consumed in place, keeping its indentation. A selection that is
 * a bare request becomes a whole stub function, which is the shape the real backend
 * returns for one. Code with no request in it comes back untouched.
 */
function rewrite(text) {
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i += 1) {
    const match = COMMENT.exec(lines[i]);
    if (!match || match[2] === '') continue;
    const [, indent, request] = match;
    lines[i] = `${indent}stub(${JSON.stringify(request)});`;
    return lines.join('\n');
  }

  if (!LOOKS_LIKE_CODE.test(text)) {
    const lead = /^\s*/.exec(text)[0];
    const trail = /\s*$/.exec(text)[0];
    return `${lead}def stub():\n    return ${JSON.stringify(text.trim())}${trail}`;
  }

  // Code that asks for nothing: leave it alone, exactly as the real backend would.
  return text;
}

const GRAMMAR_RULES = [
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

/** Canned corrections, plus a capital and a full stop so a paste is obvious. */
function correct(text) {
  let out = text;
  for (const [pattern, replacement] of GRAMMAR_RULES) out = out.replace(pattern, replacement);

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
      let mode;
      try {
        ({ text, mode } = JSON.parse(raw));
      } catch {
        return send(400, {
          error: { code: 'INVALID_REQUEST', message: 'Body is not valid JSON.', retryable: false },
        });
      }

      if (typeof text !== 'string' || text.trim() === '') {
        return send(400, {
          error: { code: 'EMPTY_TEXT', message: 'Nothing to fix.', retryable: false },
        });
      }

      const corrected = mode === 'grammar' ? correct(text) : rewrite(text);
      console.log(`fix [${mode ?? 'code'}]: ${JSON.stringify(text)} -> ${JSON.stringify(corrected)}`);
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
  console.log('This fakes the fix with a canned edit. It does NOT call a model.');
});
