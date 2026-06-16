import { useState } from "react";
import { useClientValue } from "@/hooks/use-client";

const STORAGE_KEY = "ironclaw-verbose";

export function useVerboseMode() {
  const initial = useClientValue(() => localStorage.getItem(STORAGE_KEY) === "1", false);
  const [verbose, setVerbose] = useState(initial);

  const toggle = () => {
    setVerbose((v) => {
      const next = !v;
      if (typeof window !== "undefined") {
        localStorage.setItem(STORAGE_KEY, next ? "1" : "0");
      }
      return next;
    });
  };

  return { verbose, toggle };
}
