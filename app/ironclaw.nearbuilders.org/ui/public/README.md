# everything.dev

everything.dev is an open runtime for apps on NEAR.

The bootstrap root is published from `dev.everything.near/everything.dev` and composed at runtime from public configuration rather than a single fixed bundle.

## What it is

- A site for browsing and inspecting published runtimes
- A product surface built from a host, remote UI, and remote API
- A public reference for runtime composition on NEAR

## How it works

1. A published `bos.config.json` record defines the runtime.
2. The bootstrap root is published first, without `extends`.
3. Other configs can extend that root record once it exists.
3. The UI loads through Module Federation.
4. The API loads through `every-plugin`.
5. Public metadata can be layered on without replacing the canonical runtime record.

## Why it matters

- Runtime configuration stays public and inspectable.
- Sites can share the same host while changing composition through config.
- UI and API can evolve independently.
- The system can keep being built over time because composition is externalized.
- Integrity hashes prove what runs matches what was published.
- Agents compose from verified primitives instead of building insecure bundles from scratch.

## Public files

- `/README.md` - human-readable overview
- `/skill.md` - agent-oriented usage notes
- `/llms.txt` - concise machine-ingestible summary
- `/manifest.json` - install and browser metadata

## Related ideas

- BOS — composable on-chain frontends
- web4 — verifiable on-chain hosting
- NEAR Intents — cross-chain settlement
- near-dns — blockchain-backed DNS
- NameSky — composable identity
- OutLayer — verifiable off-chain computation
- `every-plugin` — typed plugin contracts
- better-near-auth — cryptographic identity + gasless relay

## Canonical context

- Bootstrap runtime: `dev.everything.near/everything.dev`
- Stable host URLs can be reused across multiple sites
- Composition happens through published config, not rebuild-only deployment
