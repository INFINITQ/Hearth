import { AlertDialog } from 'radix-ui';
import { useChat } from '../../stores/chat';
import { Button } from '../ui/Button';

/** Asked the first time a folder is opened: the agent follows what the project's files say. */
export function TrustDialog() {
  const pending = useChat((s) => s.pendingTrust);
  return (
    <AlertDialog.Root
      open={pending !== null}
      onOpenChange={(open) => !open && useChat.getState().cancelTrust()}
    >
      <AlertDialog.Portal>
        <AlertDialog.Overlay className="dialog-overlay" />
        <AlertDialog.Content className="dialog w-[440px]">
          <AlertDialog.Title className="text-[15px] font-semibold">
            Trust the files in {pending?.name}?
          </AlertDialog.Title>
          <AlertDialog.Description className="mt-1.5 text-[13.5px] leading-relaxed text-text-2">
            The agent reads the files in this folder and follows its AGENTS.md. Text in a project
            can try to steer the agent, so only open folders whose contents you trust.
          </AlertDialog.Description>
          <p
            className="mt-3 truncate rounded-control bg-surface-2 px-2.5 py-1.5 font-mono text-[12px] text-text-2"
            title={pending?.path}
          >
            {pending?.path}
          </p>
          <div className="mt-5 flex justify-end gap-2">
            <AlertDialog.Cancel asChild>
              <Button variant="ghost">Cancel</Button>
            </AlertDialog.Cancel>
            <AlertDialog.Action asChild>
              <Button variant="primary" onClick={() => void useChat.getState().trustPending()}>
                Trust and open
              </Button>
            </AlertDialog.Action>
          </div>
        </AlertDialog.Content>
      </AlertDialog.Portal>
    </AlertDialog.Root>
  );
}
