import type { StorybookConfig } from '@storybook/react-vite';

// `@storybook/addon-mcp` registers an unauthenticated `/mcp` endpoint on the
// `storybook dev` server that can trigger the story-test suite. It is useful
// for driving the catalog from a local coding agent, but it must NOT be on by
// default — a Storybook dev server bound beyond loopback would expose a remote
// story-test DoS surface. Enable it explicitly for local agent work with
// `STORYBOOK_MCP=1 pnpm storybook`; it is never included in `storybook build`
// or the CI story-test run.
const enableMcpAddon = process.env.STORYBOOK_MCP === "1";

const config: StorybookConfig = {
  "stories": [
    "../src/**/*.stories.@(js|jsx|mjs|ts|tsx)"
  ],
  "addons": [
    "@chromatic-com/storybook",
    "@storybook/addon-vitest",
    "@storybook/addon-a11y",
    "@storybook/addon-docs",
    ...(enableMcpAddon ? ["@storybook/addon-mcp"] : []),
  ],
  "framework": "@storybook/react-vite"
};
export default config;
