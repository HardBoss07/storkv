#include "storkv.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

#ifdef _WIN32
#include <direct.h>
#else
#include <sys/stat.h>
#endif

static int failures = 0;

#define CHECK(condition)                                                        \
    do {                                                                        \
        if (!(condition)) {                                                     \
            fprintf(stderr, "FAIL %s:%d: %s\n", __FILE__, __LINE__, #condition); \
            failures += 1;                                                      \
        }                                                                       \
    } while (0)

#define CHECK_STATUS(expression)                                                \
    do {                                                                        \
        StorkvStatus status_ = (expression);                                    \
        if (status_ != Ok) {                                                    \
            fprintf(stderr, "FAIL %s:%d: %s returned %d\n", __FILE__, __LINE__, \
                    #expression, (int)status_);                                  \
            failures += 1;                                                      \
        }                                                                       \
    } while (0)

static StorkvSlice slice_of(const char *text) {
    StorkvSlice slice;
    slice.data = (const uint8_t *)text;
    slice.len = strlen(text);
    return slice;
}

static int bytes_equal(StorkvBytes value, const char *expected) {
    size_t expected_len = strlen(expected);
    if (value.len != expected_len) {
        return 0;
    }
    if (expected_len == 0) {
        return 1;
    }
    return memcmp(value.data, expected, expected_len) == 0;
}

static void free_value(StorkvBytes value) {
    storkv_bytes_free(value);
}

static void check_crud(const char *uri, const char *label) {
    StorkvPoolHandle *pool = NULL;
    StorkvScopeHandle *scope = NULL;
    StorkvBytes value;
    int32_t flag = 0;

    printf("C FFI: %s (%s)\n", label, uri);
    CHECK_STATUS(storkv_pool_open(uri, NULL, &pool));
    CHECK(pool != NULL);
    if (pool == NULL) {
        return;
    }
    CHECK_STATUS(storkv_scope_open(pool, "default", &scope));
    CHECK(scope != NULL);
    if (scope == NULL) {
        storkv_pool_close(pool);
        return;
    }

    CHECK_STATUS(storkv_put(scope, slice_of("c:greeting"), slice_of("hello")));
    CHECK_STATUS(storkv_get(scope, slice_of("c:greeting"), &value));
    CHECK(bytes_equal(value, "hello"));
    free_value(value);

    CHECK_STATUS(storkv_exists(scope, slice_of("c:greeting"), &flag));
    CHECK(flag == 1);
    CHECK_STATUS(storkv_delete(scope, slice_of("c:greeting")));
    CHECK_STATUS(storkv_exists(scope, slice_of("c:greeting"), &flag));
    CHECK(flag == 0);

    CHECK_STATUS(storkv_put_with_ttl(scope, slice_of("c:session"), slice_of("live"), 30000));
    CHECK_STATUS(storkv_get_and_touch(scope, slice_of("c:session"), 5000, &value));
    CHECK(bytes_equal(value, "live"));
    free_value(value);

    CHECK_STATUS(storkv_put(scope, slice_of("scan:1"), slice_of("one")));
    CHECK_STATUS(storkv_put(scope, slice_of("scan:2"), slice_of("two")));
    {
        StorkvEntries entries;
        memset(&entries, 0, sizeof(entries));
        CHECK_STATUS(storkv_scan_prefix(scope, slice_of("scan:"), &entries));
        CHECK(entries.len == 2);
        storkv_entries_free(entries);
    }
    {
        StorkvEntries entries;
        memset(&entries, 0, sizeof(entries));
        CHECK_STATUS(storkv_scan_range(scope, slice_of("scan:1"), slice_of("scan:3"), &entries));
        CHECK(entries.len == 2);
        storkv_entries_free(entries);
    }

    {
        StorkvPutCondition condition;
        StorkvPutIfResult result;
        memset(&condition, 0, sizeof(condition));
        memset(&result, 0, sizeof(result));
        condition.kind = Absent;
        CHECK_STATUS(storkv_put_if(scope, slice_of("cas:key"), &condition, slice_of("first"), 0,
                                   &result));
        CHECK(result.applied == 1);
        CHECK_STATUS(storkv_put_if(scope, slice_of("cas:key"), &condition, slice_of("second"), 0,
                                   &result));
        CHECK(result.applied == 0);
        condition.kind = ExpectedSeq;
        condition.expected_seq = result.seq;
        CHECK_STATUS(storkv_put_if(scope, slice_of("cas:key"), &condition, slice_of("second"), 0,
                                   &result));
        CHECK(result.applied == 1);
    }

    {
        StorkvTxHandle *tx = NULL;
        CHECK_STATUS(storkv_tx_begin(pool, &tx));
        CHECK_STATUS(storkv_tx_put(tx, scope, slice_of("tx-key"), slice_of("tx-value")));
        CHECK_STATUS(storkv_tx_get(tx, scope, slice_of("tx-key"), &value));
        CHECK(bytes_equal(value, "tx-value"));
        free_value(value);
        CHECK_STATUS(storkv_tx_commit(tx));
        CHECK_STATUS(storkv_get(scope, slice_of("tx-key"), &value));
        CHECK(bytes_equal(value, "tx-value"));
        free_value(value);

        tx = NULL;
        CHECK_STATUS(storkv_tx_begin(pool, &tx));
        CHECK_STATUS(storkv_tx_put(tx, scope, slice_of("rolled"), slice_of("back")));
        storkv_tx_rollback(tx);
        CHECK_STATUS(storkv_get(scope, slice_of("rolled"), &value));
        CHECK(value.len == 0);
        free_value(value);
    }

    {
        StorkvWatchHandle *watch = NULL;
        StorkvWatchEvent event;
        memset(&event, 0, sizeof(event));
        CHECK_STATUS(storkv_watch_prefix(scope, slice_of("events:"), &watch));
        CHECK_STATUS(storkv_put(scope, slice_of("events:1"), slice_of("payload")));
        CHECK_STATUS(storkv_watch_next(watch, &event, 5000));
        CHECK(event.kind == Put);
        CHECK(event.key.len == 8);
        CHECK(event.value.len == 7);
        storkv_bytes_free(event.key);
        storkv_bytes_free(event.value);
        storkv_watch_close(watch);
    }

    CHECK_STATUS(storkv_pool_is_closed(pool, &flag));
    CHECK(flag == 0);
    storkv_scope_close(scope);
    storkv_pool_close(pool);
}

static void check_error_paths(void) {
    StorkvBytes value;
    StorkvPoolHandle *pool = NULL;
    memset(&value, 0, sizeof(value));
    CHECK(storkv_get(NULL, slice_of("key"), &value) == NullPointer);
    CHECK(storkv_pool_open(NULL, NULL, &pool) == NullPointer);
    CHECK(pool == NULL);
    {
        StorkvString message = storkv_last_error_message();
        storkv_string_free(message);
    }
}

int main(int argc, char **argv) {
    char dir[1024];
    char uri[1100];
    const char *server_uri;

    if (argc > 1) {
        snprintf(dir, sizeof(dir), "%s", argv[1]);
    } else {
        snprintf(dir, sizeof(dir), "storkv-c-test-%lu", (unsigned long)time(NULL));
    }
#ifdef _WIN32
    _mkdir(dir);
#else
    mkdir(dir, 0700);
#endif
    snprintf(uri, sizeof(uri), "file://%s", dir);

    check_error_paths();
    check_crud(uri, "embedded");

    server_uri = getenv("STORKV_TEST_URI");
    if (server_uri != NULL && server_uri[0] != '\0') {
        check_crud(server_uri, "networked");
    } else {
        printf("C FFI: networked tests skipped (STORKV_TEST_URI is unset)\n");
    }

    if (failures == 0) {
        printf("C FFI: all checks passed\n");
        return 0;
    }
    fprintf(stderr, "C FFI: %d check(s) failed\n", failures);
    return 1;
}
