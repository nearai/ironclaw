/**
 * Bootstrapping `@ledgerhq/coin-evm` outside Ledger Live.
 *
 * `createApi` refuses to construct until a currencies resolver is registered
 * ("Framework currencies resolver not initialized"). Inside Ledger Live that
 * comes from `@ledgerhq/cryptoassets`; that package is NOT in coin-evm's
 * dependency tree, so a sidecar supplies the five-method interface itself.
 *
 * Verified by the §E5 spike: with this resolver in place, `createApi` runs in
 * plain Node with no app context and exposes the whole `CoinModuleApi`.
 *
 * Keeping the currency table explicit here — rather than pulling the full
 * cryptoassets catalogue — also means this process only knows about chains we
 * deliberately support.
 */

import { createRequire } from "node:module";

// The Ledger packages are CommonJS; this module is ESM. `createRequire` is the
// supported bridge, and it keeps the heavy dependency tree lazily loaded so
// this file stays unit-testable without it present.
const require = createRequire(import.meta.url);

// The framework's resolver interface. Typed structurally so the sidecar does
// not depend on Ledger Live's internal type layout.
type CryptoCurrency = {
  type: "CryptoCurrency";
  id: string;
  coinType: number;
  name: string;
  managerAppName: string;
  ticker: string;
  scheme: string;
  color: string;
  family: string;
  ethereumLikeInfo?: { chainId: number };
  explorerViews: unknown[];
  units: Array<{ name: string; code: string; magnitude: number }>;
};

export type SupportedChain = {
  /** Ledger currency id, e.g. `ethereum_sepolia`. */
  currencyId: string;
  /** EVM chain id — the exact-chain axis Rust binds and re-checks. */
  chainId: number;
  /** JSON-RPC endpoint for this chain. */
  rpcUri: string;
};

function currencyFor(chain: SupportedChain): CryptoCurrency {
  return {
    type: "CryptoCurrency",
    id: chain.currencyId,
    coinType: 60,
    name: chain.currencyId,
    managerAppName: "Ethereum",
    ticker: "ETH",
    scheme: chain.currencyId,
    color: "#627eea",
    family: "evm",
    ethereumLikeInfo: { chainId: chain.chainId },
    explorerViews: [],
    units: [{ name: "ETH", code: "ETH", magnitude: 18 }],
  };
}

/**
 * Register the currencies resolver for `chains`.
 *
 * Fails closed on an unknown currency id rather than returning `undefined` from
 * the throwing accessor — an unknown chain must surface as an error, never as a
 * silently mis-configured API.
 */
export function installCurrenciesResolver(chains: SupportedChain[]): void {
  const byId = new Map(chains.map((chain) => [chain.currencyId, currencyFor(chain)]));

  const { setCurrenciesResolver } = require(
    "@ledgerhq/ledger-wallet-framework/lib/currencies/resolver",
  );

  setCurrenciesResolver({
    getCryptoCurrencyById(id: string) {
      const currency = byId.get(id);
      if (!currency) {
        throw new Error(`unsupported currency: ${id}`);
      }
      return currency;
    },
    findCryptoCurrencyById: (id: string) => byId.get(id),
    findCryptoCurrencyByScheme: (scheme: string | undefined) =>
      scheme === undefined ? undefined : [...byId.values()].find((c) => c.scheme === scheme),
    listCryptoCurrencies: () => [...byId.values()],
    hasCryptoCurrencyId: (id: string) => byId.has(id),
  });
}

/**
 * Build the per-chain `CoinModuleApi` handles.
 *
 * Note the config shape, which the spike had to discover: the RPC endpoint is
 * `node: { type: "external", uri }` — NOT `info.rpc` — and `explorer: { type:
 * "none" }` is valid for a sidecar that never lists historical operations.
 */
export function createChainApis(chains: SupportedChain[]): Map<string, unknown> {
  installCurrenciesResolver(chains);
  // Path has no `.js`: coin-evm's exports map appends the extension itself.
  const { createApi } = require("@ledgerhq/coin-evm/lib/api/index");

  const apis = new Map<string, unknown>();
  for (const chain of chains) {
    apis.set(
      chain.currencyId,
      createApi(
        {
          info: { chainId: chain.chainId },
          node: { type: "external", uri: chain.rpcUri },
          explorer: { type: "none" },
        },
        chain.currencyId,
      ),
    );
  }
  return apis;
}
