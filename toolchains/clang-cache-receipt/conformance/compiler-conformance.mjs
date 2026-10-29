import { createHash } from 'node:crypto';
import { mkdir, open, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { WASI } from 'node:wasi';

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const utf8 = new TextDecoder('utf-8', { fatal: true });

/** Test-only complete filesystem import trace. It is not cache admission authority. */
export async function invokeCompiler({ module, root, args, evidence, label,
  clockBase, entropySeed, mtime, receiptLimit = 1024 * 1024 }) {
  await mkdir(evidence, { recursive: true });
  const [stdin, stdout, stderr] = await Promise.all([
    open('/dev/null', 'r'), open(path.join(evidence, `${label}.stdout`), 'w'),
    open(path.join(evidence, `${label}.stderr`), 'w'),
  ]);
  const wasi = new WASI({ version: 'preview1', args: ['clang', ...args], env: {},
    preopens: { '/': root }, stdin: stdin.fd, stdout: stdout.fd, stderr: stderr.fd,
    returnOnExit: true });
  const imports = wasi.getImportObject();
  const events = [];
  const fdPaths = new Map([[0, '<stdin>'], [1, '<stdout>'], [2, '<stderr>']]);
  let instance, overflow = false, traceError = null, clockCalls = 0;
  let rng = entropySeed === undefined ? undefined : (entropySeed >>> 0) || 1;
  let receipt = null, receiptInvalid = null, completionCount = 0;
  const memory = () => new DataView(instance.exports.memory.buffer);
  const bytes = (ptr, len) => {
    ptr >>>= 0; len >>>= 0;
    const buffer = instance.exports.memory.buffer;
    if (len > buffer.byteLength || ptr > buffer.byteLength - len) throw new Error('invalid memory range');
    return new Uint8Array(buffer, ptr, len);
  };
  const string = (ptr, len) => utf8.decode(bytes(ptr, len));
  const guestPath = (fd, name) => {
    if (name.startsWith('/')) return path.posix.normalize(name);
    const parent = fdPaths.get(fd);
    return parent?.startsWith('/') ? path.posix.resolve(parent, name) : null;
  };
  const event = value => {
    if (events.length >= 100000) overflow = true;
    else events.push(value);
  };
  const descriptors = {
    path_open: { fd: 0, ptr: 2, len: 3 },
    path_filestat_get: { fd: 0, ptr: 2, len: 3 },
    path_filestat_set_times: { fd: 0, ptr: 2, len: 3 },
    path_readlink: { fd: 0, ptr: 1, len: 2 },
    path_create_directory: { fd: 0, ptr: 1, len: 2 },
    path_remove_directory: { fd: 0, ptr: 1, len: 2 },
    path_unlink_file: { fd: 0, ptr: 1, len: 2 },
    path_rename: { fd: 0, ptr: 1, len: 2, second: { fd: 3, ptr: 4, len: 5 } },
    path_link: { fd: 0, ptr: 2, len: 3, second: { fd: 4, ptr: 5, len: 6 } },
    path_symlink: { fd: 2, ptr: 3, len: 4 },
  };
  for (const [name, original] of Object.entries(imports.wasi_snapshot_preview1)) {
    if (!name.startsWith('path_') && !name.startsWith('fd_') &&
        !['clock_time_get', 'clock_res_get', 'random_get'].includes(name)) continue;
    imports.wasi_snapshot_preview1[name] = (...values) => {
      const item = { name, args: values.map(value => typeof value === 'bigint' ? value.toString() : value) };
      const descriptor = descriptors[name];
      try {
        if (descriptor) {
          item.spelling = string(values[descriptor.ptr], values[descriptor.len]);
          item.path = guestPath(values[descriptor.fd], item.spelling);
          if (descriptor.second) {
            const other = descriptor.second;
            item.secondSpelling = string(values[other.ptr], values[other.len]);
            item.secondPath = guestPath(values[other.fd], item.secondSpelling);
          }
        } else if (name.startsWith('path_')) {
          traceError ??= `unclassified path syscall: ${name}`;
        } else if (name.startsWith('fd_')) {
          item.path = fdPaths.get(values[0]) ?? null;
        }
      } catch (error) { traceError ??= String(error); }
      const result = original(...values);
      item.result = result;
      try {
        if (result === 0) {
          if (name === 'fd_prestat_dir_name') {
            item.path = string(values[1], values[2]);
            fdPaths.set(values[0], item.path);
          } else if (name === 'path_open') {
            item.openedFd = memory().getUint32(values[8] >>> 0, true);
            fdPaths.set(item.openedFd, item.path);
          } else if (name === 'fd_close') {
            fdPaths.delete(values[0]);
          } else if (name === 'fd_renumber') {
            fdPaths.set(values[1], fdPaths.get(values[0]));
            fdPaths.delete(values[0]);
          } else if (name === 'fd_read' || name === 'fd_pread') {
            item.bytesRead = memory().getUint32(values[name === 'fd_read' ? 3 : 4] >>> 0, true);
          } else if (name === 'fd_readdir') {
            item.bytesRead = memory().getUint32(values[4] >>> 0, true);
            item.directoryBytesSha256 = sha256(bytes(values[1], item.bytesRead));
          } else if (name === 'path_readlink') {
            const count = memory().getUint32(values[5] >>> 0, true);
            item.target = string(values[3], count);
          }
          if (name === 'clock_time_get' && clockBase !== undefined) {
            memory().setBigUint64(values[2] >>> 0, BigInt(clockBase) + BigInt(clockCalls++) * 1000000n, true);
          }
          if (name === 'random_get' && rng !== undefined) {
            for (const index of bytes(values[0], values[1]).keys()) {
              rng ^= rng << 13; rng ^= rng >>> 17; rng ^= rng << 5;
              bytes(values[0], values[1])[index] = rng & 255;
            }
          }
          if (mtime !== undefined && (name === 'path_filestat_get' || name === 'fd_filestat_get')) {
            const offset = values[name === 'path_filestat_get' ? 4 : 1] >>> 0;
            memory().setBigUint64(offset + 40, BigInt(mtime), true);
            memory().setBigUint64(offset + 48, BigInt(mtime), true);
            memory().setBigUint64(offset + 56, BigInt(mtime), true);
          }
        }
      } catch (error) { traceError ??= String(error); }
      event(item);
      return result;
    };
  }
  imports.webide_compiler = { cache_receipt_v1(ptr, length) {
    completionCount++;
    try {
      if (completionCount !== 1) throw new Error('duplicate completion');
      if (!Number.isInteger(length) || length <= 0 || length > Math.min(1024 * 1024, receiptLimit)) throw new Error('oversized or empty receipt');
      receipt = JSON.parse(utf8.decode(Uint8Array.from(bytes(ptr, length))));
      return 0;
    } catch (error) { receipt = null; receiptInvalid = String(error); return 1; }
  } };
  const started = performance.now();
  let exit, thrown;
  try {
    instance = await WebAssembly.instantiate(module, imports);
    exit = wasi.start(instance);
  } catch (error) { thrown = String(error); }
  finally { await Promise.all([stdin.close(), stdout.close(), stderr.close()]); }
  const result = { label, args, exit, thrown, elapsedMs: performance.now() - started,
    timingUse: 'exploratory Node WASI; concurrent with development; not a browser performance gate',
    clockBase: String(clockBase), entropySeed, mtime: String(mtime),
    traceComplete: !overflow && traceError === null, overflow, traceError,
    completionCount, receipt, receiptInvalid, events,
    stdout: await readFile(path.join(evidence, `${label}.stdout`), 'utf8'),
    stderr: await readFile(path.join(evidence, `${label}.stderr`), 'utf8') };
  const outputIndex = args.indexOf('-o');
  if (outputIndex >= 0) {
    try {
      const output = await readFile(path.join(root, args[outputIndex + 1]));
      result.output = { bytes: output.length, sha256: sha256(output) };
      await writeFile(path.join(evidence, `${label}.output`), output);
    } catch { result.output = null; }
  }
  await writeFile(path.join(evidence, `${label}.json`), JSON.stringify(result, null, 2) + '\n');
  return result;
}
