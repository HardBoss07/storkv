import koffi from "koffi";
import { existsSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(here, "..", "..", "..");

function libraryPath(): string {
  const configured = process.env.STORKV_FFI_LIB;
  if (configured) {
    return configured;
  }
  const names = ["storkv_ffi.dll", "libstorkv_ffi.so", "libstorkv_ffi.dylib"];
  for (const profile of ["debug", "release"]) {
    for (const name of names) {
      const candidate = join(repositoryRoot, "target", profile, name);
      if (existsSync(candidate)) {
        return candidate;
      }
    }
  }
  throw new StorkvError(
    Status.Internal,
    "could not find storkv_ffi; set STORKV_FFI_LIB",
  );
}

export enum Status {
  Ok = 0,
  NullPointer = 1,
  InvalidArgument = 2,
  Pool = 3,
  Scope = 4,
  Transaction = 5,
  Serialization = 6,
  Watch = 7,
  Timeout = 8,
  Internal = 9,
}

export enum EventKind {
  Put = 0,
  Delete = 1,
  DropScope = 2,
}

export class StorkvError extends Error {
  constructor(
    public readonly status: Status,
    message: string,
  ) {
    super(message);
    this.name = "StorkvError";
  }
}

export interface PoolOptions {
  healthCheck?: boolean;
  maxConnections?: number;
  minConnections?: number;
  acquireTimeoutMs?: number;
  idleTimeoutMs?: number;
}

export interface Entry {
  key: Uint8Array;
  value: Uint8Array;
}

export interface Event {
  kind: EventKind;
  scopeId: number;
  seq: bigint;
  key: Uint8Array;
  value: Uint8Array | null;
}

const StorkvString = koffi.struct("StorkvString", {
  data: "char*",
  len: "size_t",
});

const StorkvBytes = koffi.struct("StorkvBytes", {
  data: koffi.pointer("uint8"),
  len: "size_t",
});

const StorkvSlice = koffi.struct("StorkvSlice", {
  data: koffi.pointer("uint8"),
  len: "size_t",
});

const StorkvEntry = koffi.struct("StorkvEntry", {
  key: StorkvBytes,
  value: StorkvBytes,
});

const StorkvEntries = koffi.struct("StorkvEntries", {
  entries: koffi.pointer(StorkvEntry),
  len: "size_t",
});

const StorkvPoolConfig = koffi.struct("StorkvPoolConfig", {
  health_check: "bool",
  max_connections: "uint32",
  min_connections: "uint32",
  acquire_timeout_ms: "uint64",
  idle_timeout_ms: "uint64",
});

const StorkvWatchEvent = koffi.struct("StorkvWatchEvent", {
  kind: "int32",
  scope_id: "uint32",
  seq: "uint64",
  key: StorkvBytes,
  value: StorkvBytes,
});

const poolHandle = koffi.opaque("StorkvPoolHandle");
const scopeHandle = koffi.opaque("StorkvScopeHandle");
const txHandle = koffi.opaque("StorkvTxHandle");
const watchHandle = koffi.opaque("StorkvWatchHandle");

const lib = koffi.load(libraryPath());

const poolOpen = lib.func("storkv_pool_open", "int", [
  "str",
  koffi.pointer(StorkvPoolConfig),
  koffi.out(koffi.pointer(poolHandle, 2)),
]);
const poolClose = lib.func("storkv_pool_close", "void", [
  koffi.pointer(poolHandle),
]);
const poolIsClosed = lib.func("storkv_pool_is_closed", "int", [
  koffi.pointer(poolHandle),
  koffi.out(koffi.pointer("int32")),
]);

const scopeOpen = lib.func("storkv_scope_open", "int", [
  koffi.pointer(poolHandle),
  "str",
  koffi.out(koffi.pointer(scopeHandle, 2)),
]);
const scopeClose = lib.func("storkv_scope_close", "void", [
  koffi.pointer(scopeHandle),
]);

const get = lib.func("storkv_get", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  koffi.out(koffi.pointer(StorkvBytes)),
]);
const put = lib.func("storkv_put", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  StorkvSlice,
]);
const putWithTtl = lib.func("storkv_put_with_ttl", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  StorkvSlice,
  "uint64",
]);
const getAndTouch = lib.func("storkv_get_and_touch", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  "uint64",
  koffi.out(koffi.pointer(StorkvBytes)),
]);
const del = lib.func("storkv_delete", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
]);
const exists = lib.func("storkv_exists", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  koffi.out(koffi.pointer("int32")),
]);
const scanPrefix = lib.func("storkv_scan_prefix", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  koffi.out(koffi.pointer(StorkvEntries)),
]);
const scanRange = lib.func("storkv_scan_range", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  StorkvSlice,
  koffi.out(koffi.pointer(StorkvEntries)),
]);

const txBegin = lib.func("storkv_tx_begin", "int", [
  koffi.pointer(poolHandle),
  koffi.out(koffi.pointer(txHandle, 2)),
]);
const txGet = lib.func("storkv_tx_get", "int", [
  koffi.pointer(txHandle),
  koffi.pointer(scopeHandle),
  StorkvSlice,
  koffi.out(koffi.pointer(StorkvBytes)),
]);
const txPut = lib.func("storkv_tx_put", "int", [
  koffi.pointer(txHandle),
  koffi.pointer(scopeHandle),
  StorkvSlice,
  StorkvSlice,
]);
const txPutWithTtl = lib.func("storkv_tx_put_with_ttl", "int", [
  koffi.pointer(txHandle),
  koffi.pointer(scopeHandle),
  StorkvSlice,
  StorkvSlice,
  "uint64",
]);
const txDelete = lib.func("storkv_tx_delete", "int", [
  koffi.pointer(txHandle),
  koffi.pointer(scopeHandle),
  StorkvSlice,
]);
const txCommit = lib.func("storkv_tx_commit", "int", [
  koffi.pointer(txHandle),
]);
const txRollback = lib.func("storkv_tx_rollback", "void", [
  koffi.pointer(txHandle),
]);

const watchPrefix = lib.func("storkv_watch_prefix", "int", [
  koffi.pointer(scopeHandle),
  StorkvSlice,
  koffi.out(koffi.pointer(watchHandle, 2)),
]);
const watchNext = lib.func("storkv_watch_next", "int", [
  koffi.pointer(watchHandle),
  koffi.out(koffi.pointer(StorkvWatchEvent)),
  "uint64",
]);
const watchClose = lib.func("storkv_watch_close", "void", [
  koffi.pointer(watchHandle),
]);

const lastErrorMessage = lib.func(
  "storkv_last_error_message",
  StorkvString,
  [],
);
const stringFree = lib.func("storkv_string_free", "void", [StorkvString]);
const bytesFree = lib.func("storkv_bytes_free", "void", [StorkvBytes]);
const entriesFree = lib.func("storkv_entries_free", "void", [StorkvEntries]);

type Handle = bigint | number | null;

function errorText(fallback: string): string {
  const raw = lastErrorMessage() as { data: unknown; len: bigint };
  let text = fallback;
  try {
    const length = Number(raw.len);
    if (raw.data && length > 0) {
      text = koffi.decode(raw.data, "char", length) as string;
    }
  } finally {
    stringFree(raw as never);
  }
  return text;
}

function check(status: number): void {
  if (status === Status.Ok) {
    return;
  }
  throw new StorkvError(status as Status, errorText(`StorKV error ${status}`));
}

function bufferOf(value: string | Uint8Array): Buffer {
  return typeof value === "string" ? Buffer.from(value, "utf8") : Buffer.from(value);
}

function sliceOf(value: string | Uint8Array): { data: unknown; len: number } {
  const buffer = bufferOf(value);
  return {
    data: buffer.length === 0 ? null : koffi.as(buffer, "uint8 *"),
    len: buffer.length,
  };
}

const emptySlice = (): { data: unknown; len: number } => ({ data: null, len: 0 });

function decodeBytes(value: {
  data: unknown;
  len: bigint;
}): Uint8Array | null {
  const length = Number(value.len);
  if (!value.data || length === 0) {
    return null;
  }
  return Uint8Array.from(koffi.decode(value.data, "uint8", length) as number[]);
}

function readBytes(value: {
  data: unknown;
  len: bigint;
}): Uint8Array | null {
  const owned = decodeBytes(value);
  bytesFree(value as never);
  return owned;
}

function readEntries(raw: {
  entries: unknown;
  len: bigint;
}): Entry[] {
  const length = Number(raw.len);
  const owned: Entry[] = [];
  if (raw.entries && length > 0) {
    const decoded = koffi.decode(raw.entries, StorkvEntry, length) as Array<{
      key: { data: unknown; len: bigint };
      value: { data: unknown; len: bigint };
    }>;
    for (const entry of decoded) {
      owned.push({
        key: decodeBytes(entry.key) ?? new Uint8Array(),
        value: decodeBytes(entry.value) ?? new Uint8Array(),
      });
    }
  }
  entriesFree(raw as never);
  return owned;
}

function requireHandle(handle: Handle, what: string): Handle {
  if (handle === null || handle === undefined) {
    throw new StorkvError(Status.InvalidArgument, `${what} is closed`);
  }
  return handle;
}

export class Watch {
  private handle: Handle;

  constructor(handle: Handle) {
    this.handle = handle;
  }

  next(timeoutMs = 5000): Event | null {
    const handle = requireHandle(this.handle, "watch");
    const raw: Record<string, unknown> = {};
    const status = watchNext(handle, raw, timeoutMs);
    if (status === Status.Timeout) {
      return null;
    }
    check(status);
    const event = raw as {
      kind: number;
      scope_id: number;
      seq: bigint;
      key: { data: unknown; len: bigint };
      value: { data: unknown; len: bigint };
    };
    return {
      kind: event.kind as EventKind,
      scopeId: event.scope_id,
      seq: event.seq,
      key: readBytes(event.key) ?? new Uint8Array(),
      value: readBytes(event.value),
    };
  }

  close(): void {
    if (this.handle !== null) {
      watchClose(this.handle);
      this.handle = null;
    }
  }

  [Symbol.dispose](): void {
    this.close();
  }
}

export class Transaction {
  private handle: Handle;

  constructor(handle: Handle) {
    this.handle = handle;
  }

  get(scope: Scope, key: string | Uint8Array): Uint8Array | null {
    const handle = requireHandle(this.handle, "transaction");
    const raw: Record<string, unknown> = {};
    check(txGet(handle, scope.raw(), sliceOf(key), raw));
    return readBytes(raw as { data: unknown; len: bigint });
  }

  put(scope: Scope, key: string | Uint8Array, value: string | Uint8Array): void {
    check(txPut(requireHandle(this.handle, "transaction"), scope.raw(), sliceOf(key), sliceOf(value)));
  }

  putWithTtl(
    scope: Scope,
    key: string | Uint8Array,
    value: string | Uint8Array,
    ttlMs: number,
  ): void {
    check(
      txPutWithTtl(
        requireHandle(this.handle, "transaction"),
        scope.raw(),
        sliceOf(key),
        sliceOf(value),
        ttlMs,
      ),
    );
  }

  delete(scope: Scope, key: string | Uint8Array): void {
    check(txDelete(requireHandle(this.handle, "transaction"), scope.raw(), sliceOf(key)));
  }

  commit(): void {
    const handle = requireHandle(this.handle, "transaction");
    this.handle = null;
    check(txCommit(handle));
  }

  rollback(): void {
    if (this.handle !== null) {
      const handle = this.handle;
      this.handle = null;
      txRollback(handle);
    }
  }

  [Symbol.dispose](): void {
    this.rollback();
  }
}

export class Scope {
  private handle: Handle;

  constructor(handle: Handle) {
    this.handle = handle;
  }

  raw(): Handle {
    return requireHandle(this.handle, "scope");
  }

  get(key: string | Uint8Array): Uint8Array | null {
    const raw: Record<string, unknown> = {};
    check(get(this.raw(), sliceOf(key), raw));
    return readBytes(raw as { data: unknown; len: bigint });
  }

  getString(key: string | Uint8Array): string | null {
    const value = this.get(key);
    return value === null ? null : Buffer.from(value).toString("utf8");
  }

  put(key: string | Uint8Array, value: string | Uint8Array): void {
    check(put(this.raw(), sliceOf(key), sliceOf(value)));
  }

  putWithTtl(
    key: string | Uint8Array,
    value: string | Uint8Array,
    ttlMs: number,
  ): void {
    check(putWithTtl(this.raw(), sliceOf(key), sliceOf(value), ttlMs));
  }

  getAndTouch(key: string | Uint8Array, ttlMs: number): Uint8Array | null {
    const raw: Record<string, unknown> = {};
    check(getAndTouch(this.raw(), sliceOf(key), ttlMs, raw));
    return readBytes(raw as { data: unknown; len: bigint });
  }

  delete(key: string | Uint8Array): void {
    check(del(this.raw(), sliceOf(key)));
  }

  exists(key: string | Uint8Array): boolean {
    const flag: unknown[] = [null];
    check(exists(this.raw(), sliceOf(key), flag));
    return Number(flag[0]) !== 0;
  }

  scanPrefix(prefix: string | Uint8Array): Entry[] {
    const raw: Record<string, unknown> = {};
    check(scanPrefix(this.raw(), sliceOf(prefix), raw));
    return readEntries(raw as { entries: unknown; len: bigint });
  }

  scanRange(
    start: string | Uint8Array | null,
    end: string | Uint8Array | null,
  ): Entry[] {
    const raw: Record<string, unknown> = {};
    check(
      scanRange(
        this.raw(),
        start === null ? emptySlice() : sliceOf(start),
        end === null ? emptySlice() : sliceOf(end),
        raw,
      ),
    );
    return readEntries(raw as { entries: unknown; len: bigint });
  }

  watchPrefix(prefix: string | Uint8Array): Watch {
    const raw: unknown[] = [null];
    check(watchPrefix(this.raw(), sliceOf(prefix), raw));
    return new Watch(raw[0] as Handle);
  }

  close(): void {
    if (this.handle !== null) {
      scopeClose(this.handle);
      this.handle = null;
    }
  }

  [Symbol.dispose](): void {
    this.close();
  }
}

export class Pool {
  private handle: Handle;

  private constructor(handle: Handle) {
    this.handle = handle;
  }

  static connect(uri: string, options: PoolOptions = {}): Pool {
    const config = {
      health_check: options.healthCheck ?? true,
      max_connections: options.maxConnections ?? 10,
      min_connections: options.minConnections ?? 1,
      acquire_timeout_ms: options.acquireTimeoutMs ?? 0,
      idle_timeout_ms: options.idleTimeoutMs ?? 0,
    };
    const raw: unknown[] = [null];
    check(poolOpen(uri, config, raw));
    return new Pool(raw[0] as Handle);
  }

  scope(name: string): Scope {
    const raw: unknown[] = [null];
    check(scopeOpen(requireHandle(this.handle, "pool"), name, raw));
    return new Scope(raw[0] as Handle);
  }

  transaction(): Transaction {
    const raw: unknown[] = [null];
    check(txBegin(requireHandle(this.handle, "pool"), raw));
    return new Transaction(raw[0] as Handle);
  }

  isClosed(): boolean {
    const flag: unknown[] = [null];
    check(poolIsClosed(requireHandle(this.handle, "pool"), flag));
    return Number(flag[0]) !== 0;
  }

  close(): void {
    if (this.handle !== null) {
      poolClose(this.handle);
      this.handle = null;
    }
  }

  [Symbol.dispose](): void {
    this.close();
  }
}

export default Pool;
