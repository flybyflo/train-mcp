import structuredClone from '@ungap/structured-clone';
import { polyfillWebCrypto } from 'expo-standard-web-crypto';

if (typeof globalThis.crypto === 'undefined') {
  polyfillWebCrypto();
}

if (typeof globalThis.structuredClone === 'undefined') {
  (globalThis as { structuredClone: typeof structuredClone }).structuredClone = structuredClone;
}
