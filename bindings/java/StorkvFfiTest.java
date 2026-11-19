import storkv.StorkvClient;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;

/**
 * Runnable smoke test for the StorKV Java FFM binding.
 */
public final class StorkvFfiTest {
    private static int failures = 0;

    private StorkvFfiTest() {
    }

    public static void main(String[] args) throws Exception {
        Path directory = Files.createTempDirectory("storkv-java-test-");
        try {
            exercise("file://" + directory, "embedded");
            String server = System.getenv("STORKV_TEST_URI");
            if (server != null && !server.isBlank()) {
                exercise(server, "networked");
            } else {
                System.out.println("Java FFM: networked tests skipped (STORKV_TEST_URI is unset)");
            }
        } finally {
            deleteRecursively(directory);
        }
        if (failures == 0) {
            System.out.println("Java FFM: all checks passed");
            return;
        }
        System.err.println("Java FFM: " + failures + " check(s) failed");
        System.exit(1);
    }

    private static void exercise(String uri, String label) {
        System.out.println("Java FFM: " + label + " (" + uri + ")");
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

            scope.putWithTtl("java:session", "live", 30000);
            check("live".equals(text(scope.getAndTouch("java:session", 5000))), "get and touch");

            scope.put("java:scan:1", "one");
            scope.put("java:scan:2", "two");
            List<StorkvClient.Entry> entries = scope.scanPrefix("java:scan:");
            check(entries.size() == 2, "scan prefix count");
            List<StorkvClient.Entry> ranged = scope.scanRange("java:scan:1", "java:scan:3");
            check(ranged.size() == 2, "scan range count");

            try (StorkvClient.Transaction transaction = pool.transaction()) {
                transaction.put(scope, "java:tx", "value");
                check("value".equals(text(transaction.get(scope, "java:tx"))), "tx read own write");
                transaction.commit();
            }
            check("value".equals(text(scope.get("java:tx"))), "tx commit");

            StorkvClient.Transaction rolled = pool.transaction();
            rolled.put(scope, "java:rolled", "back");
            rolled.rollback();
            check(scope.get("java:rolled") == null, "tx rollback");

            try (StorkvClient.Watch watch = scope.watchPrefix("java:events:")) {
                scope.put("java:events:1", "payload");
                StorkvClient.Event event = watch.next(5000);
                check(event != null, "watch event delivered");
                if (event != null) {
                    check(event.kind() == StorkvClient.EventKind.PUT, "watch kind");
                    check("java:events:1".equals(text(event.key())), "watch key");
                    check("payload".equals(text(event.value())), "watch value");
                }
            }

            StorkvClient.Scope closedScope = pool.scope("default");
            closedScope.close();
            try {
                closedScope.get("key");
                check(false, "closed scope rejects calls");
            } catch (StorkvClient.StorkvException error) {
                check(error.status() == StorkvClient.INVALID_ARGUMENT, "closed scope status");
            }
        } catch (RuntimeException error) {
            failures += 1;
            System.err.println("Java FFM failure: " + error);
            error.printStackTrace(System.err);
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
            System.err.println("FAIL Java FFM: " + what);
        }
    }

    private static void deleteRecursively(Path directory) {
        try (var paths = Files.walk(directory)) {
            paths.sorted(java.util.Comparator.reverseOrder()).forEach(path -> {
                try {
                    Files.deleteIfExists(path);
                } catch (Exception ignored) {
                    ignored.getClass();
                }
            });
        } catch (Exception ignored) {
            ignored.getClass();
        }
    }
}
