import receiptProfile from './receipt-toolchain-profile.json' with { type: 'json' };

/** Build-fixed legacy identities, verified against the shipping @yowasp/clang artifact.
 * Receipt profiles must be added only with a validated immutable release tuple.
 */
export const LEGACY_TOOLCHAIN_DIGESTS = Object.freeze({
  compiler: '24fbed474c7b5b4968fd73fc4827440b93fb351c1b6264516130300eff3e7bf5',
  sysroot: '8745c6af23fd841fa507d0c73f6b6b8e8da1b4ec216a6339739a28b38ff7227b',
  exceptions: '20986166552953de0e3ca5c1e9f341e7aec8b9ba54dcb244d0d2b2f9ca843ab4'
});

export interface ToolchainManifest {
  readonly profile: 'legacy' | 'cache-receipt-v1';
  readonly digests: Readonly<Record<keyof typeof LEGACY_TOOLCHAIN_DIGESTS, string>>;
  readonly urls?: readonly [string, string];
  readonly receiptAbi?: string;
}

interface ReceiptBuildProfile {
  profile: 'cache-receipt-v1';
  compiler: { sha256: string };
  sysroot: { sha256: string };
  exceptionsSha256: string;
  engineAbi: string;
}

/** Deliberately absent until exact control/observer artifacts pass their gates.
 * Adding an immutable manifest is a reviewed release change, never a runtime
 * option, supplied digest, filesystem file or environment variable. */
function receiptManifest(): ToolchainManifest | undefined {
  const profile = receiptProfile as ReceiptBuildProfile | null;
  if (!profile) return undefined;
  return {
    profile: profile.profile,
    digests: {
      compiler: profile.compiler.sha256,
      sysroot: profile.sysroot.sha256,
      exceptions: profile.exceptionsSha256
    },
    receiptAbi: profile.engineAbi,
    // Replaced after the engine library's asset transform. The final package
    // retains literal relative URLs for its consumer's ordinary asset bundler.
    urls: [
      new URL(/* @vite-ignore */ '__WEBIDE_RECEIPT_COMPILER_ASSET__', import.meta.url).href,
      new URL(/* @vite-ignore */ '__WEBIDE_RECEIPT_SYSROOT_ASSET__', import.meta.url).href
    ]
  };
}

export function requireToolchainManifest(profile = 'legacy'): ToolchainManifest {
  if (profile === 'legacy') return { profile, digests: LEGACY_TOOLCHAIN_DIGESTS };
  const manifest = profile === 'cache-receipt-v1' ? receiptManifest() : undefined;
  if (!manifest) throw new Error('The cache-receipt-v1 toolchain is not available in this build');
  return manifest;
}
