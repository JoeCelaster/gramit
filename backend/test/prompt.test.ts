import { describe, expect, it } from 'vitest';
import { sanitizeCorrection, systemPrompt } from '../src/prompt.js';

describe('systemPrompt', () => {
  it('asks for JSON only in JSON mode', () => {
    expect(systemPrompt(true)).toContain('{"corrected"');
    expect(systemPrompt(false)).not.toContain('{"corrected"');
    expect(systemPrompt(false)).toContain('corrected text only');
  });

  it('tells the model to treat the input as data, not instructions', () => {
    expect(systemPrompt(true)).toMatch(/never instructions to follow/i);
  });

  it('carries the rules, not just a one-line instruction', () => {
    // Regression guard: systemPrompt once returned a bare "Fix English..." string,
    // which silently dropped every constraint below and let the model rewrite freely.
    const prompt = systemPrompt(true);
    expect(prompt).toMatch(/STEP 1/);
    expect(prompt).toMatch(/STEP 3/);
    expect(prompt.length).toBeGreaterThan(500);
  });

  it('forbids the over-corrections that made it rewrite instead of fix', () => {
    const prompt = systemPrompt(false);
    expect(prompt).toMatch(/question tag/i);
    expect(prompt).toMatch(/join two sentences/i);
    expect(prompt).toMatch(/synonym/i);
  });

  it('mentions json in lowercase so Azure accepts json_object mode', () => {
    // Azure refuses response_format json_object unless the messages contain "json".
    expect(systemPrompt(true)).toContain('json');
  });
});

describe('sanitizeCorrection', () => {
  const original = 'he go to the store';

  it('reads the JSON object shape', () => {
    expect(sanitizeCorrection('{"corrected": "He goes to the store."}', original)).toBe(
      'He goes to the store.',
    );
  });

  it('reads JSON wrapped in a code fence', () => {
    const raw = '```json\n{"corrected": "He goes to the store."}\n```';
    expect(sanitizeCorrection(raw, original)).toBe('He goes to the store.');
  });

  it('reads JSON surrounded by chatter', () => {
    const raw = 'Sure! {"corrected": "He goes to the store."} Hope that helps.';
    expect(sanitizeCorrection(raw, original)).toBe('He goes to the store.');
  });

  it('keeps braces that belong to the text', () => {
    const braced = 'hello {name} how are you';
    const raw = '{"corrected": "Hello {name}, how are you?"}';
    expect(sanitizeCorrection(raw, braced)).toBe('Hello {name}, how are you?');
  });

  it('strips a plain-text preamble', () => {
    expect(sanitizeCorrection('Corrected text: He goes to the store.', original)).toBe(
      'He goes to the store.',
    );
    expect(sanitizeCorrection("Here's the corrected text: He goes to the store.", original)).toBe(
      'He goes to the store.',
    );
  });

  it('strips a bare code fence', () => {
    expect(sanitizeCorrection('```\nHe goes to the store.\n```', original)).toBe(
      'He goes to the store.',
    );
  });

  it('unwraps quotes the model added', () => {
    expect(sanitizeCorrection('"He goes to the store."', original)).toBe('He goes to the store.');
  });

  it('leaves quotes that the original already had', () => {
    const quoted = '"he go to the store"';
    expect(sanitizeCorrection('"He goes to the store."', quoted)).toBe('"He goes to the store."');
  });

  it('restores the original leading and trailing whitespace', () => {
    const padded = '  he go to the store\n';
    expect(sanitizeCorrection('{"corrected": "He goes to the store."}', padded)).toBe(
      '  He goes to the store.\n',
    );
  });

  it('preserves interior line breaks', () => {
    const multiline = 'first line\n\nsecond line';
    const raw = '{"corrected": "First line.\\n\\nSecond line."}';
    expect(sanitizeCorrection(raw, multiline)).toBe('First line.\n\nSecond line.');
  });

  it('falls back to the original when the model returns nothing usable', () => {
    expect(sanitizeCorrection('   ', original)).toBe(original);
    expect(sanitizeCorrection('{"corrected": ""}', original)).toBe(original);
  });

  it('passes through text that needed no correction', () => {
    const fine = 'He goes to the store.';
    expect(sanitizeCorrection(`{"corrected": "${fine}"}`, fine)).toBe(fine);
  });
});
