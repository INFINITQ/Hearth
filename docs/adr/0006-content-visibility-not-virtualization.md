# 0006. Long transcripts: memoized messages and content-visibility, not a virtual list

Status: accepted (2026-11-03)

## Context

With 300 messages (replies with tables, code and math), every streamed token took 40 ms in the
development build: each token gave the store a new message list, so every message rendered
again, and the browser laid out about 33,000 elements. A virtual list (react-virtuoso,
TanStack Virtual) renders only the messages on screen, but needs their heights measured as
they change (streaming text, highlighted code, expanding tool output, images loading),
takes messages off screen out of the page (so out of text selection and the accessibility
tree), and fights the "stay at the newest text unless you scroll up" behaviour.

## Decision

- Messages keep their identity until they change, and the message components are memoized,
  so a token renders only the reply being written. Values rebuilt on every change (each
  step's answers, web turns) are compared by contents.
- Messages away from the screen use CSS `content-visibility: auto` with
  `contain-intrinsic-size: auto 320px`: the browser skips their layout and painting and keeps
  their size once shown. `overflow-clip-margin: 32px` stops the containment from cutting off
  focus rings and shadows at a message's edge. The newest six messages always render, so
  opening a conversation lands exactly at its end.

## Consequences

- A token costs about 2 ms with 300 messages, as with none; every message stays in the page,
  so selection and screen readers work across the whole conversation.
- Opening a very long conversation still renders all of its Markdown once (about 0.6 s for
  300 rich messages in the development build). Rendering off-screen messages as plain text
  until they come near (`contentvisibilityautostatechange`) is the next step if that matters.
- After a big jump (Ctrl+End through hundreds of messages) the browser may show one frame
  before the messages arriving on screen are painted.
- `Transcript.test.tsx` counts renders, so a change that renders the whole conversation on
  every token again fails a test.
