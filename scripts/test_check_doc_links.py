"""Tests of check-doc-links.py: python3 -m unittest discover -s scripts -p 'test_*.py'"""

import importlib.util
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location(
    "check_doc_links", Path(__file__).with_name("check-doc-links.py")
)
check_doc_links = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check_doc_links)


class CheckDocLinks(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.root = Path(directory.name)
        (self.root / "docs").mkdir()
        self.write("README.md", "# Project\n\n[Running](docs/running.md), [docs](docs)\n")
        self.write("docs/SUMMARY.md", "# Summary\n\n- [Running](running.md)\n")
        self.write("docs/running.md", "# Running\n\n## The `PACS_*` settings, all\n")

    def write(self, name, text):
        (self.root / name).write_text(text)

    def problems(self):
        return check_doc_links.check(self.root)

    def test_good_links_pass(self):
        self.write("docs/SUMMARY.md", "- [Running](running.md)\n- [Other](other.md)\n")
        self.write(
            "docs/other.md",
            "[settings](running.md#the-pacs_-settings-all), [here](#other), <https://example.org>,\n"
            "[site](https://example.org/x.md#nothing)\n\n# Other\n",
        )
        self.assertEqual(self.problems(), [])
        self.assertEqual(check_doc_links.main([str(self.root)]), 0)

    def test_missing_file(self):
        self.write("README.md", "[Gone](docs/gone.md)\n")
        self.assertEqual(self.problems(), ["README.md:1: docs/gone.md: no such file"])
        self.assertEqual(check_doc_links.main([str(self.root)]), 1)

    def test_missing_heading(self):
        self.write("README.md", "\n[Running](docs/running.md#nowhere)\n")
        self.assertEqual(self.problems(), ["README.md:2: docs/running.md#nowhere: no heading #nowhere"])

    def test_page_linking_outside_the_book(self):
        self.write("docs/running.md", "[readme](../README.md)\n")
        self.assertEqual(
            self.problems(), ["docs/running.md:1: ../README.md: outside docs/, so not in the book"]
        )

    def test_page_missing_from_the_summary(self):
        self.write("docs/orphan.md", "# Orphan\n")
        self.assertEqual(self.problems(), ["docs/orphan.md: not in docs/SUMMARY.md, so not in the book"])

    def test_links_and_headings_in_code_are_not_read(self):
        self.write("docs/running.md", "```\n[x](gone.md)\n# Not a heading\n```\n`[y](gone.md)`\n")
        self.write("README.md", "[Running](docs/running.md#not-a-heading)\n")
        self.assertEqual(
            self.problems(), ["README.md:1: docs/running.md#not-a-heading: no heading #not-a-heading"]
        )

    def test_no_summary(self):
        (self.root / "docs/SUMMARY.md").unlink()
        self.assertEqual(self.problems(), ["docs/SUMMARY.md: missing"])

    def test_too_many_arguments(self):
        self.assertEqual(check_doc_links.main(["a", "b"]), 2)


if __name__ == "__main__":
    unittest.main()
