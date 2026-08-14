import { diffWords } from 'diff';

/**
 * Counts how many distinct edits turn `before` into `after`.
 *
 * A contiguous run of added/removed word groups counts as one change, so a single
 * substitution ("go" → "goes") reports 1, not 2. This number is what the daemon
 * shows in its toast ("Fixed 3 issues"), so it should match what a human would
 * count when eyeballing the diff.
 */
export function countChanges(before: string, after: string): number {
  if (before === after) return 0;

  const parts = diffWords(before, after);
  let changes = 0;
  let inRun = false;

  for (const part of parts) {
    const isChange = Boolean(part.added || part.removed);
    if (isChange) {
      if (!inRun) changes += 1;
      inRun = true;
    } else if (part.value.trim() === '') {
      // Pure whitespace between two edits doesn't break the run — "he go store" →
      // "he goes to the store" reads as one fix, not two.
      continue;
    } else {
      inRun = false;
    }
  }

  // Differences only in whitespace still constitute a real edit.
  return changes === 0 ? 1 : changes;
}
