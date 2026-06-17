const STORAGE_PREFIX = "chat-draft:";

export function loadDraft(threadId: string): string {
  try {
    return localStorage.getItem(`${STORAGE_PREFIX}${threadId}`) ?? "";
  } catch {
    return "";
  }
}

export function saveDraft(threadId: string, text: string): void {
  try {
    if (text.length > 0) {
      localStorage.setItem(`${STORAGE_PREFIX}${threadId}`, text);
    } else {
      localStorage.removeItem(`${STORAGE_PREFIX}${threadId}`);
    }
  } catch {}
}

export function clearDraft(threadId: string): void {
  try {
    localStorage.removeItem(`${STORAGE_PREFIX}${threadId}`);
  } catch {}
}
