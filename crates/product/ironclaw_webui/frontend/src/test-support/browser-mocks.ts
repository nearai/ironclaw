/** A standards-complete in-memory Storage fake with an inspection helper. */
export interface MemoryStorage extends Storage {
  dump(): Record<string, string>;
}

export function createMemoryStorage(
  initial: Record<string, string> = {},
): MemoryStorage {
  const values = new Map(Object.entries(initial));
  return {
    get length() {
      return values.size;
    },
    clear() {
      values.clear();
    },
    getItem(key) {
      return values.get(key) ?? null;
    },
    key(index) {
      return Array.from(values.keys())[index] ?? null;
    },
    removeItem(key) {
      values.delete(key);
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
    dump() {
      return Object.fromEntries(values);
    },
  };
}

/**
 * Replace a browser global for the duration of a test and return a precise
 * restore callback. This supports deliberately partial browser fakes without
 * weakening the DOM types used by production code.
 */
export function replaceBrowserGlobal<T>(name: PropertyKey, value: T): () => void {
  const prior = Object.getOwnPropertyDescriptor(globalThis, name);
  Object.defineProperty(globalThis, name, {
    configurable: true,
    value,
    writable: true,
  });
  return () => {
    if (prior) {
      Object.defineProperty(globalThis, name, prior);
    } else {
      Reflect.deleteProperty(globalThis, name);
    }
  };
}

/** Build the deliberately small DOM shape a pure helper test needs. */
export function domFixture<T extends Element>(fixture: Partial<T>): T {
  return fixture as T;
}
