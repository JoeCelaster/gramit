// Measures the correction prompt against real cases, so prompt changes can be judged
// instead of guessed at.
//
//   node backend/eval/run.mjs            # all cases
//   node backend/eval/run.mjs tag        # only cases whose name matches "tag"
//   node backend/eval/run.mjs --runs 3   # repeat each case (the model is non-deterministic)
//
// Spawns its own backend on a spare port, so the one you have running is untouched.
// Requires `npm run build` first.

import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const BACKEND = path.resolve(HERE, '..');
const PORT = Number(process.env.EVAL_PORT ?? 8799);
const BASE = `http://127.0.0.1:${PORT}`;

const args = process.argv.slice(2);
const runsFlag = args.indexOf('--runs');
const RUNS = runsFlag === -1 ? 1 : Number(args[runsFlag + 1] ?? 1);
const filter = args.find((a) => !a.startsWith('--') && a !== String(RUNS));

const cases = JSON.parse(readFileSync(path.join(HERE, 'cases.json'), 'utf8')).filter(
  (c) => !filter || c.name.includes(filter),
);

// U+2019 counts as an apostrophe, so "don't" is one word however it is spelled.
const words = (text) => text.toLowerCase().match(/[\p{L}\p{N}'\u2019+]+/gu) ?? [];

function check(testCase, output) {
  const failures = [];
  const lower = output.toLowerCase();

  if (testCase.unchanged && output.trim() !== testCase.input.trim()) {
    failures.push(`should have been left unchanged`);
  }
  if (testCase.equals !== undefined && output.trim() !== testCase.equals) {
    failures.push(`expected exactly ${JSON.stringify(testCase.equals)}`);
  }
  for (const word of testCase.keepWords ?? []) {
    if (!lower.includes(word.toLowerCase())) failures.push(`dropped ${JSON.stringify(word)}`);
  }
  for (const needle of testCase.contains ?? []) {
    if (!output.includes(needle)) failures.push(`missing ${JSON.stringify(needle)}`);
  }
  for (const needle of testCase.forbid ?? []) {
    if (lower.includes(needle.toLowerCase())) failures.push(`invented ${JSON.stringify(needle)}`);
  }
  if (testCase.endsWith && !output.trim().endsWith(testCase.endsWith)) {
    failures.push(`should end with ${JSON.stringify(testCase.endsWith)}`);
  }
  if (testCase.maxAddedWords !== undefined) {
    const added = words(output).length - words(testCase.input).length;
    if (added > testCase.maxAddedWords) {
      failures.push(`added ${added} words, limit ${testCase.maxAddedWords}`);
    }
  }
  if (testCase.containsNewlines !== undefined) {
    const count = (output.match(/\n/g) ?? []).length;
    if (count !== testCase.containsNewlines) {
      failures.push(`expected ${testCase.containsNewlines} newlines, got ${count}`);
    }
  }
  if (testCase.sentenceCount !== undefined) {
    const count = (output.match(/[.!?](\s|$)/g) ?? []).length;
    if (count !== testCase.sentenceCount) {
      failures.push(`expected ${testCase.sentenceCount} sentences, got ${count}`);
    }
  }
  return failures;
}

async function waitForHealth(deadlineMs = 30_000) {
  const deadline = Date.now() + deadlineMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${BASE}/health`);
      if (res.ok) return await res.json();
    } catch {
      // not up yet
    }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error(`backend did not start on ${BASE}`);
}

async function fix(text) {
  const res = await fetch(`${BASE}/v1/fix`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ text }),
  });
  const body = await res.json();
  if (!res.ok) throw new Error(`${body?.error?.code}: ${body?.error?.message}`);
  return body.corrected;
}

const server = spawn('node', ['dist/index.js'], {
  cwd: BACKEND,
  env: { ...process.env, PORT: String(PORT) },
  stdio: ['ignore', 'ignore', 'inherit'],
});
process.on('exit', () => server.kill());

let failed = 0;
let total = 0;

try {
  const health = await waitForHealth();
  if (!health.hasKey) throw new Error('backend has no API key — eval needs the real model');
  console.log(`model: ${health.model}   cases: ${cases.length}   runs each: ${RUNS}\n`);

  for (const testCase of cases) {
    for (let run = 0; run < RUNS; run += 1) {
      total += 1;
      let output;
      try {
        output = await fix(testCase.input);
      } catch (err) {
        failed += 1;
        console.log(`✗ ${testCase.name}\n    request failed: ${err.message}\n`);
        continue;
      }

      const failures = check(testCase, output);
      if (failures.length === 0) {
        console.log(`✓ ${testCase.name}`);
        console.log(`    ${JSON.stringify(testCase.input)} → ${JSON.stringify(output)}`);
      } else {
        failed += 1;
        console.log(`✗ ${testCase.name}`);
        console.log(`    ${JSON.stringify(testCase.input)} → ${JSON.stringify(output)}`);
        for (const failure of failures) console.log(`    · ${failure}`);
        if (testCase.why) console.log(`    why it matters: ${testCase.why}`);
      }
      console.log();
    }
  }
} finally {
  server.kill();
}

console.log(`${total - failed}/${total} passed`);
process.exit(failed > 0 ? 1 : 0);
