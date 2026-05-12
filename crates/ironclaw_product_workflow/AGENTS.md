# ironclaw_product_workflow Agent Notes

- This crate is the product-facing Reborn workflow facade between product adapters and host-layer services.
- Keep product adapters thin: binding resolution, inbound message staging, turn submission, idempotency, and product-safe acknowledgements belong here.
- Do not add dependencies on dispatcher, extensions, host_runtime, mcp, wasm, scripts, network, engine, or gateway crates.
- User-message acceptance must persist canonical thread content through `ironclaw_threads::SessionThreadService` before turn submission.
- Do not return a successful product ack unless the inbound action has a durable terminal ledger outcome.
