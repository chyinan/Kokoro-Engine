// @vitest-environment jsdom
// pattern: Imperative Shell
// Review regression: a delayed local event subscription must be released after unmount.
import { act, createElement } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { useTypingReveal } from '../../../../src/ui/hooks/useTypingReveal';

const mocks = vi.hoisted(() => ({ listen: vi.fn() }));
vi.mock('@tauri-apps/api/event', () => ({ listen: mocks.listen }));

let container: HTMLDivElement;
let root: Root;
let mounted = false;
let resolveSubscriptions: Array<(unlisten: () => void) => void>;

function TypingProbe() {
  useTypingReveal({ active: false, onReveal: () => {} });
  return null;
}

beforeEach(() => {
  vi.stubGlobal('IS_REACT_ACT_ENVIRONMENT', true);
  resolveSubscriptions = [];
  mocks.listen.mockReset().mockImplementation(() => new Promise<() => void>((resolve) => {
    resolveSubscriptions.push(resolve);
  }));
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(async () => {
  if (mounted) await act(async () => root.unmount());
  mounted = false;
  container.remove();
  vi.unstubAllGlobals();
});

it('releases both event subscriptions that finish registering after unmount', async () => {
  await act(async () => {
    root.render(createElement(TypingProbe));
    mounted = true;
  });
  expect(mocks.listen.mock.calls.map(([eventName]) => eventName)).toEqual(['chat-typing', 'chat-cue']);
  expect(resolveSubscriptions).toHaveLength(2);

  await act(async () => root.unmount());
  mounted = false;
  const unlistenTyping = vi.fn();
  const unlistenCue = vi.fn();
  await act(async () => {
    resolveSubscriptions[0](unlistenTyping);
    resolveSubscriptions[1](unlistenCue);
  });

  expect(unlistenTyping).toHaveBeenCalledOnce();
  expect(unlistenCue).toHaveBeenCalledOnce();
});
