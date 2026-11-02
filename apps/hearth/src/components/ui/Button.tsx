import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { cn } from '../../lib/cn';

type Variant = 'primary' | 'secondary' | 'ghost' | 'danger';
type Size = 'sm' | 'md';

const VARIANTS: Record<Variant, string> = {
  primary: 'bg-accent-strong text-on-accent hover:bg-accent-strong-hover',
  secondary: 'bg-surface text-text shadow-[0_0_0_1px_var(--line)] hover:bg-surface-2',
  ghost: 'text-text-2 hover:bg-surface-3/70 hover:text-text',
  danger: 'bg-danger text-on-danger hover:opacity-90',
};

const SIZES: Record<Size, string> = {
  sm: 'h-7 gap-1.5 px-2.5 text-[12.5px] [&_svg]:size-3.5',
  md: 'h-8 gap-2 px-3 text-[13px] [&_svg]:size-4',
};

type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: Size };

export function Button({ variant = 'secondary', size = 'md', className, ...props }: ButtonProps) {
  return (
    <button
      type="button"
      className={cn(
        'inline-flex shrink-0 items-center justify-center rounded-control font-medium transition-colors duration-150 disabled:pointer-events-none disabled:opacity-45',
        VARIANTS[variant],
        SIZES[size],
        className,
      )}
      {...props}
    />
  );
}

type IconButtonProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'> & {
  /** Accessible name, also shown as the tooltip. */
  label: string;
  shortcut?: string;
  active?: boolean;
  children: ReactNode;
};

/** A small square button holding one icon. */
export function IconButton({
  label,
  shortcut,
  active,
  className,
  children,
  ...props
}: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={shortcut ? `${label} (${shortcut})` : label}
      aria-pressed={active}
      className={cn(
        'inline-grid size-7 shrink-0 place-items-center rounded-[7px] text-text-2 transition-colors duration-150 hover:bg-surface-3/70 hover:text-text disabled:pointer-events-none disabled:opacity-40 [&_svg]:size-[17px] [&_svg]:stroke-[1.75]',
        active && 'bg-surface-3 text-text',
        className,
      )}
      {...props}
    >
      {children}
    </button>
  );
}
