import { describe, expect, it, vi } from 'vitest';
import { AppError } from '../src/errors.js';
import type { LinkContext, LinkReader } from '../src/links.js';
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
    const out = await service.fix('he go to the store', 'code');

    expect(out.corrected).toBe('He goes to the store.');
    expect(out.changed).toBe(true);
    expect(out.changes).toBeGreaterThan(0);
    expect(out.model).toBe('gpt-5.6-luna');
  });

  it('reports changed=false and 0 changes for already-correct text', async () => {
    const clean = 'He goes to the store.';
    const service = createFixService({ ...base, corrector: fakeCorrector(clean) });
    const out = await service.fix(clean, 'code');

    expect(out.changed).toBe(false);
    expect(out.changes).toBe(0);
  });

  it('calls the model every time, even for identical text', async () => {
    // There is no cache: the same text must be re-corrected, so a prompt change or a
    // different sampling of the model is never masked by a stale answer.
    const corrector = fakeCorrector('He goes to the store.');
    const service = createFixService({ ...base, corrector });

    await service.fix('he go to the store', 'code');
    await service.fix('he go to the store', 'code');

    expect(corrector.calls).toBe(2);
  });

  it('rejects whitespace-only text', async () => {
    const service = createFixService({ ...base, corrector: fakeCorrector('x') });
    await expect(service.fix('   \n ', 'code')).rejects.toMatchObject({ code: 'EMPTY_TEXT' });
  });

  it('rejects text over the limit', async () => {
    const service = createFixService({ ...base, corrector: fakeCorrector('x') });
    await expect(service.fix('a'.repeat(101), 'code')).rejects.toMatchObject({ code: 'TOO_LONG' });
  });

  it('fails with NO_API_KEY when Azure is not configured', async () => {
    const service = createFixService({
      ...base,
      corrector: null,
      missingAzureVars: ['AZURE_OPENAI_API_KEY'],
    });

    const err = await service.fix('hello', 'code').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(AppError);
    expect(err).toMatchObject({ code: 'NO_API_KEY', status: 503 });
    expect((err as AppError).message).toContain('AZURE_OPENAI_API_KEY');
  });

  it('reports latency', async () => {
    const now = vi.fn().mockReturnValueOnce(1_000).mockReturnValue(1_250);
    const service = createFixService({ ...base, corrector: fakeCorrector('Fixed.'), now });

    const out = await service.fix('fixed', 'code');
    expect(out.latency_ms).toBe(250);
  });
});

describe('linked content', () => {
  /** Records what the corrector was handed alongside the instruction. */
  function recordingCorrector(): Corrector & { context: string | undefined } {
    const stub = {
      context: undefined as string | undefined,
      async fix(_text: string, _mode: never, context?: string) {
        stub.context = context;
        return { corrected: 'a written piece', model: 'm' };
      },
    };
    return stub as unknown as Corrector & { context: string | undefined };
  }

  function fakeLinks(contexts: LinkContext[]): LinkReader & { calls: number } {
    const stub = {
      calls: 0,
      async read() {
        stub.calls += 1;
        return contexts;
      },
    };
    return stub;
  }

  it('hands the page text to the model in write mode', async () => {
    const corrector = recordingCorrector();
    const links = fakeLinks([{ url: 'https://example.com/a', title: 'A', text: 'ten days' }]);
    const service = createFixService({ ...base, corrector, links });

    await service.fix('linkedin post about https://example.com/a', 'write');

    expect(links.calls).toBe(1);
    expect(corrector.context).toContain('LINKED CONTENT');
    expect(corrector.context).toContain('ten days');
  });

  it('reads no link in code, grammar or prompt mode', async () => {
    // Those modes transform the selection in front of them. Fetching a URL they happen
    // to contain would cost a round trip and leak where the user's text points.
    // Prompt mode included: it rewrites the asking, and a URL in a rough request is
    // context for whichever model the prompt is finally sent to, not for this one.
    const links = fakeLinks([]);
    const service = createFixService({ ...base, corrector: recordingCorrector(), links });

    await service.fix('see https://example.com/a', 'grammar');
    await service.fix('// see https://example.com/a', 'code');
    await service.fix('summarise https://example.com/a', 'prompt');

    expect(links.calls).toBe(0);
  });

  it('sends no context when the page could not be read', async () => {
    // The prompt then tells the model to write from the instruction alone rather than
    // guess at what the page said, so `undefined` here is load-bearing.
    const corrector = recordingCorrector();
    const service = createFixService({ ...base, corrector, links: fakeLinks([]) });

    await service.fix('post about https://dead.example/a', 'write');

    expect(corrector.context).toBeUndefined();
  });

  it('still writes when link reading is switched off entirely', async () => {
    const corrector = recordingCorrector();
    const service = createFixService({ ...base, corrector, links: null });

    const out = await service.fix('post about https://example.com/a', 'write');

    expect(out.corrected).toBe('a written piece');
    expect(corrector.context).toBeUndefined();
  });
});
