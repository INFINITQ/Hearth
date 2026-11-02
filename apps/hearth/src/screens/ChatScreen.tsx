import { Composer } from '../components/Composer';
import { ParamsPopover } from '../components/ParamsPopover';
import { RunErrorBanner, Transcript } from '../components/Transcript';
import { useApp } from '../stores/app';
import { useChat } from '../stores/chat';

function send(text: string) {
  void useChat.getState().send(text, 'chat');
}

function stop() {
  useChat.getState().stop();
}

/** A conversation, or a greeting and the message box for a new one. */
export function ChatScreen() {
  const session = useChat((s) => s.session);
  const messages = useChat((s) => s.messages);
  const run = useChat((s) => s.run);
  const started = session?.mode === 'chat' && (messages.length > 0 || run !== null);

  if (!started) return <NewChat />;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <Transcript />
      <div className="mx-auto w-full max-w-[760px] px-6 pb-3">
        <Composer
          running={run !== null}
          onSend={send}
          onStop={stop}
          tools={<ParamsPopover />}
          autoFocus
        />
        <p className="mt-2 text-center text-[11.5px] text-text-3">
          Runs on your PC. Replies can be wrong, so check what matters.
        </p>
      </div>
    </div>
  );
}

function NewChat() {
  const model = useApp((s) => s.settings.models.chat);
  const run = useChat((s) => s.run);
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center px-6 pb-[14vh]">
      <div className="w-full max-w-[720px]">
        <h1 className="mb-7 text-center text-[27px] font-semibold tracking-[-0.022em] text-text">
          What's on your mind?
        </h1>
        <Composer
          running={run !== null}
          onSend={send}
          onStop={stop}
          tools={<ParamsPopover />}
          autoFocus
        />
        <div className="mt-4">
          <RunErrorBanner />
        </div>
        {!model && (
          <p className="mt-3.5 text-center text-[12.5px] text-text-3">
            No model is set up yet.{' '}
            <button
              type="button"
              className="font-medium text-accent-text hover:underline"
              onClick={() => useApp.getState().go('settings')}
            >
              Choose one
            </button>
          </p>
        )}
      </div>
    </div>
  );
}
