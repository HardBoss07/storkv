# Use StorKV from TypeScript

The TypeScript binding loads the `storkv-ffi` library with koffi. It runs in Node.js and provides typed classes for the pool, scopes, transactions, and watches.

The binding lives in `bindings/ts/src/index.ts`. Read the [C ABI reference](../reference/ffi-c-abi.md) for the functions behind the class.

## Prerequisites

- Node.js 22 or later
- npm
- Rust 1.87 or later, to build the FFI library

Build the shared library:

```bash
cargo build -p storkv-ffi
```

The binding searches `target/debug` and `target/release` from the binding directory. Set `STORKV_FFI_LIB` to an absolute path when the library is in another location.

Install the dependencies and build the binding:

```bash
cd bindings/ts
npm install
npm run build
```

The build writes JavaScript and type declarations to `bindings/ts/dist`.

## Quickstart

```typescript
import { EventKind, Pool } from "storkv";

using pool = Pool.connect("file://./data/ts-app");
using scope = pool.scope("default");

scope.put("greeting", "hello");
console.log(scope.getString("greeting"));

scope.putWithTtl("session", "live", 900000);
console.log(Buffer.from(scope.getAndTouch("session", 900000)!).toString("utf8"));

scope.put("scan:1", "one");
scope.put("scan:2", "two");
console.log(scope.scanPrefix("scan:").length);

{
  using tx = pool.transaction();
  tx.put(scope, "tx:key", "value");
  tx.commit();
}

{
  using watch = scope.watchPrefix("events:");
  scope.put("events:1", "payload");
  const event = watch.next(5000);
  if (event && event.kind === EventKind.Put) {
    console.log(Buffer.from(event.key).toString("utf8"));
  }
}
```

The `using` keyword calls `[Symbol.dispose]` at the end of the block. Each class also has a `close()` method for code that does not use `using`.

## Install the package

The package has no published registry entry. Import it by path:

```typescript
import { Pool } from "./bindings/ts/dist/index.js";
```

A workspace package can also declare a local dependency:

```json
{
  "dependencies": {
    "storkv": "file:./bindings/ts"
  }
}
```

The package uses ECMAScript modules. Set `"type": "module"` in your project and use `module: "NodeNext"` in `tsconfig.json`.

## API types

| Type | Purpose |
| :--- | :--- |
| `Pool` | Pool handle |
| `Scope` | Scope handle |
| `Transaction` | Explicit OCC transaction |
| `Watch` | Stream of change events |
| `Entry` | Object with `key` and `value` byte arrays |
| `Event` | Object with `kind`, `scopeId`, `seq`, `key`, and `value` |
| `EventKind` | `Put`, `Delete`, or `DropScope` |
| `Status` | Status values from `Ok` to `Internal` |
| `StorkvError` | Error with a `status` property |

## Pool options

`Pool.connect` takes an optional options object:

| Option | Default | Effect |
| :--- | :--- | :--- |
| `healthCheck` | `true` | Probe idle clients before reuse |
| `maxConnections` | 10 | Maximum simultaneous connections |
| `minConnections` | 1 | Connections opened during connect |
| `acquireTimeoutMs` | 0 | Zero selects the SDK default |
| `idleTimeoutMs` | 0 | Zero disables idle expiry |

```typescript
const pool = Pool.connect("storkv://127.0.0.1:7878", {
  maxConnections: 16,
  acquireTimeoutMs: 3000,
});
```

## Scope methods

| Method | Result | Notes |
| :--- | :--- | :--- |
| `get` | `Uint8Array \| null` | Missing key returns null |
| `getString` | `string \| null` | UTF-8 decode |
| `put` | `void` | Accepts `string` or `Uint8Array` |
| `putWithTtl` | `void` | TTL in milliseconds |
| `getAndTouch` | `Uint8Array \| null` | Read and restart the expiry |
| `delete` | `void` | Remove one key |
| `exists` | `boolean` | Presence check |
| `scanPrefix` | `Entry[]` | One prefix |
| `scanRange` | `Entry[]` | Half-open range, nullable bounds |
| `watchPrefix` | `Watch` | Change stream |

Keys and values accept `string` or `Uint8Array`. A string encodes as UTF-8. An output value is always a `Uint8Array`.

The returned `Uint8Array` is a copy. The binding frees the native buffer before it returns. You do not call `storkv_bytes_free` or `storkv_entries_free`.

## Memory rules

koffi converts each input value to a temporary Node.js `Buffer`. The binding passes a pointer and a length to the C function. The buffer stays alive during the call.

Output buffers are decoded into JavaScript arrays. The binding calls the native free function after each decode.

The `seq` field is a `bigint`. Compare it with `===` against another `bigint`.

## Transactions

`pool.transaction()` starts an explicit transaction. Buffer writes with `put`, `putWithTtl`, and `delete`. Call `commit` or `rollback`.

```typescript
{
  using tx = pool.transaction();
  tx.put(scope, "counter", "1");
  tx.commit();
}
```

`commit` and `rollback` release the native handle. A later `close` does nothing.

The binding does not wrap `storkv_tx_run`. A TypeScript caller that needs automatic retries must write the retry loop.

## Watches

`watch.next(timeoutMs)` returns the next event or `null` on timeout. A timeout value of zero waits without a limit.

The event value is `null` for a delete or a scope drop. The key is an empty array for a scope drop.

Close the watch before the scope. A nested `using` block enforces that order.

## Errors

A non-zero status throws `StorkvError`. The `status` property holds the numeric status.

```typescript
import { Status, StorkvError } from "storkv";

try {
  scope.get("key");
} catch (error) {
  if (error instanceof StorkvError) {
    console.error(error.status, error.message);
  }
}
```

A closed scope throws `StorkvError` with status `InvalidArgument`. A closed pool throws with status `InvalidArgument` before the native call.

Read [Error codes](../reference/error-codes.md) for the server-side wire codes.

## Test example

The workspace test is `bindings/ts/tests/storkv.test.ts`. This smaller test uses the Node.js test runner:

```typescript
import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test, { after } from "node:test";

import { EventKind, Pool } from "../dist/index.js";

const directory = mkdtempSync(join(tmpdir(), "storkv-ts-test-"));
const uri = `file://${directory}`;

after(() => {
  rmSync(directory, { recursive: true, force: true });
});

test("CRUD and TTL", () => {
  using pool = Pool.connect(uri);
  using scope = pool.scope("default");

  scope.put("ts:greeting", "hello");
  assert.equal(scope.getString("ts:greeting"), "hello");
  assert.equal(scope.exists("ts:greeting"), true);
  scope.delete("ts:greeting");
  assert.equal(scope.exists("ts:greeting"), false);
  assert.equal(scope.get("ts:greeting"), null);

  scope.putWithTtl("ts:session", "live", 300);
  assert.deepEqual(
    Buffer.from(scope.getAndTouch("ts:session", 5000)!),
    Buffer.from("live"),
  );
});

test("scan and transaction", () => {
  using pool = Pool.connect(uri);
  using scope = pool.scope("default");

  scope.put("ts:scan:1", "one");
  scope.put("ts:scan:2", "two");
  assert.equal(scope.scanPrefix("ts:scan:").length, 2);
  assert.equal(scope.scanRange("ts:scan:1", "ts:scan:3").length, 2);

  {
    using tx = pool.transaction();
    tx.put(scope, "ts:tx", "value");
    assert.deepEqual(Buffer.from(tx.get(scope, "ts:tx")!), Buffer.from("value"));
    tx.commit();
  }
  assert.deepEqual(Buffer.from(scope.get("ts:tx")!), Buffer.from("value"));
});

test("watch delivery", () => {
  using pool = Pool.connect(uri);
  using scope = pool.scope("default");
  using watch = scope.watchPrefix("ts:events:");

  scope.put("ts:events:1", "payload");
  const event = watch.next(5000);
  assert.ok(event);
  assert.equal(event.kind, EventKind.Put);
  assert.deepEqual(Buffer.from(event.key), Buffer.from("ts:events:1"));
  assert.deepEqual(Buffer.from(event.value!), Buffer.from("payload"));
});
```

Compile and run the test suite:

```bash
cd bindings/ts
npm test
```

The test script runs the TypeScript compiler and then the Node.js test runner:

```bash
npm run build
node --test --experimental-strip-types tests/storkv.test.ts
```

Run against a server:

```bash
STORKV_TEST_URI=storkv://127.0.0.1:7878 npm test
```

The workspace test skips the networked section when `STORKV_TEST_URI` is unset.

## Related guides

- [C ABI reference](../reference/ffi-c-abi.md)
- [C guide](c.md)
- [Connection URIs](../getting-started/connection-uris.md)
- [Error codes](../reference/error-codes.md)
