import { describe, expect, it, vi } from 'vitest';
import { AppError } from '../src/errors.js';
import type { Corrector } from '../src/llm/azure.js';
import { createFixService } from '../src/service.js';

function fakeCorrector(corrected: string): Corrector & { calls: number } {
  const stub = {
    calls: 0,
    async fix() {
      stub.calls += 1;
      return { corrected, model: 'gpt-5.6-luna' };
    },
  };
  return stub;
}

const base = { maxChars: 100 };

describe('createFixService', () => {
  it('corrects text and counts the changes', async () => {
    const service = createFixService({ ...base, corrector: fakeCorrector('He goes to the store.') });
    const out = await service.fix('he go to the store', 'grammar');

    expect(out.corrected).toBe('He goes to the store.');
    expect(out.changed).toBe(true);
    expect(out.changes).toBeGreaterThan(0);
    expect(out.model).toBe('gpt-5.6-luna');
  });

  it('reports changed=false and 0 changes for already-correct text', async () => {
    const clean = 'He goes to the store.';
    const service = createFixService({ ...base, corrector: fakeCorrector(clean) });
    const out = await service.fix(clean, 'grammar');

    expect(out.changed).toBe(false);
    expect(out.changes).toBe(0);
  });

  it('calls the model every time, even for identical text', async () => {
    // There is no cache: the same text must be re-corrected, so a prompt change or a
    // different sampling of the model is never masked by a stale answer.
    const corrector = fakeCorrector('He goes to the store.');
    const service = createFixService({ ...base, corrector });

    await service.fix('he go to the store', 'grammar');
    await service.fix('he go to the store', 'grammar');

    expect(corrector.calls).toBe(2);
  });

  it('rejects whitespace-only text', async () => {
    const service = createFixService({ ...base, corrector: fakeCorrector('x') });
    await expect(service.fix('   \n ', 'grammar')).rejects.toMatchObject({ code: 'EMPTY_TEXT' });
  });

  it('rejects text over the limit', async () => {
    const service = createFixService({ ...base, corrector: fakeCorrector('x') });
    await expect(service.fix('a'.repeat(101), 'grammar')).rejects.toMatchObject({ code: 'TOO_LONG' });
  });

  it('fails with NO_API_KEY when Azure is not configured', async () => {
    const service = createFixService({
      ...base,
      corrector: null,
      missingAzureVars: ['AZURE_OPENAI_API_KEY'],
    });

    const err = await service.fix('hello', 'grammar').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(AppError);
    expect(err).toMatchObject({ code: 'NO_API_KEY', status: 503 });
    expect((err as AppError).message).toContain('AZURE_OPENAI_API_KEY');
  });

  it('reports latency', async () => {
    const now = vi.fn().mockReturnValueOnce(1_000).mockReturnValue(1_250);
    const service = createFixService({ ...base, corrector: fakeCorrector('Fixed.'), now });

    const out = await service.fix('fixed', 'grammar');
    expect(out.latency_ms).toBe(250);
  });
});
