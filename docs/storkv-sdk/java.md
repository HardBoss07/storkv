# Use StorKV from Java

The Java binding uses the Foreign Function and Memory API (FFM). It calls the `storkv-ffi` library without JNI. Each handle class implements `AutoCloseable`, so a try-with-resources block releases native memory.

The binding lives in `bindings/java/storkv/StorkvClient.java`. Read the [C ABI reference](../reference/ffi-c-abi.md) for the functions behind the class.

## Prerequisites

- Java 21 or later with the FFM API
- Rust 1.87 or later, to build the FFI library

Build the shared library:

```bash
cargo build -p storkv-ffi
```

The build writes one file to `target/debug`:

| Platform | File |
| :--- | :--- |
| Windows | `storkv_ffi.dll` |
| Linux | `libstorkv_ffi.so` |
| macOS | `libstorkv_ffi.dylib` |

The class searches `target/debug` and `target/release`. The search starts at the current directory and walks up two parent directories. Set `STORKV_FFI_LIB` to an absolute path when the library is in another location.

Java 21 treats the FFM API as a preview feature. Add `--enable-preview` to both `javac` and `java`. Java 22 and later do not need that flag.

Every run needs native access for the unnamed module:

```bash
--enable-native-access=ALL-UNNAMED
```

## Compile the binding

```bash
javac -d bindings/java/out bindings/java/storkv/StorkvClient.java
```

For Java 21 only, add `--release 21 --enable-preview`:

```bash
javac --release 21 --enable-preview -d bindings/java/out bindings/java/storkv/StorkvClient.java
```

## Quickstart

```java
import storkv.StorkvClient;

import java.nio.charset.StandardCharsets;
import java.util.List;

public final class Example {
    public static void main(String[] args) {
        try (StorkvClient pool = StorkvClient.connect("file://./data/java-app");
             StorkvClient.Scope scope = pool.scope("default")) {

            scope.put("greeting", "hello");
            System.out.println(scope.getString("greeting"));

            scope.putWithTtl("session", "live", 900000);
            System.out.println(new String(scope.getAndTouch("session", 900000),
                    StandardCharsets.UTF_8));

            scope.put("scan:1", "one");
            scope.put("scan:2", "two");
            List<StorkvClient.Entry> entries = scope.scanPrefix("scan:");
            System.out.println("entries: " + entries.size());

            try (StorkvClient.Transaction tx = pool.transaction()) {
                tx.put(scope, "tx:key", "value");
                tx.commit();
            }

            try (StorkvClient.Watch watch = scope.watchPrefix("events:")) {
                scope.put("events:1", "payload");
                StorkvClient.Event event = watch.next(5000);
                if (event != null) {
                    System.out.println(event.kind());
                }
            }
        }
    }
}
```

Run the program:

```bash
java --enable-native-access=ALL-UNNAMED -cp bindings/java/out Example
```

## API types

| Type | Purpose |
| :--- | :--- |
| `StorkvClient` | Pool handle and static connect method |
| `StorkvClient.Scope` | Scope handle |
| `StorkvClient.Transaction` | Explicit OCC transaction |
| `StorkvClient.Watch` | Stream of change events |
| `StorkvClient.Entry` | Record with `key()` and `value()` byte arrays |
| `StorkvClient.Event` | Record with `kind`, `scopeId`, `seq`, `key`, and `value` |
| `StorkvClient.EventKind` | `PUT`, `DELETE`, or `DROP_SCOPE` |
| `StorkvClient.StorkvException` | Runtime exception with `status()` |

`StorkvClient` also exposes status constants from `OK` to `INTERNAL`.

## Scope methods

| Method | Result | Notes |
| :--- | :--- | :--- |
| `get(String)` | `byte[]` or `null` | Missing key returns null |
| `get(byte[])` | `byte[]` or `null` | Binary key form |
| `getString(String)` | `String` or `null` | UTF-8 decode |
| `put(String, String)` | `void` | Text key and value |
| `put(byte[], byte[])` | `void` | Binary key and value |
| `putWithTtl` | `void` | TTL in milliseconds |
| `getAndTouch` | `byte[]` or `null` | Read and restart the expiry |
| `delete(String)` | `void` | Remove one key |
| `exists(String)` | `boolean` | Presence check |
| `scanPrefix(String)` | `List<Entry>` | One prefix |
| `scanRange(String, String)` | `List<Entry>` | Half-open range |
| `watchPrefix(String)` | `Watch` | Change stream |

A missing key returns `null`. The binding never returns an empty array for a missing key.

## Memory rules

The Java binding uses the FFM `Arena` API:

- Input strings and key bytes come from a temporary confined arena.
- The arena closes when the downcall returns.
- Output buffers are copied into Java arrays.
- The binding calls the native free function after each copy.

Java code therefore does not call `storkv_bytes_free` or `storkv_entries_free`. The garbage collector owns the Java arrays.

`StorkvClient` uses a shared arena for the native library lookup. The arena lives for the process lifetime. Call `close()` on the pool to release the native pool.

## Layout rules

The binding declares `StorkvPoolConfig` with explicit padding:

```java
MemoryLayout.structLayout(
        JAVA_BOOLEAN.withName("health_check"),
        MemoryLayout.paddingLayout(3),
        JAVA_INT.withName("max_connections"),
        JAVA_INT.withName("min_connections"),
        MemoryLayout.paddingLayout(4),
        JAVA_LONG.withName("acquire_timeout_ms"),
        JAVA_LONG.withName("idle_timeout_ms"));
```

The padding matches the C struct in the [C ABI reference](../reference/ffi-c-abi.md). Change the padding only when the C layout changes.

## Transactions

`pool.transaction()` starts an explicit transaction. Buffer writes with `put`, `putWithTtl`, and `delete`. Call `commit` or `rollback`.

A try-with-resources block rolls back when the block exits without a commit:

```java
try (StorkvClient.Transaction tx = pool.transaction()) {
    tx.put(scope, "counter", "1");
    tx.commit();
}
```

`commit` and `rollback` release the native handle. A later `close` does nothing.

The binding does not wrap `storkv_tx_run`. A Java caller that needs automatic retries must write the retry loop.

## Watches

`watch.next(timeoutMs)` returns the next event or `null` on timeout. A timeout value of zero waits without a limit.

The binding copies each event buffer into Java arrays. It then frees the native buffers. You receive Java arrays and need no free call.

Close the watch before the scope. A try-with-resources block handles the order when you declare resources in that order.

## Errors

A non-zero status raises `StorkvClient.StorkvException`. The `status()` method returns the integer status code.

```java
try {
    scope.get("key");
} catch (StorkvClient.StorkvException error) {
    System.err.println(error.status() + ": " + error.getMessage());
}
```

A closed scope raises `INVALID_ARGUMENT`. A closed pool raises `POOL`.

Read [Error codes](../reference/error-codes.md) for the server-side wire codes.

## Test example

The workspace test is `bindings/java/StorkvFfiTest.java`. This smaller test covers CRUD, a transaction, a watch, and one error:

```java
import storkv.StorkvClient;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

public final class StorkvTest {
    private static int failures = 0;

    public static void main(String[] args) throws Exception {
        Path directory = Files.createTempDirectory("storkv-java-test-");
        try {
            exercise("file://" + directory);
        } finally {
            try (var paths = Files.walk(directory)) {
                paths.sorted(java.util.Comparator.reverseOrder())
                        .forEach(path -> path.toFile().delete());
            }
        }
        if (failures != 0) {
            System.exit(1);
        }
        System.out.println("Java checks passed");
    }

    private static void exercise(String uri) {
        try (StorkvClient pool = StorkvClient.connect(uri);
             StorkvClient.Scope scope = pool.scope("default")) {
            check(!pool.isClosed(), "pool is open");

            scope.put("java:greeting", "hello");
            check("hello".equals(scope.getString("java:greeting")), "get string");
            check("hello".equals(text(scope.get(bytes("java:greeting")))), "get bytes");
            check(scope.exists("java:greeting"), "exists");
            scope.delete("java:greeting");
            check(!scope.exists("java:greeting"), "delete");
            check(scope.get("java:greeting") == null, "missing key");

            scope.putWithTtl("java:session", "live", 300);
            check("live".equals(text(scope.getAndTouch("java:session", 5000))), "touch");

            scope.put("java:scan:1", "one");
            scope.put("java:scan:2", "two");
            List<StorkvClient.Entry> entries = scope.scanPrefix("java:scan:");
            check(entries.size() == 2, "scan prefix");

            try (StorkvClient.Transaction tx = pool.transaction()) {
                tx.put(scope, "java:tx", "value");
                check("value".equals(text(tx.get(scope, "java:tx"))), "tx read");
                tx.commit();
            }
            check("value".equals(text(scope.get("java:tx"))), "tx commit");

            try (StorkvClient.Watch watch = scope.watchPrefix("java:events:")) {
                scope.put("java:events:1", "payload");
                StorkvClient.Event event = watch.next(5000);
                check(event != null, "watch event");
                if (event != null) {
                    check(event.kind() == StorkvClient.EventKind.PUT, "watch kind");
                    check("java:events:1".equals(text(event.key())), "watch key");
                }
            }

            StorkvClient.Scope closed = pool.scope("default");
            closed.close();
            try {
                closed.get("key");
                check(false, "closed scope rejects calls");
            } catch (StorkvClient.StorkvException error) {
                check(error.status() == StorkvClient.INVALID_ARGUMENT, "closed scope status");
            }
        }
    }

    private static byte[] bytes(String value) {
        return value.getBytes(StandardCharsets.UTF_8);
    }

    private static String text(byte[] value) {
        return value == null ? null : new String(value, StandardCharsets.UTF_8);
    }

    private static void check(boolean condition, String what) {
        if (!condition) {
            failures += 1;
            System.err.println("FAIL: " + what);
        }
    }
}
```

Compile and run:

```bash
javac -d bindings/java/out bindings/java/storkv/StorkvClient.java StorkvTest.java
java --enable-native-access=ALL-UNNAMED -cp bindings/java/out:. StorkvTest
```

On Windows, use `;` as the class path separator.

Run the workspace test:

```bash
javac -d bindings/java/out bindings/java/storkv/StorkvClient.java bindings/java/StorkvFfiTest.java
java --enable-native-access=ALL-UNNAMED -cp bindings/java/out StorkvFfiTest
```

Run the same test against a server:

```bash
STORKV_TEST_URI=storkv://127.0.0.1:7878 java --enable-native-access=ALL-UNNAMED -cp bindings/java/out StorkvFfiTest
```

The test skips the networked section when `STORKV_TEST_URI` is unset.

## Related guides

- [C ABI reference](../reference/ffi-c-abi.md)
- [C guide](c.md)
- [Connection URIs](../getting-started/connection-uris.md)
- [Error codes](../reference/error-codes.md)
