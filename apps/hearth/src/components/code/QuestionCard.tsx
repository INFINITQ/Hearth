import { ArrowUp, MessageCircleQuestionMark } from 'lucide-react';
import { useState } from 'react';
import type { Question } from '../../lib/types';
import { useChat } from '../../stores/chat';
import { Button } from '../ui/Button';

/** A question from the agent: pick an option or write an answer. */
export function QuestionCard({ question }: { question: Question }) {
  const [text, setText] = useState('');
  const [sent, setSent] = useState(false);
  const answer = (value: string) => {
    if (!value.trim()) return;
    setSent(true);
    void useChat.getState().answerQuestion(value.trim());
  };
  return (
    <section
      aria-label="Question from the agent"
      className="rounded-card bg-surface p-4 shadow-float ring-1 ring-line-strong"
    >
      <div className="flex items-start gap-3">
        <div className="grid size-8 shrink-0 place-items-center rounded-full bg-surface-2 text-text-2">
          <MessageCircleQuestionMark className="size-4" />
        </div>
        <div className="min-w-0 flex-1 pt-0.5">
          <p className="text-[12px] font-medium text-text-3">The agent asks</p>
          <p className="mt-0.5 whitespace-pre-wrap text-[14.5px] leading-[1.55] text-text select-text">
            {question.question}
          </p>
        </div>
      </div>
      {question.options.length > 0 && (
        <div className="mt-3.5 flex flex-wrap gap-2 pl-11">
          {question.options.map((option) => (
            <Button key={option} disabled={sent} onClick={() => answer(option)}>
              {option}
            </Button>
          ))}
        </div>
      )}
      <form
        className="ml-11 mt-2.5 flex h-9 items-center gap-2 rounded-control bg-surface-2 pl-3 pr-1.5"
        onSubmit={(event) => {
          event.preventDefault();
          answer(text);
        }}
      >
        <input
          value={text}
          disabled={sent}
          autoFocus={question.options.length === 0}
          onChange={(event) => setText(event.target.value)}
          aria-label="Your answer"
          placeholder={question.options.length ? 'Or write your own answer…' : 'Write your answer…'}
          className="min-w-0 flex-1 bg-transparent text-[13px] text-text outline-none select-text placeholder:text-text-3"
        />
        <button
          type="submit"
          aria-label="Send answer"
          disabled={sent || !text.trim()}
          className="grid size-6 place-items-center rounded-full bg-accent text-on-accent transition-colors hover:bg-accent-hover disabled:bg-surface-3 disabled:text-text-3"
        >
          <ArrowUp className="size-3.5 stroke-[2.25]" />
        </button>
      </form>
    </section>
  );
}
