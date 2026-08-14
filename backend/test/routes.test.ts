import request from 'supertest';
import { describe, expect, it } from 'vitest';
import { createApp } from '../src/app.js';
import { loadConfig, type Config } from '../src/config.js';
import { AppError } from '../src/errors.js';
import type { FixService } from '../src/service.js';

const CONFIGURED_ENV = {
  AZURE_OPENAI_ENDPOINT: 'https://example.openai.azure.com/',
  AZURE_OPENAI_API_KEY: 'secret',
  AZURE_OPENAI_DEPLOYMENT: 'gpt-5.6-luna',
  AZURE_OPENAI_API_VERSION: '2024-10-21',
} satisfies NodeJS.ProcessEnv;

function appWith(service: Partial<FixService>, config: Config = loadConfig(CONFIGURED_ENV)) {
  const full: FixService = {
    fix: service.fix ?? (async () => Promise.reject(new Error('not stubbed'))),
  };
  return createApp({ config, service: full });
}

const okService: FixService = {
  async fix(text) {
    return {
      corrected: 'He goes to the store.',
      changed: true,
      changes: 2,
      model: 'gpt-5.6-luna',
      latency_ms: 42,
      cached: false,
    };
  },
};

describe('GET /health', () => {
  it('reports hasKey true when Azure is configured', async () => {
    const res = await request(appWith(okService)).get('/health');

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({ ok: true, hasKey: true, model: 'gpt-5.6-luna', missing: [] });
  });

  it('reports hasKey false and names the missing vars', async () => {
    const config = loadConfig({ ...CONFIGURED_ENV, AZURE_OPENAI_API_KEY: '' });
    const res = await request(appWith(okService, config)).get('/health');

    expect(res.body.hasKey).toBe(false);
    expect(res.body.model).toBeNull();
    expect(res.body.missing).toContain('AZURE_OPENAI_API_KEY');
  });

  it('never leaks the api key', async () => {
    const res = await request(appWith(okService)).get('/health');
    expect(JSON.stringify(res.body)).not.toContain('secret');
  });
});

describe('POST /v1/fix', () => {
  it('returns the correction', async () => {
    const res = await request(appWith(okService))
      .post('/v1/fix')
      .send({ text: 'he go to the store' });

    expect(res.status).toBe(200);
    expect(res.body).toMatchObject({
      corrected: 'He goes to the store.',
      changed: true,
      changes: 2,
      model: 'gpt-5.6-luna',
      cached: false,
    });
  });

  it('defaults mode to grammar', async () => {
    let seenMode: string | undefined;
    const app = appWith({
      async fix(text, mode) {
        seenMode = mode;
        return { corrected: text, changed: false, changes: 0, model: 'm', latency_ms: 1, cached: false };
      },
    });

    await request(app).post('/v1/fix').send({ text: 'hello' }).expect(200);
    expect(seenMode).toBe('grammar');
  });

  it('rejects a missing text field', async () => {
    const res = await request(appWith(okService)).post('/v1/fix').send({});

    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_REQUEST');
    expect(res.body.error.message).toContain('text');
  });

  it('rejects a non-string text field', async () => {
    const res = await request(appWith(okService)).post('/v1/fix').send({ text: 42 });
    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_REQUEST');
  });

  it('rejects an unknown mode', async () => {
    const res = await request(appWith(okService)).post('/v1/fix').send({ text: 'hi', mode: 'sarcastic' });
    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_REQUEST');
  });

  it('rejects malformed JSON', async () => {
    const res = await request(appWith(okService))
      .post('/v1/fix')
      .set('content-type', 'application/json')
      .send('{"text": ');

    expect(res.status).toBe(400);
    expect(res.body.error.code).toBe('INVALID_REQUEST');
  });

  it('surfaces service errors with their code and status', async () => {
    const app = appWith({
      async fix() {
        throw AppError.rateLimited();
      },
    });

    const res = await request(app).post('/v1/fix').send({ text: 'hello' });

    expect(res.status).toBe(429);
    expect(res.body.error).toMatchObject({ code: 'RATE_LIMITED', retryable: true });
  });

  it('maps an unexpected throw to INTERNAL', async () => {
    const app = appWith({
      async fix() {
        throw new Error('boom');
      },
    });

    const res = await request(app).post('/v1/fix').send({ text: 'hello' });

    expect(res.status).toBe(500);
    expect(res.body.error.code).toBe('INTERNAL');
  });
});

describe('unknown routes', () => {
  it('returns a structured 404', async () => {
    const res = await request(appWith(okService)).get('/nope');

    expect(res.status).toBe(404);
    expect(res.body.error.code).toBe('NOT_FOUND');
  });
});
