import { LEGACY_TOOLCHAIN_DIGESTS, requireToolchainManifest } from './toolchain-manifest.js';

/** Engine-private immutable toolchain ownership. Seeds never cross the public API. */
export interface ToolchainSeed {
  compiler: Uint8Array;
  sysroot: Uint8Array;
  compilerHash: string;
  sysrootHash: string;
  exceptionsHash: string;
  receiptAbi?: string;
  module?: WebAssembly.Module;
}

export class VerifiedToolchain {
  #seed: ToolchainSeed;
  readonly hashCount = 3;
  readonly hashMs: number;
  private constructor(seed: ToolchainSeed, hashMs: number) {
    this.#seed = seed;
    this.hashMs = hashMs;
  }

  static async load(
    urls: readonly string[],
    cacheModule: boolean,
    signal?: AbortSignal,
    exceptionsHash: string = LEGACY_TOOLCHAIN_DIGESTS.exceptions,
    profile = 'legacy'
  ): Promise<VerifiedToolchain> {
    const manifest = requireToolchainManifest(profile);
    urls = manifest.urls ?? urls;
    if (urls.length !== 2) throw new Error('Missing C++ toolchain assets');
    const [compiler, sysroot] = await Promise.all(
      urls.map(async (url) => {
        const response = await fetch(url, { cache: 'force-cache', signal });
        if (!response.ok) throw new Error(`Cannot load C++ toolchain (${response.status})`);
        return new Uint8Array(await response.arrayBuffer());
      })
    );
    const owner = await VerifiedToolchain.ingestOwned(compiler, sysroot, false, exceptionsHash);
    if (
      owner.#seed.compilerHash !== manifest.digests.compiler ||
      owner.#seed.sysrootHash !== manifest.digests.sysroot ||
      owner.#seed.exceptionsHash !== manifest.digests.exceptions
    )
      throw new Error('C++ toolchain integrity mismatch');
    if (signal?.aborted) throw new DOMException('Toolchain loading aborted', 'AbortError');
    owner.#seed.receiptAbi = manifest.receiptAbi;
    if (cacheModule)
      owner.#seed.module = await WebAssembly.compile(
        owner.#seed.compiler as Uint8Array<ArrayBuffer>
      );
    return owner;
  }

  /** Captures even Buffer/subclass inputs before the first asynchronous boundary. */
  static async fromBytes(
    compilerInput: Uint8Array,
    sysrootInput: Uint8Array,
    cacheModule: boolean
  ): Promise<VerifiedToolchain> {
    const compiler = new Uint8Array(compilerInput);
    const sysroot = new Uint8Array(sysrootInput);
    return VerifiedToolchain.ingestOwned(
      compiler,
      sysroot,
      cacheModule,
      LEGACY_TOOLCHAIN_DIGESTS.exceptions
    );
  }

  private static async ingestOwned(
    compiler: Uint8Array<ArrayBuffer>,
    sysroot: Uint8Array<ArrayBuffer>,
    cacheModule: boolean,
    exceptionsHash: string
  ): Promise<VerifiedToolchain> {
    const started = performance.now();
    const [compilerHash, sysrootHash] = await Promise.all([digest(compiler), digest(sysroot)]);
    const module = cacheModule ? await WebAssembly.compile(compiler) : undefined;
    return new VerifiedToolchain(
      { compiler, sysroot, compilerHash, sysrootHash, exceptionsHash, module },
      performance.now() - started
    );
  }

  matchesExpectation(expected: {
    compilerSha256: string;
    sysrootSha256: string;
    exceptionsSha256: string;
  }): boolean {
    return (
      expected.compilerSha256 === this.#seed.compilerHash &&
      expected.sysrootSha256 === this.#seed.sysrootHash &&
      expected.exceptionsSha256 === this.#seed.exceptionsHash
    );
  }

  get identity(): string {
    return `${this.#seed.compilerHash}:${this.#seed.sysrootHash}:${this.#seed.exceptionsHash}`;
  }
  get identityBytes(): number {
    return this.#seed.compiler.byteLength + this.#seed.sysroot.byteLength;
  }
  get compilerBytes(): number {
    return this.#seed.compiler.byteLength;
  }

  /** postMessage clones the private seed; never transfer/detach the restart owner. */
  configure(worker: Worker, envelope: object): boolean {
    try {
      worker.postMessage({ ...envelope, toolchain: this.#seed });
      return !!this.#seed.module;
    } catch (error) {
      if (!(error instanceof DOMException) || error.name !== 'DataCloneError' || !this.#seed.module)
        throw error;
      worker.postMessage({ ...envelope, toolchain: { ...this.#seed, module: undefined } });
      return false;
    }
  }
}

async function digest(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  const hash = await crypto.subtle.digest('SHA-256', bytes);
  return Array.from(new Uint8Array(hash), (byte) => byte.toString(16).padStart(2, '0')).join('');
}
