import { ChevronDown, Circle, CircleCheck, CircleDot, ListTodo } from 'lucide-react';
import { useState } from 'react';
import { cn } from '../../lib/cn';
import type { Todo } from '../../lib/types';

export function TodoList({ todos }: { todos: Todo[] }) {
  return (
    <ul className="space-y-1" aria-label="To-do list">
      {todos.map((todo, index) => (
        <li
          key={index}
          data-status={todo.status}
          className="flex items-start gap-2 text-[13px] leading-[1.5]"
        >
          {todo.status === 'completed' ? (
            <CircleCheck className="mt-[3px] size-3.5 shrink-0 text-success" aria-label="Done" />
          ) : todo.status === 'in_progress' ? (
            <CircleDot
              className="mt-[3px] size-3.5 shrink-0 text-accent"
              aria-label="In progress"
            />
          ) : (
            <Circle className="mt-[3px] size-3.5 shrink-0 text-text-3" aria-label="To do" />
          )}
          <span
            className={cn(
              todo.status === 'completed' && 'text-text-3 line-through decoration-text-3/60',
              todo.status === 'in_progress' && 'font-medium text-text',
              todo.status === 'pending' && 'text-text-2',
            )}
          >
            {todo.content}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** The agent's to-do list, above the message box. */
export function TodoPanel({ todos }: { todos: Todo[] }) {
  const done = todos.filter((todo) => todo.status === 'completed').length;
  const [open, setOpen] = useState(true);
  const allDone = done === todos.length;
  return (
    <section
      aria-label="To-do"
      className="mb-2 rounded-card bg-surface px-3.5 py-2 ring-1 ring-line"
    >
      <button
        type="button"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
        className="flex w-full items-center gap-2 text-[12.5px] font-medium text-text-2"
      >
        <ListTodo className="size-3.5" />
        To-do
        <span className="font-normal text-text-3">
          {allDone ? `All ${todos.length} done` : `${done} of ${todos.length} done`}
        </span>
        <ChevronDown
          className={cn(
            'ml-auto size-3.5 text-text-3 transition-transform duration-200',
            !open && '-rotate-90',
          )}
        />
      </button>
      {open && (
        <div className="mt-2 max-h-[132px] overflow-y-auto pb-1">
          <TodoList todos={todos} />
        </div>
      )}
    </section>
  );
}
