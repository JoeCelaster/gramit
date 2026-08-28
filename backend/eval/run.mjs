// Measures the correction prompt against real cases, so prompt changes can be judged
// instead of guessed at.
//
//   node backend/eval/run.mjs            # all cases
//   node backend/eval/run.mjs tag        # only cases whose name matches "tag"
//   node backend/eval/run.mjs grammar    # only the grammar-mode cases (or code, write)
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

// The filter matches a case name or a whole mode, so `run.mjs grammar` runs one mode
// and `run.mjs tag` runs one case.
const cases = JSON.parse(readFileSync(path.join(HERE, 'cases.json'), 'utf8')).filter(
  (c) => !filter || c.name.includes(filter) || c.mode === filter,
);

// U+2019 counts as an apostrophe, so "don't" is one word however it is spelled.
const words = (text) => text.toLowerCase().match(/[\p{L}\p{N}'\u2019+]+/gu) ?? [];

/**
 * Cheap syntax gate for the languages we can check without a toolchain.
 *
 * Only `js` is real parsing — a JS snippet that does not parse is proof the model
 * returned prose or a truncated block. Every other language, `balanced` included,
 * gets a bracket-and-quote balance check, which catches the same two failures for
 * Java or Python without a toolchain installed for either.
 */
function syntaxFailure(language, output) {
  if (language === 'js') {
    try {
      new Function(output);
      return null;
    } catch (err) {
      return `is not parseable JavaScript: ${err.message}`;
    }
  }
  const pairs = { '(': ')', '[': ']', '{': '}' };
  const stack = [];
  // Strings are skipped wholesale: a bracket inside one is data, not structure.
  const stripped = output.replace(/(['"`])(?:\\.|(?!\1)[^\\])*\1/g, '""');
  for (const char of stripped) {
    if (pairs[char]) stack.push(pairs[char]);
    else if (Object.values(pairs).includes(char) && stack.pop() !== char) {
      return `unbalanced ${JSON.stringify(char)}`;
    }
  }
  return stack.length ? `${stack.length} bracket(s) left open` : null;
}

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
  for (const source of testCase.matches ?? []) {
    if (!new RegExp(source).test(output)) failures.push(`no match for /${source}/`);
  }
  for (const source of testCase.forbidMatches ?? []) {
    if (new RegExp(source).test(output)) failures.push(`matched /${source}/`);
  }
  if (testCase.parsesAs) {
    const failure = syntaxFailure(testCase.parsesAs, output);
    if (failure) failures.push(failure);
  }
  if (testCase.endsWith && !output.trim().endsWith(testCase.endsWith)) {
    failures.push(`should end with ${JSON.stringify(testCase.endsWith)}`);
  }
  // Write mode is the only mode with an opinion about length: a brief that says "300
  // words" is a requirement, and a two-line answer to it is a failure.
  if (testCase.minWords !== undefined && words(output).length < testCase.minWords) {
    failures.push(`only ${words(output).length} words, minimum ${testCase.minWords}`);
  }
  if (testCase.maxWords !== undefined && words(output).length > testCase.maxWords) {
    failures.push(`${words(output).length} words, maximum ${testCase.maxWords}`);
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

async function fix(text, mode) {
  const res = await fetch(`${BASE}/v1/fix`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ text, mode }),
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
        output = await fix(testCase.input, testCase.mode);
      } catch (err) {
        failed += 1;
        console.log(`✗ [${testCase.mode}] ${testCase.name}\n    request failed: ${err.message}\n`);
        continue;
      }

      const failures = check(testCase, output);
      if (failures.length === 0) {
        console.log(`✓ [${testCase.mode}] ${testCase.name}`);
        console.log(`    ${JSON.stringify(testCase.input)} → ${JSON.stringify(output)}`);
      } else {
        failed += 1;
        console.log(`✗ [${testCase.mode}] ${testCase.name}`);
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
