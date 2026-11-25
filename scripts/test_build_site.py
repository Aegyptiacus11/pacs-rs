"""Tests of build-site.py: the site it assembles, and the links it refuses."""

import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("build_site", Path(__file__).parent / "build-site.py")
build_site = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build_site)


def repository(directory, home='<a href="docs/quickstart.html#send">start</a>'):
    """A repository with a homepage, a built book of two pages and a sample study."""
    root = Path(directory)
    (root / "site").mkdir()
    (root / "site" / "index.html").write_text(home)
    (root / "site" / "style.css").write_text("body {}")
    book = root / "target" / "book"
    book.mkdir(parents=True)
    (book / "index.html").write_text("<h1>The book</h1>")
    (book / "quickstart.html").write_text('<h2 id="send">Send it</h2>')
    (book / "settings.html").write_text("a page written after the move")
    (book / "404.html").write_text("not found")
    (book / "toc.html").write_text("contents")
    (book / "book.js").write_text("")
    (root / "docs" / "sample").mkdir(parents=True)
    (root / "docs" / "sample" / "00001.dcm").write_bytes(b"DICM")
    return root, book, root / "target" / "site"


class BuildSite(unittest.TestCase):
    def test_the_site_has_the_homepage_the_book_and_its_old_addresses(self):
        with tempfile.TemporaryDirectory() as directory:
            root, book, site = repository(directory)
            self.assertEqual(build_site.assemble(root, book, site), ["quickstart.html"])
            self.assertIn("start", (site / "index.html").read_text())
            self.assertEqual((site / "docs" / "index.html").read_text(), "<h1>The book</h1>")
            self.assertTrue((site / "docs" / "book.js").is_file())
            self.assertEqual((site / "sample" / "00001.dcm").read_bytes(), b"DICM")
            self.assertEqual((site / "404.html").read_text(), "not found")
            # the old address of a page sends to the new one, with its fragment
            moved = (site / "quickstart.html").read_text()
            self.assertIn('content="0; url=docs/quickstart.html"', moved)
            self.assertIn("location.replace('docs/quickstart.html' + location.hash)", moved)
            # the homepage keeps the root: the book's first page is not redirected over it
            self.assertFalse((site / "toc.html").exists())
            # a page written since the move never had another address
            self.assertFalse((site / "settings.html").exists())
            self.assertTrue((site / "docs" / "settings.html").is_file())
            self.assertEqual(build_site.check_links(site / "index.html", site), [])
            # built again over what is there
            self.assertEqual(build_site.assemble(root, book, site), ["quickstart.html"])

    def test_broken_links_of_the_homepage_are_named(self):
        home = (
            '<a href="docs/missing.html">a</a> <a href="docs/quickstart.html#nowhere">b</a>'
            ' <a href="../outside.html">c</a> <img src="img/none.jpg">'
            ' <a href="https://example.org/x">d</a> <a href="docs/">e</a> <a href="#top">f</a>'
        )
        with tempfile.TemporaryDirectory() as directory:
            root, book, site = repository(directory, home)
            build_site.assemble(root, book, site)
            self.assertEqual(
                build_site.check_links(site / "index.html", site),
                [
                    "index.html: docs/missing.html: no such file",
                    "index.html: docs/quickstart.html#nowhere: no id 'nowhere' on that page",
                    "index.html: ../outside.html: outside the site",
                    "index.html: img/none.jpg: no such file",
                    "index.html: #top: no id 'top' on that page",
                ],
            )

    def test_broken_links_between_pages_of_the_book_are_named(self):
        with tempfile.TemporaryDirectory() as directory:
            root, book, site = repository(directory)
            (book / "settings.html").write_text(
                '<a href="quickstart.html#send">a</a> <a href="quickstart.html#gone">b</a>'
                ' <a href="login.html">c</a> <a href="/pacs-rs/docs/anything.html">d</a>'
            )
            build_site.assemble(root, book, site)
            self.assertEqual(
                build_site.check_book(site / "docs", site),
                [
                    "docs/settings.html: quickstart.html#gone: no id 'gone' on that page",
                    "docs/settings.html: login.html: no such file",
                ],
            )
            # a book with no page at all is not a book that has no broken link
            for page in (site / "docs").glob("*.html"):
                page.unlink()
            with self.assertRaises(SystemExit) as stopped:
                build_site.check_book(site / "docs", site)
            self.assertIn("no page", str(stopped.exception))

    def test_a_site_is_not_built_without_the_builder_s_packages(self):
        with tempfile.TemporaryDirectory() as directory:
            root, _, _ = repository(directory)
            (root / "docs-site").mkdir()
            with self.assertRaises(SystemExit) as stopped:
                build_site.main([str(root)])
            self.assertIn("npm ci", str(stopped.exception))

    def test_what_is_missing_stops_the_build(self):
        with tempfile.TemporaryDirectory() as directory:
            root, book, site = repository(directory)
            (root / "site" / "quickstart.html").write_text("a page of the homepage's")
            with self.assertRaises(SystemExit) as stopped:
                build_site.assemble(root, book, site)
            self.assertIn("share this name", str(stopped.exception))
        for missing, why in [
            ("site/index.html", "site/index.html: missing"),
            ("target/book/index.html", "no book there"),
            ("docs/sample/00001.dcm", "no sample study"),
        ]:
            with tempfile.TemporaryDirectory() as directory:
                root, book, site = repository(directory)
                (root / missing).unlink()
                with self.assertRaises(SystemExit) as stopped:
                    build_site.assemble(root, book, site)
                self.assertIn(why, str(stopped.exception))

    def test_a_redirect_escapes_its_target(self):
        page = build_site.redirect('docs/a"b.html')
        self.assertIn('href="docs/a&quot;b.html"', page)


if __name__ == "__main__":
    unittest.main()
