# Wire-contract golden fixtures

These JSON files are the **shared** definition of the Alpaca wire contract. Both
sides parse the same bytes:

* the sidecar's vitest suite (`test/fixtures.test.ts`), and
* the Rust client's test (`crates/ironclaw_attested_runtime/src/alpaca_uds.rs`).

That is the point: the sidecar is TypeScript and the caller is Rust, so nothing
in the type system connects them. A fixture both suites read is the only thing
that makes a silent divergence impossible — change the shape on one side and the
other side's test fails.

If you change a fixture, you are changing the contract. Bump `version` and
update both suites deliberately.
