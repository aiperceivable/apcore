export { matchPattern, calculateSpecificity } from './pattern.js';
export { normalizeToCanonicalId } from './normalize.js';
export { guardCallChain, DEFAULT_MAX_CALL_DEPTH, DEFAULT_MAX_MODULE_REPEAT } from './call-chain.js';
export { propagateError } from './error-propagation.js';

export function deepCopy<T>(obj: T): T {
  return JSON.parse(JSON.stringify(obj));
}

/**
 * Generate a random hex string of the given byte length.
 *
 * Uses the Web Crypto API (`crypto.getRandomValues`), available as
 * `globalThis.crypto` in all modern browsers and Node.js ≥ 19.
 */
export function randomHex(byteLength: number): string {
  const bytes = new Uint8Array(byteLength);
  const c = globalThis.crypto;
  if (c) {
    c.getRandomValues(bytes);
  } else {
    console.warn('[apcore:utils] crypto unavailable — falling back to Math.random for randomHex (not cryptographically secure)');
    for (let i = 0; i < bytes.length; i++) {
      bytes[i] = Math.floor(Math.random() * 256);
    }
  }
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}
