const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const bs58 = require("bs58");
const nacl = require("tweetnacl");

const NEAR_CREDENTIALS_SUFFIX = "/secrets/near-credentials";

/**
 * @typedef {Object} NearImplicitAccountCredentials
 * @property {string} implicit_account_id 64-char hex (Ed25519 public key bytes)
 * @property {string} public_key ed25519: + base58(public)
 * @property {string} private_key ed25519: + base58(seed||public) — never return from agent tools
 */

/**
 * Generate NEAR implicit-account credentials (gennearaccount-compatible JSON fields).
 *
 * @param {Uint8Array} [seed] Optional 32-byte seed (for tests only)
 * @returns {NearImplicitAccountCredentials}
 */
function generateNearImplicitAccount(seed) {
  const seedBytes = seed ?? crypto.randomBytes(32);
  if (!(seedBytes instanceof Uint8Array) || seedBytes.length !== 32) {
    throw new Error("seed must be a 32-byte Uint8Array");
  }

  const keyPair = nacl.sign.keyPair.fromSeed(seedBytes);
  const implicit_account_id = Buffer.from(keyPair.publicKey).toString("hex");
  const public_key = `ed25519:${bs58.encode(keyPair.publicKey)}`;
  const private_key = `ed25519:${bs58.encode(keyPair.secretKey)}`;

  return { implicit_account_id, public_key, private_key };
}

/**
 * @param {string} resolved Absolute output directory path
 * @returns {boolean}
 */
function hasNearCredentialsDirSuffix(resolved) {
  const normalized = resolved.split(path.sep).join("/").replace(/\/+$/, "");
  return (
    normalized.endsWith(NEAR_CREDENTIALS_SUFFIX) ||
    normalized.endsWith("/near-credentials")
  );
}

/**
 * Resolve and validate an output directory for NEAR credential JSON files.
 *
 * Allowed when the path ends with `secrets/near-credentials` (recommended) or matches
 * an explicit operator allowlist prefix from plugin config.
 *
 * @param {string} outputDir
 * @param {{ allowedOutputDirs?: string[] }} [options]
 * @returns {string} Absolute resolved directory
 */
function validateNearCredentialsOutputDir(outputDir, options = {}) {
  const { allowedOutputDirs = [] } = options;

  if (typeof outputDir !== "string" || outputDir.trim().length === 0) {
    throw new Error("outputDir must be a non-empty string");
  }

  const resolved = path.resolve(outputDir.trim());
  const tmpRoot = path.resolve(os.tmpdir());
  if (resolved === tmpRoot) {
    throw new Error("Refusing to write NEAR credentials to the system temp directory root");
  }

  const allowedPrefixes = allowedOutputDirs
    .filter((entry) => typeof entry === "string" && entry.trim().length > 0)
    .map((entry) => path.resolve(entry.trim()));

  const underAllowlist = allowedPrefixes.some(
    (prefix) => resolved === prefix || resolved.startsWith(`${prefix}${path.sep}`)
  );

  if (!hasNearCredentialsDirSuffix(resolved) && !underAllowlist) {
    throw new Error(
      "Output directory must end with secrets/near-credentials or appear in nearCredentialsOutputDirs"
    );
  }

  return resolved;
}

/**
 * Write gennearaccount-compatible JSON to `<dir>/<implicit_account_id>.json`.
 * Sets directory mode 0700 and file mode 0600 when supported.
 *
 * @param {string} outputDir
 * @param {{ force?: boolean, allowedOutputDirs?: string[], seed?: Uint8Array }} [options]
 * @returns {{ implicit_account_id: string, public_key: string, filePath: string }}
 */
function writeNearCredentialsFile(outputDir, options = {}) {
  const { force = false, allowedOutputDirs, seed } = options;
  const dir = validateNearCredentialsOutputDir(outputDir, { allowedOutputDirs });
  const credentials = generateNearImplicitAccount(seed);
  const filePath = path.join(dir, `${credentials.implicit_account_id}.json`);

  if (!force && fs.existsSync(filePath)) {
    throw new Error(`Refusing to overwrite existing credentials file: ${filePath}`);
  }

  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  try {
    fs.chmodSync(dir, 0o700);
  } catch {
    // Best effort — some filesystems ignore mode on mkdir/chmod.
  }

  const payload = {
    implicit_account_id: credentials.implicit_account_id,
    public_key: credentials.public_key,
    private_key: credentials.private_key
  };

  fs.writeFileSync(filePath, `${JSON.stringify(payload)}\n`, { encoding: "utf8", mode: 0o600 });
  try {
    fs.chmodSync(filePath, 0o600);
  } catch {
    // Best effort
  }

  return {
    implicit_account_id: credentials.implicit_account_id,
    public_key: credentials.public_key,
    filePath
  };
}

module.exports = {
  generateNearImplicitAccount,
  validateNearCredentialsOutputDir,
  writeNearCredentialsFile,
  hasNearCredentialsDirSuffix
};
