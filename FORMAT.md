# StorKV On-Disk Binary Format Specification (v1)

All multi-byte integers are encoded in **Little-Endian (`LE`)** byte order unless explicitly marked **Big-Endian (`BE`)** for lexicographical byte sorting. All checksums use **CRC32C (Castagnoli)** (`0x1EDC6F41` polynomial).

### Global Size Limits

- **`MAX_KEY_LEN`:** `65,535` bytes (`u16::MAX`)
- **`MAX_VAL_LEN`:** `268,435,456` bytes (`256 MiB`)
- **File Number Formatting:** `u64` file identifiers (`MANIFEST`, `.wal`, `.sst`) are formatted as **16-character lowercase hex** (`%016x`). `u32` `scope_id` directories are formatted as **8-character lowercase hex** (`%08x`).

---

## 1. Directory Layout

```text
<db_root>/
├── LOCK                            # 0-byte advisory flock() file
├── CURRENT                         # ASCII filename of active manifest + '\n'
├── MANIFEST-0000000000000001       # Append-only VersionEdit log
├── wal/
│   ├── 0000000000000001.wal        # Sealed WAL segment
│   └── 0000000000000002.wal        # Active WAL segment
├── scopes/
│   ├── 00000000/                   # Hex-encoded Scope ID (0 = "default")
│   │   ├── 0000000000000001.sst    # Immutable SSTable file
│   │   └── 0000000000000002.sst
│   └── 00000001/                   # Hex-encoded Scope ID (1 = e.g. "sessions")
│       └── 0000000000000003.sst
└── checkpoints/
    └── <checkpoint_id>/            # Self-contained hard-linked point-in-time snapshot
```

---

## 2. Internal Key Encoding

Inside `MemTable`s and `SSTable` data blocks, every entry key is encoded as an **Internal Key** so that standard bytewise comparison (`[u8]::cmp`) orders entries by:

1. `user_key` **ascending** (lexicographical)
2. `sequence_number` **descending** (newest version first)

```text
+-----------------------+------------------------+-------------------------+
| user_key ([u8; N])    | inverted_tag (u64 BE)  | expires_at_ms (u64 LE)  |
+-----------------------+------------------------+-------------------------+
|<------ N bytes ------>|<------- 8 bytes ------>|<------- 8 bytes ------->|
```

- **`inverted_tag` (`u64 BE`):** Computed as `!((seq << 8) | (value_type as u64))` and serialized in **Big-Endian**. Because higher `seq` values produce smaller bitwise-inverted values, raw byte comparison places the newest version of a key immediately first.
  - `value_type` values:
    - `0x01` = `Value`
    - `0x02` = `Tombstone` (point delete; value is empty)
    - `0x03` = `MergeOperand`
    - `0x04` = `RangeTombstone` (`user_key` is `start_key` inclusive; entry value holds `end_key` exclusive)
- **`expires_at_ms` (`u64 LE`):** Unix epoch milliseconds at which the entry expires. `0` indicates no expiration (permanent).

---

## 3. Write-Ahead Log (`*.wal`) Format

### 3.1 WAL File Header (16 Bytes)

Written once at byte offset `0` when a new `.wal` segment is created.

| Offset | Size | Type      | Field            | Description                                       |
| :----- | :--- | :-------- | :--------------- | :------------------------------------------------ |
| `0`    | `8`  | `[u8; 8]` | `magic`          | ASCII `b"STORKWAL"` (`0x53 54 4F 52 4B 57 41 4C`) |
| `8`    | `2`  | `u16 LE`  | `version`        | Format version (`1`)                              |
| `10`   | `2`  | `u16 LE`  | `flags`          | Bitmask (`0x0001` = sealed segment)               |
| `12`   | `4`  | `u32 LE`  | `created_at_sec` | Segment creation Unix timestamp (seconds)         |

### 3.2 WAL Batch Frame

Every committed `WriteBatch` or `Transaction` appends one contiguous frame starting at offset `16`.

```text
+----------+-----------+-----------+----------+-------+----------+--------------------+
| crc32c   | frame_len | batch_seq | op_count | flags | reserved | payload            |
| (u32 LE) | (u32 LE)  | (u64 LE)  | (u32 LE) | (u16) | (u16)    | ([u8; ...])        |
+----------+-----------+-----------+----------+-------+----------+--------------------+
|<------- 24-byte Fixed Frame Header -------------------------->|<-- Variable Body -->|
```

- **`crc32c` (`u32 LE`):** Checksum computed over bytes `[4 .. 8 + frame_len]` (covering `frame_len`, `batch_seq`, `op_count`, `flags`, `reserved`, and `payload`).
- **`frame_len` (`u32 LE`):** Byte length of `batch_seq (8) + op_count (4) + flags (2) + reserved (2) + payload`.
- **`batch_seq` (`u64 LE`):** Starting `SequenceNumber` assigned to the first operation in this batch. Operation `i` receives `batch_seq + i`.
- **`op_count` (`u32 LE`):** Number of serialized operations in `payload`.
- **`flags` (`u16 LE`):**
  - `0x0000`: Uncompressed payload
  - `0x0001`: LZ4-compressed payload (prefixed with `uncompressed_len: u32 LE`)

### 3.3 WAL Operation Records (`WalOp`)

Within `payload`, `op_count` operations are packed sequentially. Writers must reject any operation where `key_len > MAX_KEY_LEN` (`65,535`) or `val_len > MAX_VAL_LEN` (`256 MiB`) prior to WAL serialization.

| Offset         | Size      | Type     | Field           | Description                                                                          |
| :------------- | :-------- | :------- | :-------------- | :----------------------------------------------------------------------------------- |
| `0`            | `1`       | `u8`     | `opcode`        | `0x01` Put, `0x02` Delete, `0x03` Merge, `0x04` DeleteRange, `0x05` DropScope        |
| `1`            | `4`       | `u32 LE` | `scope_id`      | Target Scope ID                                                                      |
| `5`            | `8`       | `u64 LE` | `expires_at_ms` | Expiration timestamp (`0` if permanent; `0` for non-Put ops)                         |
| `13`           | `4`       | `u32 LE` | `key_len`       | Byte length of `key` (`0` for `DropScope`; must be `<= 65,535`)                      |
| `17`           | `key_len` | `[u8]`   | `key`           | User key (or `start_key` for `DeleteRange`)                                          |
| `17 + key_len` | `4`       | `u32 LE` | `val_len`       | Byte length of `val` (`0` for `Delete`/`DropScope`; `end_key_len` for `DeleteRange`) |
| `21 + key_len` | `val_len` | `[u8]`   | `val`           | Value bytes (or `end_key` for `DeleteRange`)                                         |

---

## 4. Sorted String Table (`*.sst`) Format

SSTables are immutable files written sequentially from offset `0` to `EOF`.

```text
+-----------------------------------+
| Data Block 0                      |
+-----------------------------------+
| Data Block 1                      |
+-----------------------------------+
| ...                               |
+-----------------------------------+
| Filter Block (Blocked Bloom)      |
+-----------------------------------+
| Meta Block (Stats & Bounds)       |
+-----------------------------------+
| Index Block (Data Block Handles)  |
+-----------------------------------+
| Footer (Fixed 56 Bytes)           |
+-----------------------------------+
```

### 4.1 BlockHandle Encoding (16 Bytes)

Used in the Index Block and Footer to locate blocks within the file:

- `offset: u64 LE` (8 bytes): Byte offset from start of `.sst` file.
- `size: u64 LE` (8 bytes): **On-disk stored byte length** of the block payload (compressed size if compressed, raw size if uncompressed), excluding the 5-byte Block Trailer.

### 4.2 Block Trailer & Compression Framing (5 Bytes)

Appended immediately after every Data, Filter, Meta, and Index block:

- `compression_type: u8` (`0x00` = None, `0x01` = LZ4, `0x02` = Zstd)
- `crc32c: u32 LE` (Checksum over the stored block payload bytes + `compression_type` byte)

When `compression_type != 0x00`, the stored block payload begins with `uncompressed_len: u32 LE` followed immediately by the compressed block bytes.

### 4.3 Data Block Layout (Target 4,096 Bytes Uncompressed)

Keys inside a Data Block use **Prefix Compression** with a restart point every `16` entries (`RESTART_INTERVAL = 16`) to enable binary search within blocks:

1. **Prefix-Compressed Entries:**
   ```text
   +--------------------+----------------------+------------------+----------------+-------------+
   | shared_prefix_len  | unshared_key_len     | val_len          | unshared_key   | value       |
   | (u16 LE)           | (u16 LE)             | (u32 LE)         | ([u8; ...])    | ([u8; ...]) |
   +--------------------+----------------------+------------------+----------------+-------------+
   ```
   _(At every restart point `i % 16 == 0`, `shared_prefix_len` is `0`, storing the full internal key.)_
2. **Restart Array:**
   - `restart_offsets: [u32 LE; num_restarts]` (Byte offsets within the uncompressed block of each restart entry)
   - `num_restarts: u32 LE` (4 bytes)
3. **5-Byte Block Trailer**

### 4.4 Filter Block Layout (Prefix Bloom Filter)

- **Header (8 Bytes):** `num_probes: u8` (`7`), `bits_per_key: u8` (`10`), `reserved: u16 LE`, `bitset_len: u32 LE`.
- **Bitset (`[u8; bitset_len]`):** Double-hashed (`h1 + i * h2`) Bloom filter populated with both the full `user_key` and configured scope prefix lengths, eliminating `>98%` of negative point lookups before touching Data Blocks.
- **5-Byte Block Trailer**

### 4.5 Meta Block Layout (69 Bytes + 5-Byte Trailer)

| Offset | Size | Type     | Field               | Description                         |
| :----- | :--- | :------- | :------------------ | :---------------------------------- |
| `0`    | `4`  | `u32 LE` | `scope_id`          | Owning Scope ID                     |
| `4`    | `1`  | `u8`     | `level`             | LSM tree level (`0..=6`)            |
| `5`    | `8`  | `u64 LE` | `entry_count`       | Total entries in SSTable            |
| `13`   | `8`  | `u64 LE` | `tombstone_count`   | Total delete/range tombstones       |
| `21`   | `8`  | `u64 LE` | `min_seq`           | Minimum `SequenceNumber` in file    |
| `29`   | `8`  | `u64 LE` | `max_seq`           | Maximum `SequenceNumber` in file    |
| `37`   | `8`  | `u64 LE` | `min_expires_at_ms` | Earliest non-zero TTL (`0` if none) |
| `45`   | `8`  | `u64 LE` | `max_expires_at_ms` | Latest non-zero TTL (`0` if none)   |
| `53`   | `8`  | `u64 LE` | `raw_key_bytes`     | Uncompressed key bytes              |
| `61`   | `8`  | `u64 LE` | `raw_val_bytes`     | Uncompressed value bytes            |

### 4.6 Index Block Layout

Contains one entry per Data Block, sorted by the Data Block's upper-bound internal key:

- `last_key_len: u16 LE`
- `last_internal_key: [u8; last_key_len]` (Greatest internal key present in that Data Block)
- `block_handle: BlockHandle` (`16` bytes: `offset` + `size`)
- Followed by **5-Byte Block Trailer**.

_(Note: A version chain for the same `user_key` may span across adjacent Data Blocks. Index lookups must compare full `InternalKey`s including `inverted_tag`.)_

### 4.7 Fixed Footer (56 Bytes at `EOF - 56`)

| Offset (from Footer start) | Size | Type          | Field           | Description                                       |
| :------------------------- | :--- | :------------ | :-------------- | :------------------------------------------------ |
| `0`                        | `16` | `BlockHandle` | `meta_handle`   | Offset and size of Meta Block                     |
| `16`                       | `16` | `BlockHandle` | `filter_handle` | Offset and size of Filter Block                   |
| `32`                       | `16` | `BlockHandle` | `index_handle`  | Offset and size of Index Block                    |
| `48`                       | `8`  | `[u8; 8]`     | `magic`         | ASCII `b"STORKSST"` (`0x53 54 4F 52 4B 53 53 54`) |

---

## 5. Manifest (`MANIFEST-<num>`) Record Format

### 5.1 Manifest Header (16 Bytes)

- `magic`: `[u8; 8]` = `b"STORKMNF"` (`0x53 54 4F 52 4B 4D 4E 46`)
- `version`: `u16 LE` = `1`
- `reserved`: `u16 LE` = `0`
- `created_at_sec`: `u32 LE`

### 5.2 VersionEdit Frame

Each atomic metadata commit appends one `VersionEdit` frame:

- `crc32c: u32 LE` (Covers `payload_len` + `edit_count` + `edits`)
- `payload_len: u32 LE`
- `edit_count: u16 LE`
- `edits: [EditRecord; edit_count]`

### 5.3 VersionEdit Tags (`EditRecord`)

Each `EditRecord` begins with a 1-byte `tag`:

| Tag    | Variant           | Binary Payload Fields                                                                                                                                                                                                       |
| :----- | :---------------- | :-------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0x01` | `SetNextSequence` | `next_seq: u64 LE`                                                                                                                                                                                                          |
| `0x02` | `SetWalNumber`    | `min_unflushed_wal_num: u64 LE`                                                                                                                                                                                             |
| `0x03` | `CreateScope`     | `scope_id: u32 LE`, `durability: u8`, `sync_ms: u32 LE`, `retention: u8`, `default_ttl_ms: u64 LE`, `compaction_style: u8`, `name_len: u16 LE`, `name: [u8; name_len]`                                                      |
| `0x04` | `DropScope`       | `scope_id: u32 LE`, `drop_seq: u64 LE`                                                                                                                                                                                      |
| `0x05` | `AddSstFile`      | `scope_id: u32 LE`, `level: u8`, `file_num: u64 LE`, `file_size: u64 LE`, `min_seq: u64 LE`, `max_seq: u64 LE`, `max_expires_at_ms: u64 LE`, `min_key_len: u16 LE`, `min_key: [u8]`, `max_key_len: u16 LE`, `max_key: [u8]` |
| `0x06` | `DeleteSstFile`   | `scope_id: u32 LE`, `level: u8`, `file_num: u64 LE`                                                                                                                                                                         |
| `0x07` | `PinSnapshot`     | `seq: u64 LE`, `created_at_ms: u64 LE`, `name_len: u16 LE`, `name: [u8; name_len]`                                                                                                                                          |
| `0x08` | `UnpinSnapshot`   | `name_len: u16 LE`, `name: [u8; name_len]`                                                                                                                                                                                  |
