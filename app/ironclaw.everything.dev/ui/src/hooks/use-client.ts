import { useSyncExternalStore } from "react";

const emptySubscribe = () => () => {};

export function useIsClient(): boolean {
  return useSyncExternalStore(
    emptySubscribe,
    () => true,
    () => false,
  );
}

export function useClientValue<T>(clientValue: () => T, serverValue: T): T {
  return useSyncExternalStore(emptySubscribe, clientValue, () => serverValue);
}

export function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    emptySubscribe,
    () => window.matchMedia(query).matches,
    () => false,
  );
}

export function usePrefersDarkMode(): boolean {
  return useMediaQuery("(prefers-color-scheme: dark)");
}

export function useLocalStorage<T>(key: string, fallback: T): T {
  return useSyncExternalStore(
    emptySubscribe,
    () => {
      const stored = localStorage.getItem(key);
      if (stored === null) return fallback;
      try {
        return JSON.parse(stored) as T;
      } catch {
        return stored as T;
      }
    },
    () => fallback,
  );
}
