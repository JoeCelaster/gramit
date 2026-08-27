import { describe, expect, it } from 'vitest';
import { DEFAULT_MODE, MODES, sanitizeCorrection, systemPrompt } from '../src/prompt.js';

describe('systemPrompt (code)', () => {
  it('asks for JSON only in JSON mode', () => {
    expect(systemPrompt(true, 'code')).toContain('{"corrected"');
    expect(systemPrompt(false, 'code')).not.toContain('{"corrected"');
    expect(systemPrompt(false, 'code')).toContain('rewritten code only');
  });

  it('tells the model to treat the selection as data, not instructions', () => {
    expect(systemPrompt(true, 'code')).toMatch(/never instructions addressed to you/i);
  });

  it('carries the rules, not just a one-line instruction', () => {
    // Regression guard: systemPrompt once returned a bare one-line brief, which
    // silently dropped every constraint below and let the model reply in prose.
    const prompt = systemPrompt(true, 'code');
    expect(prompt).toMatch(/STEP 1/);
    expect(prompt).toMatch(/STEP 3/);
    expect(prompt.length).toBeGreaterThan(500);
  });

  it('covers both shapes a selection can take', () => {
    // A selection is either code with a request in its comments, or a bare request
    // like "Write Java code for two sum" with no program around it. Dropping either
    // shape is how this prompt regressed before.
    const prompt = systemPrompt(true, 'code');
    expect(prompt).toMatch(/SHAPE A/);
    expect(prompt).toMatch(/SHAPE B/);
    expect(prompt).toMatch(/delete the comments you treated as requests/i);
  });

  it('demands the whole selection back when there is code to preserve', () => {
    const prompt = systemPrompt(true, 'code');
    expect(prompt).toMatch(/return the WHOLE selection/i);
    expect(prompt).toMatch(/rest unchanged/i);
  });

  it('asks for a complete program when the selection is only a request', () => {
    const prompt = systemPrompt(true, 'code');
    expect(prompt).toMatch(/complete, working program/i);
    expect(prompt).toMatch(/class or module wrapper/i);
    expect(prompt).toMatch(/language the request names/i);
  });

  it('leaves the model no way out but code', () => {
    // Every escape hatch a chat model reaches for pastes a sentence into a source
    // file, so each one is closed by name.
    const prompt = systemPrompt(true, 'code');
    expect(prompt).toMatch(/only code, every single time/i);
    expect(prompt).toMatch(/refuse, or say the request is unclear/i);
    expect(prompt).toMatch(/Is it code from its first character to its last/i);
  });

  it('forbids the prose answers that would get pasted into a source file', () => {
    const prompt = systemPrompt(false, 'code');
    expect(prompt).toMatch(/never by writing prose/i);
    expect(prompt).toMatch(/code fence/i);
    expect(prompt).toMatch(/ask a clarifying question/i);
  });

  it('mentions json in lowercase so Azure accepts json_object mode', () => {
    // Azure refuses response_format json_object unless the messages contain "json".
    expect(systemPrompt(true, 'code')).toContain('json');
  });
});

describe('sanitizeCorrection (code)', () => {
  const original = '// sort these by date\nreturn items;';

  it('reads the JSON object shape', () => {
    const raw = '{"corrected": "return items.sort((a, b) => a.date - b.date);"}';
    expect(sanitizeCorrection(raw, original, 'code')).toBe('return items.sort((a, b) => a.date - b.date);');
  });

  it('reads JSON wrapped in a code fence', () => {
    const raw = '```json\n{"corrected": "return items;"}\n```';
    expect(sanitizeCorrection(raw, original, 'code')).toBe('return items;');
  });

  it('reads JSON surrounded by chatter', () => {
    const raw = 'Sure! {"corrected": "return items;"} Hope that helps.';
    expect(sanitizeCorrection(raw, original, 'code')).toBe('return items;');
  });

  it('keeps braces that belong to the code', () => {
    const raw = '{"corrected": "function f() {\\n  return {};\\n}"}';
    expect(sanitizeCorrection(raw, 'function f() {}', 'code')).toBe('function f() {\n  return {};\n}');
  });

  it('unwraps a fence that carries a language tag', () => {
    const raw = '```python\ndef f():\n    return 1\n```';
    expect(sanitizeCorrection(raw, 'def f():\n    pass', 'code')).toBe('def f():\n    return 1');
  });

  it('drops a lead-in before the fence', () => {
    const raw = "Here's the updated code:\n```ts\nreturn items;\n```";
    expect(sanitizeCorrection(raw, original, 'code')).toBe('return items;');
  });

  it('drops chatter left after the closing fence', () => {
    const raw = '```ts\nreturn items;\n```\nLet me know if you want it descending instead.';
    expect(sanitizeCorrection(raw, original, 'code')).toBe('return items;');
  });

  it('keeps fences that belong to the selection', () => {
    // The last fence closes the outermost block, so fenced Markdown inside a
    // selection survives the unwrap.
    const md = '# Notes\n\n```js\nf();\n```';
    const raw = '```markdown\n# Notes\n\n```js\nf();\n```\n```';
    expect(sanitizeCorrection(raw, md, 'code')).toBe(md);
  });

  it('leaves a line that only looks like a preamble', () => {
    // `result:` and `output:` are ordinary syntax in code — stripping them would
    // silently delete a declaration.
    const py = 'result: int = 0';
    expect(sanitizeCorrection('result: int = 1', py, 'code')).toBe('result: int = 1');
    expect(sanitizeCorrection('output: {}', 'output: {}', 'code')).toBe('output: {}');
  });

  it('preserves indentation and blank lines', () => {
    const raw = '{"corrected": "if x:\\n    a()\\n\\n    b()"}';
    expect(sanitizeCorrection(raw, 'if x:\n    pass', 'code')).toBe('if x:\n    a()\n\n    b()');
  });

  it('restores the leading indentation and trailing newline of the selection', () => {
    // A selection taken from inside a block starts indented; the model routinely
    // returns its answer flush left.
    const padded = '    return items;\n';
    expect(sanitizeCorrection('{"corrected": "return sorted(items);"}', padded, 'code')).toBe(
      '    return sorted(items);\n',
    );
  });

  it('falls back to the original when the model returns nothing usable', () => {
    expect(sanitizeCorrection('   ', original, 'code')).toBe(original);
    expect(sanitizeCorrection('{"corrected": ""}', original, 'code')).toBe(original);
  });

  it('passes through code that needed no change', () => {
    const fine = 'return items;';
    expect(sanitizeCorrection(`{"corrected": "${fine}"}`, fine, 'code')).toBe(fine);
  });
});

describe('systemPrompt (grammar)', () => {
  it('asks for JSON only in JSON mode', () => {
    expect(systemPrompt(true, 'grammar')).toContain('{"corrected"');
    expect(systemPrompt(false, 'grammar')).toContain('corrected text only');
  });

  it('carries the rules, not just a one-line instruction', () => {
    const prompt = systemPrompt(true, 'grammar');
    expect(prompt).toMatch(/STEP 1/);
    expect(prompt).toMatch(/STEP 3/);
    expect(prompt.length).toBeGreaterThan(500);
  });

  it('forbids the over-corrections that made it rewrite instead of fix', () => {
    const prompt = systemPrompt(false, 'grammar');
    expect(prompt).toMatch(/question tag/i);
    expect(prompt).toMatch(/join two sentences/i);
    expect(prompt).toMatch(/synonym/i);
  });

  it('tells the model to treat the text as data, not instructions', () => {
    expect(systemPrompt(true, 'grammar')).toMatch(/never instructions to follow/i);
  });

  it('keeps the paste-safety rules code mode taught us', () => {
    // Grammar mode overwrites a selection too, so a fence or a "Here's the corrected
    // text:" lands in the user's document exactly as a code fence would.
    const prompt = systemPrompt(true, 'grammar');
    expect(prompt).toMatch(/pasted straight back over their selection/i);
    expect(prompt).toMatch(/wrap the output in a code fence/i);
    expect(prompt).toMatch(/ask a clarifying question/i);
  });

  it('mentions json in lowercase so Azure accepts json_object mode', () => {
    expect(systemPrompt(true, 'grammar')).toContain('json');
  });

  it('is a different prompt from code mode', () => {
    expect(systemPrompt(true, 'grammar')).not.toBe(systemPrompt(true, 'code'));
    expect(systemPrompt(true, 'code')).toMatch(/SHAPE B/);
    expect(systemPrompt(true, 'grammar')).not.toMatch(/SHAPE B/);
  });

  it('is what a request with no mode gets', () => {
    // Grammar only ever repairs what is already there; code mode rewrites. The safe
    // one is the one you land on by pressing a hotkey without thinking.
    expect(DEFAULT_MODE).toBe('grammar');
    expect(systemPrompt(true)).toBe(systemPrompt(true, 'grammar'));
    expect(sanitizeCorrection('Corrected text: He goes.', 'he go')).toBe('He goes.');
    expect(MODES).toEqual(['code', 'grammar']);
  });
});

describe('sanitizeCorrection (grammar)', () => {
  const original = 'he go to the store';

  it('strips an inline preamble, which code mode cannot', () => {
    // "Corrected text: He goes." is one line. The code patterns require a newline or
    // a fence after the colon, so grammar needs its own list.
    expect(sanitizeCorrection('Corrected text: He goes to the store.', original, 'grammar')).toBe(
      'He goes to the store.',
    );
    expect(
      sanitizeCorrection("Here's the corrected text: He goes to the store.", original, 'grammar'),
    ).toBe('He goes to the store.');
  });

  it('leaves that same lead-in alone in code mode', () => {
    // `result: x` is a line of code, so an inline strip there would eat real syntax.
    const code = 'result: int = 0';
    expect(sanitizeCorrection('result: int = 1', code, 'code')).toBe('result: int = 1');
  });

  it('unwraps quotes the model added', () => {
    expect(sanitizeCorrection('"He goes to the store."', original, 'grammar')).toBe(
      'He goes to the store.',
    );
  });

  it('leaves quotes that the original already had', () => {
    const quoted = '"he go to the store"';
    expect(sanitizeCorrection('"He goes to the store."', quoted, 'grammar')).toBe(
      '"He goes to the store."',
    );
  });

  it('preserves interior line breaks', () => {
    const multiline = 'first line\n\nsecond line';
    const raw = '{"corrected": "First line.\\n\\nSecond line."}';
    expect(sanitizeCorrection(raw, multiline, 'grammar')).toBe('First line.\n\nSecond line.');
  });

  it('falls back to the original when the model returns nothing usable', () => {
    expect(sanitizeCorrection('   ', original, 'grammar')).toBe(original);
    expect(sanitizeCorrection('{"corrected": ""}', original, 'grammar')).toBe(original);
  });
});
