import { describe, expect, it } from 'vitest';
import { countChanges } from '../src/diff.js';

describe('countChanges', () => {
  it('reports 0 for identical text', () => {
    expect(countChanges('He goes to the store.', 'He goes to the store.')).toBe(0);
  });

  it('counts a single word substitution as one change', () => {
    expect(countChanges('he go to the store', 'he goes to the store')).toBe(1);
  });

  it('counts edits in separate places separately', () => {
    expect(countChanges('he go to the store and buyed milk', 'He goes to the store and bought milk')).toBe(
      2,
    );
  });

  it('treats an adjacent insertion and substitution as one change', () => {
    expect(countChanges('he go store', 'he goes to the store')).toBe(1);
  });

  it('counts punctuation-only fixes', () => {
    expect(countChanges('hello world', 'hello, world')).toBeGreaterThanOrEqual(1);
  });

  it('never reports 0 when the text actually changed', () => {
    expect(countChanges('a  b', 'a b')).toBeGreaterThanOrEqual(1);
  });
});
