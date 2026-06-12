# Template Plugin

Minimal starter for an `every-plugin` plugin that works both on its own and inside `bos dev`.

## Included

```bash
src/
├── contract.ts
├── service.ts
├── index.ts
└── LLM.txt
plugin.dev.ts
rspack.config.cjs
tests/
```

## Standalone Dev

```bash
cd plugins/my-plugin
bun install
bun run dev
```

The plugin dev server prefers `PORT` from the environment. If none is provided, `plugin.dev.ts` supplies a local default.

## Composed Dev With `bos dev`

Add the plugin to the root `bos.config.json`:

```json
{
  "plugins": {
    "my-plugin": {
      "development": "local:plugins/my-plugin"
    }
  }
}
```

Then run:

```bash
bos dev
```

`everything-dev` will:

1. detect `local:plugins/my-plugin`
2. choose an available localhost port
3. spawn the plugin with `PORT=<chosen port>`
4. load it into the host and stitch it into `/api`
5. shut it down when the dev session ends

If you already run the plugin yourself, use a URL instead:

```json
{
  "plugins": {
    "my-plugin": {
      "development": "http://localhost:3021"
    }
  }
}
```

That tells the host to load the plugin from that URL without spawning it.

## Root Local Targets

The workspace now uses the same pattern for built-ins:

```json
{
  "app": {
    "ui": { "development": "local:ui" },
    "api": { "development": "local:api" }
  }
}
```

## Runtime Example

```ts
import { createPluginRuntime } from "every-plugin";

const runtime = createPluginRuntime({
  registry: {
    "my-plugin": {
      remote: "http://localhost:3021/remoteEntry.js",
    },
  },
  secrets: {},
});

const { createClient } = await runtime.usePlugin("my-plugin", {
  variables: { baseUrl: "https://api.example.com", timeout: 10000 },
  secrets: { apiKey: "your-key" },
});

const client = createClient();
await client.ping();
```

## Build Checklist

1. Update `src/contract.ts`
2. Update `src/service.ts`
3. Update `src/index.ts`
4. Update `package.json` name
5. Update `plugin.dev.ts` config
6. Add the plugin to the root `bos.config.json` if you want composed dev

## Docs

See `LLM.txt` for the longer implementation guide.
