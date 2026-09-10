// After npm run build: npx -y bun tools/host-device/run.ts
import assert from 'node:assert/strict';

import { Engine, HOST_DEVICE_PATH, type HostDevice } from '../../dist/debugger-sh.js';

const size = 2 * 64 * 1024 + 37;
const expected = Uint8Array.from({ length: size }, (_, i) => i % 251);
const source = `#include <fcntl.h>
#include <unistd.h>
int main() {
  int fd = open("${HOST_DEVICE_PATH}", O_RDWR);
  if (fd < 0) return 1;
  unsigned char bytes[${size}];
  for (int offset = 0; offset < ${size};) {
    int count = ${size} - offset;
    if (count > 997) count = 997;
    int n = read(fd, bytes + offset, count);
    if (n <= 0) return 2;
    offset += n;
  }
  for (int i = 0; i < ${size}; ++i)
    if (bytes[i] != i % 251) return 3;
  for (int offset = 0; offset < ${size};) {
    int n = write(fd, bytes + offset, ${size} - offset);
    if (n <= 0) return 4;
    offset += n;
  }
  return 0; // No final acknowledgement: output must arrive before cleanup.
}`;
const engine = await Engine.create('c');
let handle!: HostDevice;
let cleanups = 0;
const unhandled: unknown[] = [];
const onUnhandled = (reason: unknown) => {
  unhandled.push(reason);
};
process.on('unhandledRejection', onUnhandled);
const timer = setTimeout(() => {
  engine.stop();
  throw new Error('Host device test timed out');
}, 90_000);

async function exchange(debug: boolean) {
  engine.debugger.enabled = debug;
  engine.fs = { 'main.cpp': source };
  const chunks: Uint8Array[] = [];
  let writing!: Promise<void>,
    overlap!: Promise<void>,
    bytesAtCleanup = 0;
  engine.hostDevice = (device) => {
    handle = device;
    device.onData((bytes) => {
      chunks.push(bytes);
    });
    const input = new Uint8Array(expected);
    writing = device.write(input);
    input.fill(0); // write() must snapshot before its first wait for space.
    overlap = assert.rejects(device.write(new Uint8Array([1])), /previous host device write/);
    return () => {
      cleanups++;
      bytesAtCleanup = chunks.reduce((total, chunk) => total + chunk.length, 0);
    };
  };
  const result = await engine.run();
  await Promise.all([writing, overlap]);
  assert.equal(result.type, 'completed', JSON.stringify(result));
  if (result.type === 'completed') assert.equal(result.exitCode, 0);
  assert.equal(bytesAtCleanup, size);
  assert.deepEqual(Buffer.concat(chunks), Buffer.from(expected));
  assert.equal(handle.signal.aborted, true);
  await assert.rejects(handle.write(new Uint8Array([1])), /closed/);
}

try {
  await exchange(false);
  assert.equal(cleanups, 1);

  // Stop an actual blocked guest read, then reuse the same Engine in Debug.
  engine.fs = {
    'main.cpp': `#include <fcntl.h>
#include <unistd.h>
int main() {
  int fd = open("${HOST_DEVICE_PATH}", O_RDWR);
  char byte = 'r';
  if (fd < 0 || write(fd, &byte, 1) != 1) return 1;
  return read(fd, &byte, 1) < 0;
}`
  };
  let ready!: () => void;
  const waiting = new Promise<void>((resolve) => {
    ready = resolve;
  });
  engine.hostDevice = (device) => {
    handle = device;
    device.onData(() => ready());
    return () => {
      cleanups++;
    };
  };
  const running = engine.run();
  await waiting;
  engine.stop();
  assert.equal((await running).type, 'stopped');
  assert.equal(cleanups, 2);
  await assert.rejects(handle.write(new Uint8Array([1])), /closed/);

  let sequence = 1;
  engine.debugger.on('event', (event: { event?: string }) => {
    if (event.event === 'initialized')
      queueMicrotask(() => {
        engine.debugger.send({ seq: sequence++, type: 'request', command: 'configurationDone' });
      });
  });
  engine.debugger.send({
    seq: sequence++,
    type: 'request',
    command: 'initialize',
    arguments: { adapterID: 'host-device' }
  });
  await exchange(true);
  assert.equal(cleanups, 3);
  engine.debugger.enabled = false;
  engine.fs = { 'main.cpp': 'int main() { return 0; }' };
  engine.hostDevice = () => async () => {
    cleanups++;
    throw new Error('Async cleanup is not supported');
  };
  const failed = await engine.run();
  assert.equal(failed.type, 'error');
  if (failed.type === 'error') {
    assert.equal(failed.error.type, 'TypeError');
    assert.match(failed.error.message, /cleanup must be synchronous/);
  }
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(cleanups, 4);
  assert.deepEqual(unhandled, []);
  console.log(
    'PASS: Run/Debug byte exchange, partial transfers, backpressure, final delivery, stop/rerun and cleanup'
  );
} finally {
  process.off('unhandledRejection', onUnhandled);
  clearTimeout(timer);
  engine.stop();
}
