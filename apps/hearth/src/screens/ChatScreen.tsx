import { Composer } from '../components/Composer';
import { useApp } from '../stores/app';

/** A new conversation: a greeting and the message box. */
export function ChatScreen() {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[14vh]">
      <div className="w-full max-w-[720px]">
        <h1 className="mb-7 text-center text-[27px] font-semibold tracking-[-0.022em] text-text">
          What's on your mind?
        </h1>
        <Composer disabled />
        <p className="mt-3.5 text-center text-[12.5px] text-text-3">
          No model is loaded yet.{' '}
          <button
            type="button"
            className="font-medium text-accent-text hover:underline"
            onClick={() => useApp.getState().go('models')}
          >
            Set one up
          </button>
        </p>
      </div>
    </div>
  );
}
