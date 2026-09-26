import { lazy, Suspense, type ComponentType } from "react";

/**
 * A component whose code loads when it's first rendered, so the first paint
 * only waits for the shell. It renders nothing for the few ms that takes.
 */
export function lazyComponent<P extends object>(load: () => Promise<ComponentType<P>>): ComponentType<P> {
  const Lazy = lazy(() => load().then((component) => ({ default: component })));
  return (props: P) => (
    <Suspense fallback={null}>
      <Lazy {...props} />
    </Suspense>
  );
}
