import type { SVGProps } from 'react';

/** Hearth's flame mark: the outer flame takes the text color, the core is cut out. */
export function HearthMark(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true" {...props}>
      <path
        fill="currentColor"
        d="M12 2C13.5 5.5 18 8.5 18 14C18 17.9 15.3 21 12 21C8.7 21 6 17.9 6 14C6 11.3 7.3 9.4 8.8 8.1C8.9 10 9.7 11.4 10.9 12C10.4 8.6 10.6 5.4 12 2Z"
      />
      <path
        fill="var(--sidebar)"
        d="M12.3 12.5C13 14.2 15 15.2 15 17.4C15 19.2 13.7 20.4 12 20.4C10.3 20.4 9 19.2 9 17.6C9 16.4 9.6 15.6 10.4 15.1C10.5 16 10.9 16.5 11.5 16.7C11.3 15.2 11.5 13.8 12.3 12.5Z"
      />
    </svg>
  );
}
