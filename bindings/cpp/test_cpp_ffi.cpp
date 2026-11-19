#include "storkv.hpp"

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <filesystem>
#include <string>

namespace {

int failures = 0;

void check(bool condition, const char* expression, int line) {
    if (!condition) {
        std::fprintf(stderr, "FAIL bindings/cpp/test_cpp_ffi.cpp:%d: %s\n", line, expression);
        failures += 1;
    }
}

#define CHECK(expression) check((expression), #expression, __LINE__)

std::string as_text(const std::vector<std::uint8_t>& bytes) {
    return std::string(reinterpret_cast<const char*>(bytes.data()), bytes.size());
}

void run_crud(const std::string& uri, const std::string& label) {
    std::printf("C++ FFI: %s (%s)\n", label.c_str(), uri.c_str());

    storkv::Pool pool = storkv::Pool::connect(uri);
    storkv::Scope scope = pool.scope("default");

    scope.put("cpp:greeting", "hello");
    auto value = scope.get("cpp:greeting");
    CHECK(value.has_value());
    CHECK(value.has_value() && as_text(*value) == "hello");
    CHECK(scope.exists("cpp:greeting"));
    scope.erase("cpp:greeting");
    CHECK(!scope.exists("cpp:greeting"));

    scope.put_with_ttl("cpp:session", "live", 30000);
    auto touched = scope.get_and_touch("cpp:session", 5000);
    CHECK(touched.has_value() && as_text(*touched) == "live");

    std::string prefix = "cpp:scan:";
    scope.put(prefix + "1", "one");
    scope.put(prefix + "2", "two");
    auto entries = scope.scan_prefix(prefix);
    CHECK(entries.size() == 2);
    auto ranged = scope.scan_range(prefix + "1", prefix + "3");
    CHECK(ranged.size() == 2);

    {
        storkv::Transaction tx = pool.begin();
        tx.put(scope, "cpp:tx", "value");
        auto inside = tx.get(scope, "cpp:tx");
        CHECK(inside.has_value() && as_text(*inside) == "value");
        tx.commit();
        auto committed = scope.get("cpp:tx");
        CHECK(committed.has_value() && as_text(*committed) == "value");
    }
    {
        storkv::Transaction tx = pool.begin();
        tx.put(scope, "cpp:rolled", "back");
        tx.rollback();
        CHECK(!scope.get("cpp:rolled").has_value());
    }

    storkv::Watch watch = scope.watch_prefix("cpp:events:");
    scope.put("cpp:events:1", "payload");
    auto event = watch.next(5000);
    CHECK(event.has_value());
    if (event.has_value()) {
        CHECK(event->kind == storkv::EventKind::Put);
        CHECK(as_text(event->key) == "cpp:events:1");
        CHECK(as_text(event->value) == "payload");
    }
    watch.close();

    CHECK(!pool.is_closed());
}

}  // namespace

int main(int argc, char** argv) {
    std::filesystem::path dir;
    if (argc > 1) {
        dir = argv[1];
    } else {
        dir = std::filesystem::temp_directory_path() /
              ("storkv-cpp-test-" + std::to_string(std::time(nullptr)));
    }
    std::filesystem::create_directories(dir);
    run_crud("file://" + dir.string(), "embedded");

    if (const char* server = std::getenv("STORKV_TEST_URI");
        server != nullptr && server[0] != '\0') {
        run_crud(server, "networked");
    } else {
        std::printf("C++ FFI: networked tests skipped (STORKV_TEST_URI is unset)\n");
    }

    if (failures == 0) {
        std::printf("C++ FFI: all checks passed\n");
        return 0;
    }
    std::fprintf(stderr, "C++ FFI: %d check(s) failed\n", failures);
    return 1;
}
