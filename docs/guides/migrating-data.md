# Migrating data

`storkvctl migrate` moves keys between StorKV instances, between scopes, from Redis, or between a live server and an offline data directory. It uses a JSONL record format for file transfers.

## Record format

Each line of a JSONL file is one JSON object:

```json
{ "key_b64": "dXNlcjo0Mg==", "value_b64": "YWN0aXZl", "expires_at_ms": 0 }
```

| Field           | Required | Meaning                                                 |
| :-------------- | :------- | :------------------------------------------------------ |
| `key_b64`       | Yes      | Standard base64 of the key bytes                        |
| `value_b64`     | Yes      | Standard base64 of the value bytes                      |
| `expires_at_ms` | No       | Absolute Unix expiry in ms; 0 or absent means no expiry |

Empty lines are skipped. A malformed line stops the import and reports the file and line number. A bad base64 payload also stops the import.

## Export a scope

Export from a running server:

```bash
./target/release/storkvctl migrate \
  --sock 127.0.0.1:7878 \
  --scope default \
  --export-jsonl ./export/default.jsonl
```

The command prints the key count:

```text
Exported 1204 keys from scope default to ./export/default.jsonl
```

Export reads every key in the scope. It also reads the visible metadata of each key to record the absolute expiry.

> [!NOTE]
> Export needs `--sock`. It cannot read an offline directory.

## Import records

Import into a running server:

```bash
./target/release/storkvctl migrate \
  --sock 127.0.0.1:7878 \
  --import-jsonl ./export/default.jsonl \
  --into-scope restored
```

Import into an offline directory:

```bash
./target/release/storkvctl migrate \
  --data-dir ./data/storkv \
  --import-jsonl ./export/default.jsonl \
  --into-scope restored
```

The command prints the imported count:

```text
Imported 1204 keys from ./export/default.jsonl into scope restored
```

When `--into-scope` is absent, the import uses the `--scope` value.

### Online and offline differences

| Mode    | Target       | Atomicity                          | Server state    |
| :------ | :----------- | :--------------------------------- | :-------------- |
| Online  | `--sock`     | One write for each record          | Runs normally   |
| Offline | `--data-dir` | One transaction for the whole file | Must be stopped |

An online import writes each record with a separate `put` or `put_with_ttl`. A failure part way leaves the earlier records in place. Run the import again or delete the target scope to recover.

An offline import opens the database, buffers every record in one transaction, and commits once. It needs exclusive access to the data directory. Do not run it while `storkvd` owns the directory.

### Scope creation

The import creates the destination scope when it does not exist. The new scope uses strict durability, permanent retention, and an 8-byte prefix Bloom filter.

The `default` scope always exists with ID 0.

## Copy from Redis

```bash
./target/release/storkvctl migrate \
  --from-redis redis://127.0.0.1:6379 \
  --sock 127.0.0.1:7878 \
  --into-scope imported
```

The command prints:

```text
Imported 900 keys from redis://127.0.0.1:6379 into scope imported
```

The importer uses `SELECT 0`, `SCAN`, `GET`, and `PTTL` on the source server. It first copies the source into records in memory, then writes them to the target.

### Redis source rules

| Item            | Rule                                         |
| :-------------- | :------------------------------------------- |
| URL form        | `redis://host:port` or `host:port`           |
| Default port    | 6379 when the address has no port            |
| Path            | Ignored after the host and port              |
| Authentication  | Not supported                                |
| TLS             | Not supported                                |
| Source database | Always database 0                            |
| Value type      | String values only                           |
| TTL             | `PTTL` above zero becomes an absolute expiry |

A key with a non-string type returns an error from `GET` and stops the import. Remove those keys from the source or copy them with a separate tool.

The importer buffers all records in memory before it writes them. Check the free memory before you copy a large database.

## Move data between scopes

Use one export and one import:

```bash
./target/release/storkvctl migrate --sock 127.0.0.1:7878 \
  --scope old_scope --export-jsonl ./old_scope.jsonl

./target/release/storkvctl migrate --sock 127.0.0.1:7878 \
  --import-jsonl ./old_scope.jsonl --into-scope new_scope
```

The import adds or replaces keys. It does not delete keys that exist only in the destination.

## Move data between servers

These steps give a consistent copy:

1. Stop writes to the source.
2. Export the scope from the source.
3. Import the file into the target.
4. Compare the reported key count with the source count.
5. Point clients at the target.

For a whole-database copy, use a checkpoint instead of JSONL. Read [Backup and restore](../operations/backup-and-restore.md).

## Verify a migration

Check the destination key count with a scan:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/imported/scan?prefix=" | head -c 200
```

Compare selected keys byte for byte. Use `format=json` on a key read to get base64 values that are safe for binary data:

```bash
curl -sS "http://127.0.0.1:8080/v1/scopes/imported/keys/greeting?format=json"
```

## Limits

`storkvctl migrate` has these constraints:

- A key must be at most 65,535 bytes.
- A value must be at most 256 MiB.
- The whole JSONL file is read into memory.
- The importer has no resume marker.
- The importer has no progress display.

Split a very large export into several files and import them in sequence.

## Related guides

- [CLI reference](../reference/cli-storkvctl.md)
- [Backup and restore](../operations/backup-and-restore.md)
- [Redis compatibility](compatibility-redis.md)
- [Scopes](../concepts/scopes.md)
