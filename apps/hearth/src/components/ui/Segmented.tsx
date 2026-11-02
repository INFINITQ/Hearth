import { type KeyboardEvent, type ReactNode, useRef } from 'react';
import { cn } from '../../lib/cn';

type Option<T extends string> = { value: T; label: string; icon?: ReactNode };

type SegmentedProps<T extends string> = {
  /** Accessible name of the group. */
  label: string;
  value: T;
  options: Option<T>[];
  onChange(value: T): void;
  className?: string;
};

/** A row of mutually exclusive choices (a radio group that looks like a switch). */
export function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
  className,
}: SegmentedProps<T>) {
  const buttons = useRef<(HTMLButtonElement | null)[]>([]);

  const onKeyDown = (event: KeyboardEvent, index: number) => {
    const step = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    const next = (index + step + options.length) % options.length;
    const option = options[next];
    if (option) {
      onChange(option.value);
      buttons.current[next]?.focus();
    }
  };

  return (
    <div
      role="radiogroup"
      aria-label={label}
      className={cn('flex rounded-[10px] bg-segment-track p-[3px]', className)}
    >
      {options.map((option, index) => {
        const checked = option.value === value;
        return (
          <button
            key={option.value}
            ref={(el) => {
              buttons.current[index] = el;
            }}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            onClick={() => onChange(option.value)}
            onKeyDown={(event) => onKeyDown(event, index)}
            className={cn(
              'flex h-7 flex-1 items-center justify-center gap-1.5 rounded-[7px] px-2 text-[13px] font-medium transition-[background-color,color,box-shadow] duration-150 [&_svg]:size-[15px] [&_svg]:stroke-[1.75]',
              checked
                ? 'bg-segment-thumb text-text shadow-[0_1px_2px_rgba(0,0,0,0.08),0_0_0_1px_var(--line)]'
                : 'text-text-2 hover:text-text',
            )}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
