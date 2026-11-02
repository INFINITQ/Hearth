import { code } from '@streamdown/code';
import { createMathPlugin } from '@streamdown/math';
import { ImageIcon } from 'lucide-react';
import { type ComponentProps, memo } from 'react';
import { Streamdown } from 'streamdown';
import { backend } from '../lib/backend';
import { normalizeMath } from '../lib/mathText';

// Local models write inline math as $…$ (and \(…\)), so single dollars count as math.
const plugins = { code, math: createMathPlugin({ singleDollarTextMath: true }) };

// Copy buttons only: downloads can't leave the webview, and tables stay inline.
const controls = {
  code: { copy: true, download: false },
  table: { copy: true, download: false, fullscreen: false },
  mermaid: false,
} as const;

function open(href: string | undefined) {
  if (href)
    backend()
      .openLink(href)
      .catch(() => {});
}

/** Links open in your browser (never inside Hearth). */
function Link({ href, children }: ComponentProps<'a'>) {
  return (
    <a
      href={href}
      title={href}
      className="font-medium text-accent-text underline decoration-accent/40 underline-offset-2 hover:decoration-accent"
      onClick={(event) => {
        event.preventDefault();
        open(href);
      }}
    >
      {children}
    </a>
  );
}

/** Images in model output are never fetched (they could track you): they show as links. */
function Image({ src, alt }: ComponentProps<'img'>) {
  const href = typeof src === 'string' ? src : undefined;
  return (
    <a
      href={href}
      title={href}
      className="inline-flex items-center gap-1 rounded-md bg-surface-2 px-1.5 py-0.5 text-[13px] text-text-2 no-underline hover:text-text"
      onClick={(event) => {
        event.preventDefault();
        open(href);
      }}
    >
      <ImageIcon className="size-3.5" />
      {alt || 'Image'}
    </a>
  );
}

const components = { a: Link, img: Image };

/** Model output as rich text: GFM, highlighted code, KaTeX math; safe while still streaming. */
export const Markdown = memo(function Markdown({
  text,
  streaming = false,
}: {
  text: string;
  streaming?: boolean;
}) {
  return (
    <Streamdown
      className="hearth-markdown"
      plugins={plugins}
      controls={controls}
      components={components}
      linkSafety={{ enabled: false }}
      skipHtml
      isAnimating={streaming}
      mode={streaming ? 'streaming' : 'static'}
    >
      {normalizeMath(text)}
    </Streamdown>
  );
});
