import os
import sys
import tempfile
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import storkv  # noqa: E402


class EmbeddedTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.directory = tempfile.TemporaryDirectory(prefix="storkv-python-test-")
        cls.uri = "file://" + cls.directory.name

    @classmethod
    def tearDownClass(cls) -> None:
        cls.directory.cleanup()

    def test_crud_and_memory(self) -> None:
        with storkv.Pool(self.uri) as pool:
            self.assertFalse(pool.is_closed())
            with pool.scope("default") as scope:
                scope.put("py:greeting", "hello")
                self.assertEqual(scope.get(b"py:greeting"), b"hello")
                self.assertEqual(scope.get_str("py:greeting"), "hello")
                self.assertTrue(scope.exists("py:greeting"))
                scope.delete("py:greeting")
                self.assertFalse(scope.exists("py:greeting"))
                self.assertIsNone(scope.get("py:greeting"))

    def test_ttl_and_touch(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            scope.put_with_ttl("py:session", "live", 30000)
            self.assertEqual(scope.get_and_touch("py:session", 5000), b"live")
            scope.put_with_ttl("short", "lived", 100)
            time.sleep(0.3)
            self.assertIsNone(scope.get("short"))

    def test_scan_and_transaction(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            scope.put("py:scan:1", "one")
            scope.put("py:scan:2", "two")
            self.assertEqual(len(scope.scan_prefix("py:scan:")), 2)
            self.assertEqual(len(scope.scan_range("py:scan:1", "py:scan:3")), 2)

            with pool.transaction() as tx:
                tx.put(scope, "py:tx", "value")
                self.assertEqual(tx.get(scope, "py:tx"), b"value")
                tx.commit()
            self.assertEqual(scope.get("py:tx"), b"value")

            tx = pool.transaction()
            tx.put(scope, "py:rolled", "back")
            tx.rollback()
            self.assertIsNone(scope.get("py:rolled"))

    def test_watch(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            with scope.watch_prefix("py:events:") as watch:
                scope.put("py:events:1", "payload")
                event = watch.next(5000)
                self.assertIsNotNone(event)
                assert event is not None
                self.assertEqual(event.kind, storkv.EventKind.PUT)
                self.assertEqual(event.key, b"py:events:1")
                self.assertEqual(event.value, b"payload")

    def test_error_mapping(self) -> None:
        with storkv.Pool(self.uri) as pool:
            with pool.scope("default") as scope:
                scope.close()
                with self.assertRaises(storkv.StorkvError) as raised:
                    scope.get("key")
                self.assertEqual(raised.exception.status, storkv.Status.SCOPE)


class NetworkedTests(unittest.TestCase):
    uri = os.environ.get("STORKV_TEST_URI", "")

    @unittest.skipUnless(uri, "STORKV_TEST_URI is unset")
    def test_networked_crud(self) -> None:
        with storkv.Pool(self.uri) as pool, pool.scope("default") as scope:
            scope.put("py:net", "value")
            self.assertEqual(scope.get("py:net"), b"value")
            with pool.transaction() as tx:
                tx.put(scope, "py:net:tx", "committed")
                tx.commit()
            self.assertEqual(scope.get("py:net:tx"), b"committed")


if __name__ == "__main__":
    unittest.main(verbosity=2)
