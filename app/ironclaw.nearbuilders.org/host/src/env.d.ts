/// <reference types="@rsbuild/core/types" />

interface ImportMetaEnv {
  readonly MODE: "development" | "production";
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

export {};
