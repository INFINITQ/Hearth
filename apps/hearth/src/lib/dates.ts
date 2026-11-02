// Grouping conversations by when they were last used, like the sidebar of most chat apps.

const DAY = 86_400_000;

export type DateGroup<T> = { label: string; items: T[] };

function startOfDay(time: number): number {
  const date = new Date(time);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** Pinned items first, then Today / Yesterday / Previous 7 days / Previous 30 days / Older. */
export function groupByDate<T extends { updatedAt: number; pinned: boolean }>(
  items: T[],
  now: number,
): DateGroup<T>[] {
  const today = startOfDay(now);
  const buckets: [string, (t: number) => boolean][] = [
    ['Today', (t) => t >= today],
    ['Yesterday', (t) => t >= today - DAY],
    ['Previous 7 days', (t) => t >= today - 7 * DAY],
    ['Previous 30 days', (t) => t >= today - 30 * DAY],
    ['Older', () => true],
  ];
  const groups: DateGroup<T>[] = [];
  const pinned = items.filter((item) => item.pinned);
  if (pinned.length) groups.push({ label: 'Pinned', items: pinned });
  for (const [label, test] of buckets) {
    const matching = items.filter(
      (item) => !item.pinned && test(item.updatedAt) && !groups.some((g) => g.items.includes(item)),
    );
    if (matching.length) groups.push({ label, items: matching });
  }
  return groups;
}

/** "12s", "1m 5s". */
export function formatDuration(ms: number): string {
  const seconds = Math.max(1, Math.round(ms / 1000));
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}
