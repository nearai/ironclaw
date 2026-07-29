/**
 * Shared harness for the extras smoke tests (happy-dom + createRoot).
 * Mirrors the pattern in src/composites/flow-list.test.tsx, with the act
 * environment flag set so React doesn't warn on every render.
 */
import { act, type ReactElement } from "react";
import { createRoot } from "react-dom/client";

declare global {
  // eslint-disable-next-line no-var
  var IS_REACT_ACT_ENVIRONMENT: boolean | undefined;
}

globalThis.IS_REACT_ACT_ENVIRONMENT = true;

export function renderIntoDocument(element: ReactElement) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(element));
  return {
    container,
    rerender(next: ReactElement) {
      act(() => root.render(next));
    },
    unmount() {
      act(() => root.unmount());
      container.remove();
    },
  };
}

/** Dispatch a bubbling event inside act() so React processes updates. */
export function fire(target: EventTarget, event: Event) {
  act(() => {
    target.dispatchEvent(event);
  });
}

export function click(target: Element) {
  fire(target, new MouseEvent("click", { bubbles: true, cancelable: true }));
}
