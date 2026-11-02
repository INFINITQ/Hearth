import { describe, expect, it } from 'vitest';
import { formatDuration, groupByDate } from './dates';

const DAY = 86_400_000;

describe('groupByDate', () => {
  it('puts pinned items first, then groups the rest by recency', () => {
    const now = new Date(2026, 10, 2, 15, 0).getTime();
    const items = [
      { id: 'a', updatedAt: now - 60_000, pinned: false },
      { id: 'b', updatedAt: now - DAY, pinned: false },
      { id: 'c', updatedAt: now - 3 * DAY, pinned: false },
      { id: 'd', updatedAt: now - 90 * DAY, pinned: false },
      { id: 'e', updatedAt: now - 90 * DAY, pinned: true },
    ];
    const groups = groupByDate(items, now).map((g) => [g.label, g.items.map((i) => i.id)]);
    expect(groups).toEqual([
      ['Pinned', ['e']],
      ['Today', ['a']],
      ['Yesterday', ['b']],
      ['Previous 7 days', ['c']],
      ['Older', ['d']],
    ]);
  });
});

describe('formatDuration', () => {
  it('reads naturally', () => {
    expect(formatDuration(400)).toBe('1s');
    expect(formatDuration(12_300)).toBe('12s');
    expect(formatDuration(65_000)).toBe('1m 5s');
  });
});
