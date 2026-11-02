import { Check, ChevronsUpDown, FolderOpen, FolderPlus } from 'lucide-react';
import { DropdownMenu } from 'radix-ui';
import { cn } from '../../lib/cn';
import { useChat } from '../../stores/chat';

/** The sidebar's project: switch to a recent one, or open another folder. */
export function ProjectSwitcher() {
  const project = useChat((s) => s.project);
  const projects = useChat((s) => s.projects);

  if (!project) {
    return (
      <button
        type="button"
        onClick={() => void useChat.getState().pickProject()}
        className="flex h-9 w-full items-center gap-2.5 rounded-control px-2.5 text-[13px] text-text-2 transition-colors duration-150 hover:bg-surface-3/70 hover:text-text [&_svg]:size-4 [&_svg]:stroke-[1.75]"
      >
        <FolderPlus />
        Open a project…
      </button>
    );
  }
  return (
    <DropdownMenu.Root onOpenChange={(open) => open && void useChat.getState().loadProjects()}>
      <DropdownMenu.Trigger asChild>
        <button
          type="button"
          aria-label={`Project: ${project.name}`}
          title={project.path}
          className="flex h-9 w-full items-center gap-2.5 rounded-control px-2.5 text-left transition-colors duration-150 hover:bg-surface-3/70 data-[state=open]:bg-surface-3 [&_svg]:size-4 [&_svg]:stroke-[1.75]"
        >
          <FolderOpen className="shrink-0 text-text-2" />
          <span className="min-w-0 flex-1 truncate text-[13px] font-medium text-text">
            {project.name}
          </span>
          <ChevronsUpDown className="shrink-0 text-text-3" />
        </button>
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content align="start" sideOffset={4} className="menu w-[248px]">
          <DropdownMenu.Label className="menu-label">Recent projects</DropdownMenu.Label>
          {projects.map((item) => (
            <DropdownMenu.Item
              key={item.id}
              disabled={!item.exists}
              className="menu-item data-[disabled]:opacity-50"
              title={item.exists ? item.path : `${item.path} (missing)`}
              onSelect={() => void useChat.getState().openProject(item)}
            >
              <Check className={cn(item.id !== project.id && 'invisible')} />
              <span className="truncate">{item.name}</span>
              {!item.exists && <span className="ml-auto text-[11.5px] text-text-3">Missing</span>}
            </DropdownMenu.Item>
          ))}
          <DropdownMenu.Separator className="menu-sep" />
          <DropdownMenu.Item
            className="menu-item"
            onSelect={() => void useChat.getState().pickProject()}
          >
            <FolderPlus /> Open folder…
          </DropdownMenu.Item>
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
