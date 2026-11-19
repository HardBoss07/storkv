#pragma once

#include "storkv.h"

#include <cstdint>
#include <cstring>
#include <optional>
#include <span>
#include <stdexcept>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace storkv {

class Error : public std::runtime_error {
public:
    Error(StorkvStatus status, std::string message)
        : std::runtime_error(std::move(message)), status_(status) {}

    StorkvStatus status() const noexcept { return status_; }

private:
    StorkvStatus status_;
};

namespace detail {

inline void check(StorkvStatus status, const char* operation) {
    if (status == Ok) {
        return;
    }
    StorkvString message = storkv_last_error_message();
    std::string text = message.data != nullptr && message.len > 0
                           ? std::string(message.data, message.len)
                           : std::string(operation);
    storkv_string_free(message);
    throw Error(status, text);
}

inline StorkvSlice slice(std::span<const std::uint8_t> bytes) {
    StorkvSlice value{};
    value.data = bytes.empty() ? nullptr : bytes.data();
    value.len = bytes.size();
    return value;
}

inline StorkvSlice slice(std::string_view text) {
    return slice(std::span<const std::uint8_t>(
        reinterpret_cast<const std::uint8_t*>(text.data()), text.size()));
}

inline std::vector<std::uint8_t> take(StorkvBytes bytes) {
    std::vector<std::uint8_t> owned;
    if (bytes.data != nullptr && bytes.len > 0) {
        owned.assign(bytes.data, bytes.data + bytes.len);
    }
    storkv_bytes_free(bytes);
    return owned;
}

inline std::optional<std::vector<std::uint8_t>> optional_value(StorkvBytes bytes) {
    std::vector<std::uint8_t> owned = take(bytes);
    if (owned.empty()) {
        return std::nullopt;
    }
    return owned;
}

using Entry = std::pair<std::vector<std::uint8_t>, std::vector<std::uint8_t>>;

inline std::vector<Entry> take(StorkvEntries entries) {
    std::vector<Entry> owned;
    for (std::size_t index = 0; index < entries.len; ++index) {
        const StorkvEntry& entry = entries.entries[index];
        std::vector<std::uint8_t> key;
        std::vector<std::uint8_t> value;
        if (entry.key.data != nullptr && entry.key.len > 0) {
            key.assign(entry.key.data, entry.key.data + entry.key.len);
        }
        if (entry.value.data != nullptr && entry.value.len > 0) {
            value.assign(entry.value.data, entry.value.data + entry.value.len);
        }
        owned.emplace_back(std::move(key), std::move(value));
    }
    storkv_entries_free(entries);
    return owned;
}

}  // namespace detail

struct Options {
    bool health_check = true;
    std::uint32_t max_connections = 10;
    std::uint32_t min_connections = 1;
    std::uint64_t acquire_timeout_ms = 0;
    std::uint64_t idle_timeout_ms = 0;
};

enum class EventKind {
    Put = 0,
    Delete = 1,
    DropScope = 2,
};

struct Event {
    EventKind kind;
    std::uint32_t scope_id;
    std::uint64_t seq;
    std::vector<std::uint8_t> key;
    std::vector<std::uint8_t> value;
};

class Pool;
class Scope;

class Watch {
public:
    Watch() = default;
    Watch(Watch&& other) noexcept : handle_(std::exchange(other.handle_, nullptr)) {}
    Watch& operator=(Watch&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = std::exchange(other.handle_, nullptr);
        }
        return *this;
    }
    Watch(const Watch&) = delete;
    Watch& operator=(const Watch&) = delete;
    ~Watch() { reset(); }

    std::optional<Event> next(std::uint64_t timeout_ms) {
        if (handle_ == nullptr) {
            return std::nullopt;
        }
        StorkvWatchEvent raw{};
        StorkvStatus status = storkv_watch_next(handle_, &raw, timeout_ms);
        if (status == Timeout) {
            return std::nullopt;
        }
        detail::check(status, "storkv_watch_next");
        Event event{};
        event.kind = static_cast<EventKind>(raw.kind);
        event.scope_id = raw.scope_id;
        event.seq = raw.seq;
        event.key = detail::take(raw.key);
        event.value = detail::take(raw.value);
        return event;
    }

    void close() { reset(); }
    explicit operator bool() const noexcept { return handle_ != nullptr; }

private:
    friend class Scope;

    explicit Watch(StorkvWatchHandle* handle) : handle_(handle) {}

    void reset() {
        if (handle_ != nullptr) {
            storkv_watch_close(handle_);
            handle_ = nullptr;
        }
    }

    StorkvWatchHandle* handle_ = nullptr;
};

class Transaction {
public:
    Transaction() = default;
    Transaction(Transaction&& other) noexcept : handle_(std::exchange(other.handle_, nullptr)) {}
    Transaction& operator=(Transaction&& other) noexcept {
        if (this != &other) {
            rollback();
            handle_ = std::exchange(other.handle_, nullptr);
        }
        return *this;
    }
    Transaction(const Transaction&) = delete;
    Transaction& operator=(const Transaction&) = delete;
    ~Transaction() { rollback(); }

    static Transaction begin(Pool& pool);

    std::optional<std::vector<std::uint8_t>> get(const Scope& scope, std::string_view key);
    std::optional<std::vector<std::uint8_t>> get(const Scope& scope,
                                                 std::span<const std::uint8_t> key);
    void put(const Scope& scope, std::string_view key, std::string_view value);
    void put(const Scope& scope, std::span<const std::uint8_t> key,
             std::span<const std::uint8_t> value);
    void put_with_ttl(const Scope& scope, std::string_view key, std::string_view value,
                      std::uint64_t ttl_ms);
    void erase(const Scope& scope, std::string_view key);
    void commit();
    void rollback() noexcept;

private:
    friend class Pool;

    explicit Transaction(StorkvTxHandle* handle) : handle_(handle) {}
    StorkvTxHandle* require() const;

    StorkvTxHandle* handle_ = nullptr;
};

class Scope {
public:
    Scope() = default;
    Scope(Scope&& other) noexcept : handle_(std::exchange(other.handle_, nullptr)) {}
    Scope& operator=(Scope&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = std::exchange(other.handle_, nullptr);
        }
        return *this;
    }
    Scope(const Scope&) = delete;
    Scope& operator=(const Scope&) = delete;
    ~Scope() { reset(); }

    std::optional<std::vector<std::uint8_t>> get(std::string_view key) const {
        return get(detail::slice(key));
    }

    std::optional<std::vector<std::uint8_t>> get(std::span<const std::uint8_t> key) const {
        return get(detail::slice(key));
    }

    void put(std::string_view key, std::string_view value) const {
        put(detail::slice(key), detail::slice(value));
    }

    void put(std::span<const std::uint8_t> key, std::span<const std::uint8_t> value) const {
        detail::check(storkv_put(require(), detail::slice(key), detail::slice(value)),
                      "storkv_put");
    }

    void put_with_ttl(std::string_view key, std::string_view value,
                      std::uint64_t ttl_ms) const {
        detail::check(storkv_put_with_ttl(require(), detail::slice(key), detail::slice(value),
                                          ttl_ms),
                      "storkv_put_with_ttl");
    }

    std::optional<std::vector<std::uint8_t>> get_and_touch(std::string_view key,
                                                           std::uint64_t ttl_ms) const {
        StorkvBytes value{};
        detail::check(storkv_get_and_touch(require(), detail::slice(key), ttl_ms, &value),
                      "storkv_get_and_touch");
        return detail::optional_value(value);
    }

    void erase(std::string_view key) const {
        detail::check(storkv_delete(require(), detail::slice(key)), "storkv_delete");
    }

    bool exists(std::string_view key) const {
        std::int32_t flag = 0;
        detail::check(storkv_exists(require(), detail::slice(key), &flag), "storkv_exists");
        return flag != 0;
    }

    std::vector<detail::Entry> scan_prefix(std::string_view prefix) const {
        StorkvEntries entries{};
        detail::check(storkv_scan_prefix(require(), detail::slice(prefix), &entries),
                      "storkv_scan_prefix");
        return detail::take(entries);
    }

    std::vector<detail::Entry> scan_range(std::optional<std::string_view> start,
                                          std::optional<std::string_view> end) const {
        StorkvSlice start_slice{};
        StorkvSlice end_slice{};
        if (start.has_value()) {
            start_slice = detail::slice(*start);
        }
        if (end.has_value()) {
            end_slice = detail::slice(*end);
        }
        StorkvEntries entries{};
        detail::check(storkv_scan_range(require(), start_slice, end_slice, &entries),
                      "storkv_scan_range");
        return detail::take(entries);
    }

    Watch watch_prefix(std::string_view prefix) const {
        StorkvWatchHandle* watch = nullptr;
        detail::check(storkv_watch_prefix(require(), detail::slice(prefix), &watch),
                      "storkv_watch_prefix");
        return Watch(watch);
    }

    StorkvScopeHandle* raw() const { return handle_; }

private:
    friend class Pool;

    explicit Scope(StorkvScopeHandle* handle) : handle_(handle) {}

    std::optional<std::vector<std::uint8_t>> get(StorkvSlice key) const {
        StorkvBytes value{};
        detail::check(storkv_get(require(), key, &value), "storkv_get");
        return detail::optional_value(value);
    }

    void put(StorkvSlice key, StorkvSlice value) const {
        detail::check(storkv_put(require(), key, value), "storkv_put");
    }

    StorkvScopeHandle* require() const {
        if (handle_ == nullptr) {
            throw Error(InvalidArgument, "scope handle is closed");
        }
        return handle_;
    }

    void reset() {
        if (handle_ != nullptr) {
            storkv_scope_close(handle_);
            handle_ = nullptr;
        }
    }

    StorkvScopeHandle* handle_ = nullptr;
};

class Pool {
public:
    Pool() = default;
    Pool(Pool&& other) noexcept : handle_(std::exchange(other.handle_, nullptr)) {}
    Pool& operator=(Pool&& other) noexcept {
        if (this != &other) {
            reset();
            handle_ = std::exchange(other.handle_, nullptr);
        }
        return *this;
    }
    Pool(const Pool&) = delete;
    Pool& operator=(const Pool&) = delete;
    ~Pool() { reset(); }

    static Pool connect(std::string_view uri, Options options = {}) {
        StorkvPoolConfig config{};
        config.health_check = options.health_check;
        config.max_connections = options.max_connections;
        config.min_connections = options.min_connections;
        config.acquire_timeout_ms = options.acquire_timeout_ms;
        config.idle_timeout_ms = options.idle_timeout_ms;
        std::string owned(uri);
        StorkvPoolHandle* handle = nullptr;
        detail::check(storkv_pool_open(owned.c_str(), &config, &handle), "storkv_pool_open");
        return Pool(handle);
    }

    Scope scope(std::string_view name) const {
        std::string owned(name);
        StorkvScopeHandle* handle = nullptr;
        detail::check(storkv_scope_open(require(), owned.c_str(), &handle),
                      "storkv_scope_open");
        return Scope(handle);
    }

    Transaction begin() {
        StorkvTxHandle* handle = nullptr;
        detail::check(storkv_tx_begin(require(), &handle), "storkv_tx_begin");
        return Transaction(handle);
    }

    bool is_closed() const {
        std::int32_t flag = 0;
        detail::check(storkv_pool_is_closed(require(), &flag), "storkv_pool_is_closed");
        return flag != 0;
    }

    void close() { reset(); }
    explicit operator bool() const noexcept { return handle_ != nullptr; }

    StorkvPoolHandle* raw() const { return handle_; }

private:
    explicit Pool(StorkvPoolHandle* handle) : handle_(handle) {}

    StorkvPoolHandle* require() const {
        if (handle_ == nullptr) {
            throw Error(InvalidArgument, "pool handle is closed");
        }
        return handle_;
    }

    void reset() {
        if (handle_ != nullptr) {
            storkv_pool_close(handle_);
            handle_ = nullptr;
        }
    }

    StorkvPoolHandle* handle_ = nullptr;
};

inline Transaction Transaction::begin(Pool& pool) {
    return pool.begin();
}

inline StorkvTxHandle* Transaction::require() const {
    if (handle_ == nullptr) {
        throw Error(InvalidArgument, "transaction handle is closed");
    }
    return handle_;
}

inline std::optional<std::vector<std::uint8_t>> Transaction::get(const Scope& scope,
                                                                std::string_view key) {
    return get(scope, std::span<const std::uint8_t>(
                          reinterpret_cast<const std::uint8_t*>(key.data()), key.size()));
}

inline std::optional<std::vector<std::uint8_t>> Transaction::get(
    const Scope& scope, std::span<const std::uint8_t> key) {
    StorkvBytes value{};
    detail::check(storkv_tx_get(require(), scope.raw(), detail::slice(key), &value),
                  "storkv_tx_get");
    return detail::optional_value(value);
}

inline void Transaction::put(const Scope& scope, std::string_view key, std::string_view value) {
    put(scope,
        std::span<const std::uint8_t>(reinterpret_cast<const std::uint8_t*>(key.data()),
                                      key.size()),
        std::span<const std::uint8_t>(reinterpret_cast<const std::uint8_t*>(value.data()),
                                      value.size()));
}

inline void Transaction::put(const Scope& scope, std::span<const std::uint8_t> key,
                             std::span<const std::uint8_t> value) {
    detail::check(storkv_tx_put(require(), scope.raw(), detail::slice(key), detail::slice(value)),
                  "storkv_tx_put");
}

inline void Transaction::put_with_ttl(const Scope& scope, std::string_view key,
                                      std::string_view value, std::uint64_t ttl_ms) {
    detail::check(storkv_tx_put_with_ttl(require(), scope.raw(), detail::slice(key),
                                         detail::slice(value), ttl_ms),
                  "storkv_tx_put_with_ttl");
}

inline void Transaction::erase(const Scope& scope, std::string_view key) {
    detail::check(storkv_tx_delete(require(), scope.raw(), detail::slice(key)),
                  "storkv_tx_delete");
}

inline void Transaction::commit() {
    StorkvTxHandle* handle = std::exchange(handle_, nullptr);
    detail::check(storkv_tx_commit(handle), "storkv_tx_commit");
}

inline void Transaction::rollback() noexcept {
    if (handle_ != nullptr) {
        storkv_tx_rollback(std::exchange(handle_, nullptr));
    }
}

}  // namespace storkv
