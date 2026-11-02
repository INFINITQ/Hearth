// Models write LaTeX with \( … \) and \[ … \] as often as with dollars. The markdown renderer
// only knows dollars, so the other forms are converted, everywhere except inside code.

const FENCE = /(```[\s\S]*?(?:```|$)|`[^`\n]*`)/g;

export function normalizeMath(text: string): string {
  if (!text.includes('\\(') && !text.includes('\\[')) return text;
  return text
    .split(FENCE)
    .map((part, index) =>
      index % 2 === 1
        ? part
        : part
            .replace(/\\\[([\s\S]+?)\\\]/g, (_, body: string) => `$$${body}$$`)
            .replace(/\\\(([\s\S]+?)\\\)/g, (_, body: string) => `$${body.trim()}$`),
    )
    .join('');
}
