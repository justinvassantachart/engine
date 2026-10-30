import { build } from 'esbuild';
import assert from 'node:assert/strict';
import test from 'node:test';

import type { ToolchainSeed } from '../../src/ts/verified-toolchain.ts';

type SeedMessage = { toolchain: ToolchainSeed };

const bundle = await build({
  entryPoints: ['src/ts/verified-toolchain.ts'],
  bundle: true,
  write: false,
  platform: 'node',
  format: 'esm'
});
const { VerifiedToolchain } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`
);

const minimal = () => new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]);

test('verified toolchain owns Buffer/subclass inputs before hashing and keeps seed after transport', async () => {
  class HostileBytes extends Uint8Array {
    slice() {
      return this.subarray();
    }
  }
  const wasm = Buffer.from(minimal());
  const sysroot = new HostileBytes([1, 2]);
  const pending = VerifiedToolchain.fromBytes(wasm, sysroot, true);
  wasm.fill(255);
  sysroot.fill(255);
  const owner = await pending;
  const messages: SeedMessage[] = [];
  const worker = {
    postMessage(value: SeedMessage) {
      messages.push(structuredClone(value));
    }
  } as Worker;
  owner.configure(worker, { type: 'configure_cpp' });
  assert.deepEqual([...messages[0].toolchain.compiler], [...minimal()]);
  assert.deepEqual([...messages[0].toolchain.sysroot], [1, 2]);
  messages[0].toolchain.compiler.fill(255);
  owner.configure(worker, { type: 'configure_cpp' });
  assert.deepEqual([...messages[1].toolchain.compiler], [...minimal()]);
  assert.equal(owner.identityBytes, 10);
});

test('module clone fallback transports exact owned bytes without requiring module clone support', async () => {
  const owner = await VerifiedToolchain.fromBytes(minimal(), new Uint8Array([1]), true);
  const messages: SeedMessage[] = [];
  const worker = {
    postMessage(value: SeedMessage) {
      if (value.toolchain.module) throw new DOMException('unsupported', 'DataCloneError');
      messages.push(value);
    }
  } as Worker;
  owner.configure(worker, {});
  assert.equal(messages.length, 1);
  assert.equal(messages[0].toolchain.module, undefined);
  assert.deepEqual([...messages[0].toolchain.compiler], [...minimal()]);
});

test('companion identity is an expectation and cannot authorize a different compiler', async () => {
  const owner = await VerifiedToolchain.fromBytes(minimal(), new Uint8Array([1]), false);
  const captured: SeedMessage[] = [];
  owner.configure(
    {
      postMessage(value: SeedMessage) {
        captured.push(value);
      }
    } as Worker,
    {}
  );
  const seed = captured[0].toolchain;
  assert.equal(
    seed.receiptAbi,
    undefined,
    'Arbitrary owned test bytes cannot acquire receipt capability'
  );
  const expectation = {
    compilerSha256: seed.compilerHash,
    sysrootSha256: seed.sysrootHash,
    exceptionsSha256: seed.exceptionsHash
  };
  assert.equal(owner.matchesExpectation(expectation), true);
  assert.equal(owner.matchesExpectation({ ...expectation, compilerSha256: '0'.repeat(64) }), false);
});

test('unavailable receipt profile fails before fetching or trusting caller bytes', async () => {
  const original = globalThis.fetch;
  let calls = 0;
  globalThis.fetch = async () => {
    calls++;
    throw new Error('unexpected fetch');
  };
  try {
    await assert.rejects(
      VerifiedToolchain.load(
        ['https://example.test/compiler', 'https://example.test/sysroot'],
        false,
        undefined,
        'a'.repeat(64),
        'cache-receipt-v1'
      ),
      /not available/
    );
    assert.equal(calls, 0);
  } finally {
    globalThis.fetch = original;
  }
});
