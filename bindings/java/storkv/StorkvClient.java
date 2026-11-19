package storkv;

import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemoryLayout;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.lang.invoke.MethodType;
import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.util.ArrayList;
import java.util.List;
import java.util.Locale;
import java.util.Optional;

import static java.lang.foreign.ValueLayout.ADDRESS;
import static java.lang.foreign.ValueLayout.JAVA_BOOLEAN;
import static java.lang.foreign.ValueLayout.JAVA_BYTE;
import static java.lang.foreign.ValueLayout.JAVA_INT;
import static java.lang.foreign.ValueLayout.JAVA_LONG;

/**
 * Java 21+ Foreign Function and Memory bindings for the StorKV C ABI.
 *
 * <p>Every handle owns native resources and must be closed. Pools, scopes,
 * transactions, and watches implement {@link AutoCloseable} and may be used in
 * try-with-resources blocks.</p>
 */
@SuppressWarnings("restricted")
public final class StorkvClient implements AutoCloseable {
    public static final int OK = 0;
    public static final int NULL_POINTER = 1;
    public static final int INVALID_ARGUMENT = 2;
    public static final int POOL = 3;
    public static final int SCOPE = 4;
    public static final int TRANSACTION = 5;
    public static final int SERIALIZATION = 6;
    public static final int WATCH = 7;
    public static final int TIMEOUT = 8;
    public static final int INTERNAL = 9;

    private static final MemoryLayout STRING = MemoryLayout.structLayout(
            ADDRESS.withName("data"),
            JAVA_LONG.withName("len"));

    private static final MemoryLayout BYTES = MemoryLayout.structLayout(
            ADDRESS.withName("data"),
            JAVA_LONG.withName("len"));

    private static final MemoryLayout SLICE = MemoryLayout.structLayout(
            ADDRESS.withName("data"),
            JAVA_LONG.withName("len"));

    private static final MemoryLayout ENTRY = MemoryLayout.structLayout(
            BYTES.withName("key"),
            BYTES.withName("value"));

    private static final MemoryLayout ENTRIES = MemoryLayout.structLayout(
            ADDRESS.withName("entries"),
            JAVA_LONG.withName("len"));

    private static final MemoryLayout POOL_CONFIG = MemoryLayout.structLayout(
            JAVA_BOOLEAN.withName("health_check"),
            MemoryLayout.paddingLayout(3),
            JAVA_INT.withName("max_connections"),
            JAVA_INT.withName("min_connections"),
            MemoryLayout.paddingLayout(4),
            JAVA_LONG.withName("acquire_timeout_ms"),
            JAVA_LONG.withName("idle_timeout_ms"));

    private static final MemoryLayout WATCH_EVENT = MemoryLayout.structLayout(
            JAVA_INT.withName("kind"),
            JAVA_INT.withName("scope_id"),
            JAVA_LONG.withName("seq"),
            BYTES.withName("key"),
            BYTES.withName("value"));

    private static final Linker LINKER = Linker.nativeLinker();
    private static final Arena LIBRARY_ARENA = Arena.ofShared();
    private static final SymbolLookup LIBRARY = loadLibrary();
    private static final MethodHandle POOL_OPEN = downcall("storkv_pool_open",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS));
    private static final MethodHandle POOL_CLOSE = downcall("storkv_pool_close",
            FunctionDescriptor.ofVoid(ADDRESS));
    private static final MethodHandle POOL_IS_CLOSED = downcall("storkv_pool_is_closed",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
    private static final MethodHandle SCOPE_OPEN = downcall("storkv_scope_open",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, ADDRESS));
    private static final MethodHandle SCOPE_CLOSE = downcall("storkv_scope_close",
            FunctionDescriptor.ofVoid(ADDRESS));
    private static final MethodHandle GET = downcall("storkv_get",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, ADDRESS));
    private static final MethodHandle PUT = downcall("storkv_put",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, SLICE));
    private static final MethodHandle PUT_WITH_TTL = downcall("storkv_put_with_ttl",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, SLICE, JAVA_LONG));
    private static final MethodHandle GET_AND_TOUCH = downcall("storkv_get_and_touch",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, JAVA_LONG, ADDRESS));
    private static final MethodHandle DELETE = downcall("storkv_delete",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE));
    private static final MethodHandle EXISTS = downcall("storkv_exists",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, ADDRESS));
    private static final MethodHandle SCAN_PREFIX = downcall("storkv_scan_prefix",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, ADDRESS));
    private static final MethodHandle SCAN_RANGE = downcall("storkv_scan_range",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, SLICE, ADDRESS));
    private static final MethodHandle TX_BEGIN = downcall("storkv_tx_begin",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS));
    private static final MethodHandle TX_GET = downcall("storkv_tx_get",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SLICE, ADDRESS));
    private static final MethodHandle TX_PUT = downcall("storkv_tx_put",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SLICE, SLICE));
    private static final MethodHandle TX_PUT_WITH_TTL = downcall("storkv_tx_put_with_ttl",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SLICE, SLICE, JAVA_LONG));
    private static final MethodHandle TX_DELETE = downcall("storkv_tx_delete",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, SLICE));
    private static final MethodHandle TX_COMMIT = downcall("storkv_tx_commit",
            FunctionDescriptor.of(JAVA_INT, ADDRESS));
    private static final MethodHandle TX_ROLLBACK = downcall("storkv_tx_rollback",
            FunctionDescriptor.ofVoid(ADDRESS));
    private static final MethodHandle WATCH_PREFIX = downcall("storkv_watch_prefix",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, SLICE, ADDRESS));
    private static final MethodHandle WATCH_NEXT = downcall("storkv_watch_next",
            FunctionDescriptor.of(JAVA_INT, ADDRESS, ADDRESS, JAVA_LONG));
    private static final MethodHandle WATCH_CLOSE = downcall("storkv_watch_close",
            FunctionDescriptor.ofVoid(ADDRESS));
    private static final MethodHandle LAST_ERROR = downcall("storkv_last_error_message",
            FunctionDescriptor.of(STRING));
    private static final MethodHandle STRING_FREE = downcall("storkv_string_free",
            FunctionDescriptor.ofVoid(STRING));
    private static final MethodHandle BYTES_FREE = downcall("storkv_bytes_free",
            FunctionDescriptor.ofVoid(BYTES));
    private static final MethodHandle ENTRIES_FREE = downcall("storkv_entries_free",
            FunctionDescriptor.ofVoid(ENTRIES));

    private long handle;
    private boolean closed;

    private StorkvClient(long handle) {
        this.handle = handle;
    }

    /**
     * Open a pooled connection to an embedded or networked StorKV backend.
     *
     * @param uri backend URI, such as {@code file:///tmp/data} or
     *            {@code storkv://127.0.0.1:7878}
     * @return an open pool
     */
    public static StorkvClient connect(String uri) {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment uriSegment = cString(arena, uri);
            MemorySegment out = arena.allocate(ADDRESS);
            int status = (int) POOL_OPEN.invokeExact(uriSegment, MemorySegment.NULL, out);
            check(status);
            return new StorkvClient(out.get(ADDRESS, 0).address());
        } catch (Throwable error) {
            throw propagate(error);
        }
    }

    /**
     * Open a lightweight handle for one scope.
     *
     * @param name scope name
     * @return an open scope handle
     */
    public Scope scope(String name) {
        requireOpen();
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment nameSegment = cString(arena, name);
            MemorySegment out = arena.allocate(ADDRESS);
            int status = (int) SCOPE_OPEN.invokeExact(pointer(handle), nameSegment, out);
            check(status);
            return new Scope(out.get(ADDRESS, 0).address());
        } catch (Throwable error) {
            throw propagate(error);
        }
    }

    /**
     * Begin an explicit transaction.
     *
     * @return an open transaction handle
     */
    public Transaction transaction() {
        requireOpen();
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment out = arena.allocate(ADDRESS);
            int status = (int) TX_BEGIN.invokeExact(pointer(handle), out);
            check(status);
            return new Transaction(out.get(ADDRESS, 0).address());
        } catch (Throwable error) {
            throw propagate(error);
        }
    }

    /**
     * Report whether the native pool has been closed.
     *
     * @return true when the pool is closed
     */
    public boolean isClosed() {
        try (Arena arena = Arena.ofConfined()) {
            MemorySegment out = arena.allocate(JAVA_INT);
            int status = (int) POOL_IS_CLOSED.invokeExact(pointer(handle), out);
            check(status);
            return out.get(JAVA_INT, 0) != 0;
        } catch (Throwable error) {
            throw propagate(error);
        }
    }

    @Override
    public void close() {
        if (!closed) {
            closed = true;
            try {
                POOL_CLOSE.invokeExact(pointer(handle));
            } catch (Throwable error) {
                throw propagate(error);
            }
            handle = 0;
        }
    }

    private void requireOpen() {
        if (closed || handle == 0) {
            throw new StorkvException(INVALID_ARGUMENT, "pool is closed");
        }
    }

    /**
     * Result kind of one watch event.
     */
    public enum EventKind {
        /** A key received a new value. */
        PUT,
        /** A key was deleted. */
        DELETE,
        /** A scope was dropped. */
        DROP_SCOPE,
    }

    /**
     * One key/value pair returned by a scan.
     *
     * @param key key bytes
     * @param value value bytes
     */
    public record Entry(byte[] key, byte[] value) {
    }

    /**
     * One decoded watch event.
     *
     * @param kind change kind
     * @param scopeId scope identifier
     * @param seq commit sequence number
     * @param key key bytes
     * @param value new value bytes, or null for deletes
     */
    public record Event(EventKind kind, int scopeId, long seq, byte[] key, byte[] value) {
    }

    /**
     * One scope handle.
     */
    public static final class Scope implements AutoCloseable {
        private long handle;
        private boolean closed;

        private Scope(long handle) {
            this.handle = handle;
        }

        public byte[] get(String key) {
            return get(bytes(key));
        }

        public byte[] get(byte[] key) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, key);
                MemorySegment out = arena.allocate(BYTES);
                int status = (int) GET.invokeExact(pointer(handle), keySlice, out);
                check(status);
                return takeBytes(out);
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public String getString(String key) {
            byte[] value = get(key);
            return value == null ? null : new String(value, StandardCharsets.UTF_8);
        }

        public void put(String key, String value) {
            put(bytes(key), bytes(value));
        }

        public void put(byte[] key, byte[] value) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, key);
                MemorySegment valueSlice = slice(arena, value);
                check((int) PUT.invokeExact(pointer(handle), keySlice, valueSlice));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void putWithTtl(String key, String value, long ttlMs) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, bytes(key));
                MemorySegment valueSlice = slice(arena, bytes(value));
                check((int) PUT_WITH_TTL.invokeExact(
                        pointer(handle), keySlice, valueSlice, ttlMs));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public byte[] getAndTouch(String key, long ttlMs) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, bytes(key));
                MemorySegment out = arena.allocate(BYTES);
                check((int) GET_AND_TOUCH.invokeExact(
                        pointer(handle), keySlice, ttlMs, out));
                return takeBytes(out);
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void delete(String key) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, bytes(key));
                check((int) DELETE.invokeExact(pointer(handle), keySlice));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public boolean exists(String key) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, bytes(key));
                MemorySegment out = arena.allocate(JAVA_INT);
                check((int) EXISTS.invokeExact(pointer(handle), keySlice, out));
                return out.get(JAVA_INT, 0) != 0;
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public List<Entry> scanPrefix(String prefix) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment prefixSlice = slice(arena, bytes(prefix));
                MemorySegment out = arena.allocate(ENTRIES);
                check((int) SCAN_PREFIX.invokeExact(pointer(handle), prefixSlice, out));
                return takeEntries(out);
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public List<Entry> scanRange(String start, String end) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment startSlice = start == null ? emptySlice(arena) : slice(arena, bytes(start));
                MemorySegment endSlice = end == null ? emptySlice(arena) : slice(arena, bytes(end));
                MemorySegment out = arena.allocate(ENTRIES);
                check((int) SCAN_RANGE.invokeExact(pointer(handle), startSlice, endSlice, out));
                return takeEntries(out);
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public Watch watchPrefix(String prefix) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment prefixSlice = slice(arena, bytes(prefix));
                MemorySegment out = arena.allocate(ADDRESS);
                check((int) WATCH_PREFIX.invokeExact(pointer(handle), prefixSlice, out));
                return new Watch(out.get(ADDRESS, 0).address());
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        long address() {
            requireOpen();
            return handle;
        }

        @Override
        public void close() {
            if (!closed) {
                closed = true;
                try {
                    SCOPE_CLOSE.invokeExact(pointer(handle));
                } catch (Throwable error) {
                    throw propagate(error);
                }
                handle = 0;
            }
        }

        private void requireOpen() {
            if (closed || handle == 0) {
                throw new StorkvException(INVALID_ARGUMENT, "scope is closed");
            }
        }
    }

    /**
     * One explicit optimistic transaction.
     */
    public static final class Transaction implements AutoCloseable {
        private long handle;
        private boolean closed;

        private Transaction(long handle) {
            this.handle = handle;
        }

        public byte[] get(Scope scope, String key) {
            return get(scope, bytes(key));
        }

        public byte[] get(Scope scope, byte[] key) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, key);
                MemorySegment out = arena.allocate(BYTES);
                check((int) TX_GET.invokeExact(
                        pointer(handle), pointer(scope.address()), keySlice, out));
                return takeBytes(out);
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void put(Scope scope, String key, String value) {
            put(scope, bytes(key), bytes(value));
        }

        public void put(Scope scope, byte[] key, byte[] value) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, key);
                MemorySegment valueSlice = slice(arena, value);
                check((int) TX_PUT.invokeExact(
                        pointer(handle), pointer(scope.address()), keySlice, valueSlice));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void putWithTtl(Scope scope, String key, String value, long ttlMs) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, bytes(key));
                MemorySegment valueSlice = slice(arena, bytes(value));
                check((int) TX_PUT_WITH_TTL.invokeExact(
                        pointer(handle), pointer(scope.address()), keySlice, valueSlice, ttlMs));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void delete(Scope scope, String key) {
            requireOpen();
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment keySlice = slice(arena, bytes(key));
                check((int) TX_DELETE.invokeExact(
                        pointer(handle), pointer(scope.address()), keySlice));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void commit() {
            long current = requireOpen();
            handle = 0;
            closed = true;
            try {
                check((int) TX_COMMIT.invokeExact(pointer(current)));
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        public void rollback() {
            if (!closed && handle != 0) {
                long current = handle;
                handle = 0;
                closed = true;
                try {
                    TX_ROLLBACK.invokeExact(pointer(current));
                } catch (Throwable error) {
                    throw propagate(error);
                }
            }
        }

        @Override
        public void close() {
            rollback();
        }

        private long requireOpen() {
            if (closed || handle == 0) {
                throw new StorkvException(TRANSACTION, "transaction is closed");
            }
            return handle;
        }
    }

    /**
     * One live watch stream.
     */
    public static final class Watch implements AutoCloseable {
        private long handle;
        private boolean closed;

        private Watch(long handle) {
            this.handle = handle;
        }

        /**
         * Receive the next event.
         *
         * @param timeoutMs timeout in milliseconds, or zero to wait forever
         * @return the next event, or null on timeout
         */
        public Event next(long timeoutMs) {
            if (closed || handle == 0) {
                return null;
            }
            try (Arena arena = Arena.ofConfined()) {
                MemorySegment out = arena.allocate(WATCH_EVENT);
                int status = (int) WATCH_NEXT.invokeExact(pointer(handle), out, timeoutMs);
                if (status == TIMEOUT) {
                    return null;
                }
                check(status);
                int kind = out.get(JAVA_INT, WATCH_EVENT.byteOffset(
                        MemoryLayout.PathElement.groupElement("kind")));
                int scopeId = out.get(JAVA_INT, WATCH_EVENT.byteOffset(
                        MemoryLayout.PathElement.groupElement("scope_id")));
                long seq = out.get(JAVA_LONG, WATCH_EVENT.byteOffset(
                        MemoryLayout.PathElement.groupElement("seq")));
                MemorySegment keySegment = out.asSlice(WATCH_EVENT.byteOffset(
                        MemoryLayout.PathElement.groupElement("key")), BYTES.byteSize());
                MemorySegment valueSegment = out.asSlice(WATCH_EVENT.byteOffset(
                        MemoryLayout.PathElement.groupElement("value")), BYTES.byteSize());
                byte[] key = takeBytes(keySegment);
                byte[] value = takeBytes(valueSegment);
                EventKind mapped = switch (kind) {
                    case 0 -> EventKind.PUT;
                    case 1 -> EventKind.DELETE;
                    default -> EventKind.DROP_SCOPE;
                };
                return new Event(mapped, scopeId, seq, key == null ? new byte[0] : key, value);
            } catch (Throwable error) {
                throw propagate(error);
            }
        }

        @Override
        public void close() {
            if (!closed) {
                closed = true;
                try {
                    WATCH_CLOSE.invokeExact(pointer(handle));
                } catch (Throwable error) {
                    throw propagate(error);
                }
                handle = 0;
            }
        }
    }

    private static byte[] takeBytes(MemorySegment value) {
        try {
            long length = value.get(JAVA_LONG, 8);
            MemorySegment data = value.get(ADDRESS, 0);
            byte[] owned = null;
            if (length > 0 && data.address() != 0) {
                owned = data.reinterpret(length).toArray(JAVA_BYTE);
            }
            BYTES_FREE.invokeExact(value);
            return owned;
        } catch (Throwable error) {
            throw propagate(error);
        }
    }

    private static List<Entry> takeEntries(MemorySegment entries) {
        try {
            long count = entries.get(JAVA_LONG, 8);
            MemorySegment pointer = entries.get(ADDRESS, 0);
            List<Entry> owned = new ArrayList<>();
            if (count > 0 && pointer.address() != 0) {
                MemorySegment array = pointer.reinterpret(count * ENTRY.byteSize());
                for (long index = 0; index < count; index += 1) {
                    MemorySegment entry = array.asSlice(index * ENTRY.byteSize(), ENTRY.byteSize());
                    byte[] key = copyBytes(entry.asSlice(0, BYTES.byteSize()));
                    byte[] value = copyBytes(entry.asSlice(BYTES.byteSize(), BYTES.byteSize()));
                    owned.add(new Entry(key == null ? new byte[0] : key, value == null ? new byte[0] : value));
                }
            }
            ENTRIES_FREE.invokeExact(entries);
            return owned;
        } catch (Throwable error) {
            throw propagate(error);
        }
    }

    private static byte[] copyBytes(MemorySegment value) {
        long length = value.get(JAVA_LONG, 8);
        MemorySegment data = value.get(ADDRESS, 0);
        if (length <= 0 || data.address() == 0) {
            return null;
        }
        return data.reinterpret(length).toArray(JAVA_BYTE);
    }

    private static MemorySegment slice(Arena arena, byte[] data) {
        MemorySegment slice = arena.allocate(SLICE);
        if (data.length == 0) {
            slice.set(ADDRESS, 0, MemorySegment.NULL);
            slice.set(JAVA_LONG, 8, 0);
        } else {
            MemorySegment buffer = arena.allocate(data.length);
            MemorySegment.copy(data, 0, buffer, JAVA_BYTE, 0, data.length);
            slice.set(ADDRESS, 0, buffer);
            slice.set(JAVA_LONG, 8, data.length);
        }
        return slice;
    }

    private static MemorySegment emptySlice(Arena arena) {
        return slice(arena, new byte[0]);
    }

    private static MemorySegment cString(Arena arena, String value) {
        byte[] encoded = value.getBytes(StandardCharsets.UTF_8);
        MemorySegment segment = arena.allocate(encoded.length + 1L);
        MemorySegment.copy(encoded, 0, segment, JAVA_BYTE, 0, encoded.length);
        segment.set(JAVA_BYTE, encoded.length, (byte) 0);
        return segment;
    }

    private static byte[] bytes(String value) {
        return value.getBytes(StandardCharsets.UTF_8);
    }

    private static MemorySegment pointer(long address) {
        return address == 0 ? MemorySegment.NULL : MemorySegment.ofAddress(address);
    }

    private static void check(int status) {
        if (status != OK) {
            throw new StorkvException(status, lastErrorMessage());
        }
    }

    private static String lastErrorMessage() {
        try {
            MemorySegment value = (MemorySegment) LAST_ERROR.invokeExact();
            long length = value.get(JAVA_LONG, 8);
            MemorySegment data = value.get(ADDRESS, 0);
            String text = "StorKV operation failed";
            if (length > 0 && data.address() != 0) {
                text = new String(data.reinterpret(length).toArray(JAVA_BYTE), StandardCharsets.UTF_8);
            }
            STRING_FREE.invokeExact(value);
            return text;
        } catch (Throwable error) {
            return "StorKV operation failed";
        }
    }

    private static RuntimeException propagate(Throwable error) {
        if (error instanceof RuntimeException runtime) {
            return runtime;
        }
        return new StorkvException(INTERNAL, error.toString());
    }

    private static MethodHandle downcall(String name, FunctionDescriptor descriptor) {
        MemorySegment symbol = LIBRARY.find(name)
                .orElseThrow(() -> new UnsatisfiedLinkError("missing symbol " + name));
        return LINKER.downcallHandle(symbol, descriptor);
    }

    private static SymbolLookup loadLibrary() {
        String configured = System.getenv("STORKV_FFI_LIB");
        if (configured != null && !configured.isBlank()) {
            return SymbolLookup.libraryLookup(Paths.get(configured), LIBRARY_ARENA);
        }
        SymbolLookup bundled = loadBundledLibrary();
        if (bundled != null) {
            return bundled;
        }
        String[] names = {"storkv_ffi.dll", "libstorkv_ffi.so", "libstorkv_ffi.dylib"};
        List<Path> candidates = new ArrayList<>();
        for (String profile : new String[]{"debug", "release"}) {
            for (String name : names) {
                candidates.add(Paths.get("target", profile, name));
                candidates.add(Paths.get("..", "target", profile, name));
                candidates.add(Paths.get("..", "..", "target", profile, name));
            }
        }
        for (Path candidate : candidates) {
            if (Files.isRegularFile(candidate)) {
                return SymbolLookup.libraryLookup(candidate.toAbsolutePath(), LIBRARY_ARENA);
            }
        }
        throw new IllegalStateException("could not find storkv_ffi; set STORKV_FFI_LIB");
    }

    /**
     * Load the native library that the JAR carries under {@code META-INF/native}.
     *
     * @return the symbol lookup, or {@code null} when the JAR carries no library
     */
    private static SymbolLookup loadBundledLibrary() {
        String resource = "/META-INF/native/" + platformId() + "/" + libraryFileName();
        try (InputStream stream = StorkvClient.class.getResourceAsStream(resource)) {
            if (stream == null) {
                return null;
            }
            Path directory = Files.createTempDirectory("storkv-native");
            Path target = directory.resolve(libraryFileName());
            Files.copy(stream, target, StandardCopyOption.REPLACE_EXISTING);
            target.toFile().deleteOnExit();
            directory.toFile().deleteOnExit();
            return SymbolLookup.libraryLookup(target, LIBRARY_ARENA);
        } catch (IOException error) {
            throw new IllegalStateException("cannot load bundled library " + resource, error);
        }
    }

    private static String platformId() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        String name = os.contains("win") ? "windows" : os.contains("mac") ? "macos" : "linux";
        String arch = System.getProperty("os.arch", "").toLowerCase(Locale.ROOT);
        String bits = arch.contains("aarch64") || arch.contains("arm64") ? "aarch64" : "x86_64";
        return name + "-" + bits;
    }

    private static String libraryFileName() {
        String os = System.getProperty("os.name", "").toLowerCase(Locale.ROOT);
        if (os.contains("win")) {
            return "storkv_ffi.dll";
        }
        if (os.contains("mac")) {
            return "libstorkv_ffi.dylib";
        }
        return "libstorkv_ffi.so";
    }

    /**
     * Exception raised when the native layer returns a non-success status.
     */
    public static final class StorkvException extends RuntimeException {
        private static final long serialVersionUID = 1L;

        private final int status;

        public StorkvException(int status, String message) {
            super(message);
            this.status = status;
        }

        public int status() {
            return status;
        }
    }

    /**
     * Return an optional native error message for diagnostics.
     *
     * @return the last error message on this thread
     */
    public static Optional<String> lastError() {
        return Optional.ofNullable(lastErrorMessage());
    }

    /**
     * Return the Java runtime method type used for callback helpers.
     *
     * @return method type for the transaction callback
     */
    public static MethodType callbackType() {
        return MethodType.methodType(int.class, MemorySegment.class, MemorySegment.class);
    }
}
