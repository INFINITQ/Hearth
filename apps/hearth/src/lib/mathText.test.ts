import { describe, expect, it } from 'vitest';
import { normalizeMath } from './mathText';

describe('normalizeMath', () => {
  it('turns LaTeX delimiters into dollars', () => {
    expect(normalizeMath('Inline \\( x^2 \\) and block \\[a+b\\]')).toBe(
      'Inline $x^2$ and block $$a+b$$',
    );
  });

  it('leaves code alone', () => {
    const text = 'See `\\(not math\\)` and\n```\nprintf("\\(x\\)")\n```\nbut \\(y\\)';
    expect(normalizeMath(text)).toBe(
      'See `\\(not math\\)` and\n```\nprintf("\\(x\\)")\n```\nbut $y$',
    );
  });

  it('is a no-op for plain text', () => {
    expect(normalizeMath('Costs $5 and $10')).toBe('Costs $5 and $10');
  });
});
