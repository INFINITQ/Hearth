import { describe, expect, it } from 'vitest';
import { diffStats, parseDiff } from './diff';

const DIFF = `--- a/src/calc.py
+++ b/src/calc.py
@@ -1,3 +1,3 @@
 def add(a, b):
-    return a - b
+    return a + b

@@ -20,2 +20,3 @@
 x = 1
+y = 2
 z = 3
`;

describe('parseDiff', () => {
  it('numbers lines from the hunk headers and marks a gap between hunks', () => {
    const rows = parseDiff(DIFF);
    expect(rows.slice(0, 4)).toEqual([
      { kind: 'same', line: 1, text: 'def add(a, b):' },
      { kind: 'del', line: 2, text: '    return a - b' },
      { kind: 'add', line: 2, text: '    return a + b' },
      { kind: 'same', line: 3, text: '' },
    ]);
    expect(rows[4]).toEqual({ kind: 'gap' });
    expect(rows.slice(5)).toEqual([
      { kind: 'same', line: 20, text: 'x = 1' },
      { kind: 'add', line: 21, text: 'y = 2' },
      { kind: 'same', line: 22, text: 'z = 3' },
    ]);
  });

  it('counts what changed', () => {
    expect(diffStats(DIFF)).toEqual({ added: 2, removed: 1 });
    expect(diffStats('')).toEqual({ added: 0, removed: 0 });
  });
});
