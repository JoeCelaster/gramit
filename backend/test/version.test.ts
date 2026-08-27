import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { VERSION } from '../src/routes/health.js';

describe('reported version', () => {
  it('matches package.json', () => {
    // These drifted once already — /health reported 0.1.2 while the package said
    // 0.1.3 — and nothing caught it, because the only symptom is a wrong number in
    // `gramit doctor`. Keeping the constant (rather than reading the file at run
    // time) avoids depending on the layout a deployment unpacks us into, so this
    // test is what holds the two together.
    const packageJson = fileURLToPath(new URL('../package.json', import.meta.url));
    const { version } = JSON.parse(readFileSync(packageJson, 'utf8')) as { version: string };

    expect(VERSION).toBe(version);
  });
});
