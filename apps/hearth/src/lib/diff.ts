// Unified diffs (as hearth-core's tools write them) split into rows for display.

export type DiffRow =
  { kind: 'gap' } | { kind: 'add' | 'del' | 'same'; line: number; text: string };

/** Rows with line numbers: the new file's for added and unchanged lines, the old file's for removed ones. */
export function parseDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldLine = 0;
  let newLine = 0;
  let inHunk = false;
  const lines = diff.split('\n');
  lines.forEach((raw, index) => {
    const line = raw.endsWith('\r') ? raw.slice(0, -1) : raw;
    if (line.startsWith('--- ') || line.startsWith('+++ ') || line.startsWith('\\')) return;
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (hunk) {
      if (rows.length) rows.push({ kind: 'gap' });
      oldLine = Number(hunk[1]);
      newLine = Number(hunk[2]);
      inHunk = true;
    } else if (line.startsWith('+')) {
      rows.push({ kind: 'add', line: newLine++, text: line.slice(1) });
    } else if (line.startsWith('-')) {
      rows.push({ kind: 'del', line: oldLine++, text: line.slice(1) });
    } else if (line.startsWith(' ') || (line === '' && inHunk && index < lines.length - 1)) {
      // An unchanged blank line can arrive without its leading space.
      rows.push({ kind: 'same', line: newLine++, text: line.slice(1) });
      oldLine++;
    }
  });
  return rows;
}

export function diffStats(diff: string): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const row of parseDiff(diff)) {
    if (row.kind === 'add') added++;
    else if (row.kind === 'del') removed++;
  }
  return { added, removed };
}
