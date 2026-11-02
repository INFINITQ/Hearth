// The models guide (docs/models.md) inside the app: installing llama.cpp, which models suit
// this PC, and how to get them. Bundled with the app, so it reads offline.

import guide from '../../../../docs/models.md?raw';
import { Markdown } from '../components/Markdown';

export function GuideScreen() {
  return (
    <div className="min-h-0 flex-1 overflow-y-auto" role="document" aria-label="Guide">
      <article className="guide mx-auto max-w-[760px] px-8 pb-20 pt-6 text-[15px]">
        <Markdown text={guide} streaming={false} />
      </article>
    </div>
  );
}
