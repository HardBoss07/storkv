import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test, { after } from "node:test";

import { EventKind, Pool, StorkvError, Status } from "../dist/index.js";

const embeddedDirectory = mkdtempSync(join(tmpdir(), "storkv-ts-test-"));
const embeddedUri = `file://${embeddedDirectory}`;

after(() => {
  rmSync(embeddedDirectory, { recursive: true, force: true });
});

function exercise(uri: string, label: string): void {
  test(`${label} CRUD and TTL`, () => {
    using pool = Pool.connect(uri);
    using scope = pool.scope("default");

    scope.put("ts:greeting", "hello");
    assert.equal(scope.getString("ts:greeting"), "hello");
    assert.deepEqual(Buffer.from(scope.get(Buffer.from("ts:greeting"))!), Buffer.from("hello"));
    assert.equal(scope.exists("ts:greeting"), true);
    scope.delete("ts:greeting");
    assert.equal(scope.exists("ts:greeting"), false);
    assert.equal(scope.get("ts:greeting"), null);

    scope.putWithTtl("ts:session", "live", 30000);
    assert.deepEqual(Buffer.from(scope.getAndTouch("ts:session", 5000)!), Buffer.from("live"));
  });

  test(`${label} scan and transactions`, () => {
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

    {
      using tx = pool.transaction();
      tx.put(scope, "ts:rolled", "back");
      tx.rollback();
    }
    assert.equal(scope.get("ts:rolled"), null);
  });

  test(`${label} watch delivery`, () => {
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

  test(`${label} error mapping`, () => {
    using pool = Pool.connect(uri);
    const scope = pool.scope("default");
    scope.close();
    assert.throws(
      () => scope.get("key"),
      (error: unknown) =>
        error instanceof StorkvError && error.status === Status.InvalidArgument,
    );
  });
}

exercise(embeddedUri, "embedded");

const networkedUri = process.env.STORKV_TEST_URI;
if (networkedUri) {
  exercise(networkedUri, "networked");
} else {
  test("networked tests skipped", { skip: "STORKV_TEST_URI is unset" }, () => {});
}

