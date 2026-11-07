/** Guest-visible path of the optional, per-run duplex byte device. */
export const HOST_DEVICE_PATH = '/dev/debugger-sh-host';

export interface HostDevice {
  /** Aborted on completion, stop, or callback failure; use it for cleanup. */
  readonly signal: AbortSignal;
  /** Arbitrary byte chunks. Returns an unsubscribe function. */
  onData(listener: (chunk: Uint8Array) => void): () => void;
  /** Snapshots bytes; resolves when published. Await before writing again. */
  write(chunk: Uint8Array): Promise<void>;
}

export type HostDeviceOpener = (device: HostDevice) => void;

// Each direction has one fixed slot: length 0 = empty, -1 = closed.
class Mailbox {
  readonly buffer = new SharedArrayBuffer(4 + 64 * 1024);
  readonly length = new Int32Array(this.buffer, 0, 1);
  readonly bytes = new Uint8Array(this.buffer, 4);

  publish(length: number) {
    Atomics.store(this.length, 0, length);
    Atomics.notify(this.length, 0);
  }
}

/** Internal transport; framing belongs to the application. */
export class HostDeviceSession {
  private readonly incoming = new Mailbox();
  private readonly outgoing = new Mailbox();
  readonly workerStart = {
    guest_to_host: this.incoming.buffer,
    host_to_guest: this.outgoing.buffer
  };
  private readonly abort = new AbortController();
  private readonly listeners = new Set<(chunk: Uint8Array) => void>();
  private writing = false;

  constructor(private readonly onFailure: (error: unknown) => void) {}

  open(opener: HostDeviceOpener) {
    opener({
      signal: this.abort.signal,
      onData: (listener) => {
        this.assertOpen();
        this.listeners.add(listener);
        return () => this.listeners.delete(listener);
      },
      write: (bytes) => this.write(bytes)
    });
  }

  private assertOpen() {
    if (this.abort.signal.aborted) throw new Error('Host device is closed');
  }

  private async write(bytes: Uint8Array): Promise<void> {
    this.assertOpen();
    if (this.writing) throw new Error('Await the previous host device write before writing again');
    this.writing = true;
    try {
      const snapshot = new Uint8Array(bytes);
      let offset = 0;
      while (offset < snapshot.length) {
        this.assertOpen();
        const length = Atomics.load(this.outgoing.length, 0);
        if (length !== 0) {
          await Atomics.waitAsync(this.outgoing.length, 0, length).value;
          continue;
        }
        const chunk = snapshot.subarray(offset, offset + this.outgoing.bytes.length);
        this.outgoing.bytes.set(chunk);
        this.outgoing.publish(chunk.length);
        offset += chunk.length;
      }
    } finally {
      this.writing = false;
    }
  }

  /** Called by the engine for each `host_wake` worker message. */
  drain() {
    if (this.abort.signal.aborted) return;
    const length = Atomics.load(this.incoming.length, 0);
    if (length <= 0) return;
    // Copy before releasing the slot; listeners may synchronously write back.
    const bytes = this.incoming.bytes.slice(0, length);
    this.incoming.publish(0);
    for (const listener of this.listeners) {
      if (this.abort.signal.aborted) break;
      void Promise.resolve(listener(bytes)).catch((error: unknown) => {
        if (!this.abort.signal.aborted) this.onFailure(error);
      });
    }
  }

  close() {
    if (this.abort.signal.aborted) return;
    this.incoming.publish(-1);
    this.outgoing.publish(-1);
    this.abort.abort();
    this.listeners.clear();
  }
}
