import { type ClassValue, clsx } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

// tailwind-merge must know Hearth's design tokens, or it could treat `text-text-2` (a color)
// and `text-[13px]` (a size) as the same kind of class and drop one.
const merge = extendTailwindMerge({
  extend: {
    theme: {
      color: [
        'bg',
        'sidebar',
        'surface',
        'surface-2',
        'surface-3',
        'text',
        'text-2',
        'text-3',
        'line',
        'line-strong',
        'accent',
        'accent-hover',
        'accent-soft',
        'accent-text',
        'on-accent',
        'danger',
        'danger-soft',
        'success',
        'segment-track',
        'segment-thumb',
      ],
      shadow: ['float', 'pop'],
      radius: ['card', 'control'],
    },
  },
});

/** Joins class names, letting later Tailwind classes override earlier ones. */
export function cn(...inputs: ClassValue[]) {
  return merge(clsx(inputs));
}
