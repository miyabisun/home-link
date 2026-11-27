#!/usr/bin/env python3
"""Tests of bin/browser: where it keeps the browser, and how it fetches it once."""
import hashlib
import importlib.machinery
import importlib.util
import io
import os
import stat
import tempfile
import unittest
import zipfile
from pathlib import Path

LOADER = importlib.machinery.SourceFileLoader("browser", str(Path(__file__).resolve().parent / "browser"))
browser = importlib.util.module_from_spec(importlib.util.spec_from_loader("browser", LOADER))
LOADER.exec_module(browser)


def archive(files):
    """A zip archive of `files` (name: (content, mode)), as bytes."""
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w") as made:
        for name, (content, mode) in files.items():
            info = zipfile.ZipInfo(name)
            info.external_attr = (stat.S_IFREG | mode) << 16
            made.writestr(info, content)
    return out.getvalue()


SHELL = archive({"shell/run": (b"#!/bin/sh\n", 0o755), "shell/notes": (b"notes", 0o644)})
DRIVER = archive({"driver/drive": (b"#!/bin/sh\n", 0o755)})
SERVED = {"shell.zip": SHELL, "driver.zip": DRIVER}
ARCHIVES = {name: hashlib.sha256(data).hexdigest() for name, data in SERVED.items()}
PROGRAMS = ("shell/run", "driver/drive")


class Home(unittest.TestCase):
    def test_the_browser_is_kept_in_the_cache_of_the_user_under_its_version(self):
        self.assertEqual(
            browser.home({"HOME": "/home/one"}),
            Path("/home/one/.cache/chrome-for-testing") / browser.VERSION,
        )

    def test_a_cache_named_by_xdg_is_used(self):
        self.assertEqual(
            browser.home({"HOME": "/home/one", "XDG_CACHE_HOME": "/var/cache/one"}),
            Path("/var/cache/one/chrome-for-testing") / browser.VERSION,
        )

    def test_a_cache_named_empty_or_relative_is_not(self):
        for named in ("", "cache"):
            with self.subTest(named=named):
                self.assertEqual(
                    browser.home({"HOME": "/home/one", "XDG_CACHE_HOME": named}),
                    Path("/home/one/.cache/chrome-for-testing") / browser.VERSION,
                )


class Agent(unittest.TestCase):
    def test_the_browser_is_asked_for_as_a_browser_of_its_version_would_ask(self):
        self.assertEqual(
            browser.agent("158.0.8127.88"),
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/158.0.0.0 Safari/537.36",
        )


class Fetch(unittest.TestCase):
    def setUp(self):
        self.cache = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.place = self.cache / "browser" / "1.2.3"
        self.asked = []

    def get(self, served=SERVED):
        def get(url, path):
            self.asked.append(url)
            path.write_bytes(served[url.rsplit("/", 1)[1]])

        return get

    def fetch(self, get):
        return browser.fetch(self.place, get, "https://example.test/{version}/{name}", ARCHIVES, PROGRAMS)

    def test_the_programs_are_unpacked_with_what_runs_them_and_the_archives_are_not_kept(self):
        self.assertEqual(self.fetch(self.get()), self.place)
        self.assertEqual(sorted(self.asked), ["https://example.test/1.2.3/driver.zip", "https://example.test/1.2.3/shell.zip"])
        for program in PROGRAMS:
            with self.subTest(program=program):
                self.assertTrue(os.access(self.place / program, os.X_OK))
        self.assertFalse(os.access(self.place / "shell/notes", os.X_OK))
        self.assertEqual(sorted(path.name for path in self.place.iterdir()), ["driver", "shell"])
        self.assertEqual(list(self.place.parent.iterdir()), [self.place])

    def test_a_browser_that_is_there_is_not_fetched_again(self):
        self.fetch(self.get())
        self.asked.clear()
        self.assertEqual(self.fetch(self.get()), self.place)
        self.assertEqual(self.asked, [])

    def test_an_archive_that_is_not_the_one_pinned_is_refused_and_nothing_is_left(self):
        with self.assertRaises(SystemExit) as refused:
            self.fetch(self.get({"shell.zip": SHELL, "driver.zip": SHELL}))
        self.assertIn("driver.zip", str(refused.exception.code))
        self.assertEqual(list(self.place.parent.iterdir()), [])

    def test_a_fetch_that_fails_leaves_nothing(self):
        def broken(url, path):
            path.write_bytes(b"half")
            raise OSError("the network went away")

        with self.assertRaises(OSError):
            self.fetch(broken)
        self.assertEqual(list(self.place.parent.iterdir()), [])

    def test_a_browser_another_run_put_there_meanwhile_is_kept(self):
        def racing(url, path):
            self.get()(url, path)
            if not self.place.exists():
                self.place.mkdir()
                (self.place / "theirs").write_text("")

        self.assertEqual(self.fetch(racing), self.place)
        self.assertEqual([path.name for path in self.place.iterdir()], ["theirs"])
        self.assertEqual(list(self.place.parent.iterdir()), [self.place])


if __name__ == "__main__":
    unittest.main()
