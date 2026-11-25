#!/usr/bin/env python3
"""Builds the site that GitHub Pages serves, into target/site.

    /                 the homepage: site/ as it is
    /docs/            the book: the pages of docs/, built by Starlight (docs-site/)
    /<page>.html      a redirect to /docs/<page>.html, for each page the book had when it
                      was the whole site (MOVED): the addresses people may still hold
    /sample/          the sample study, at the address the quickstart downloads it from

Then checks the homepage and every page of the book: every relative link must name a file of
the site, and every `#fragment` an id of the page it points to. Exits 1 with one line per
problem.

    python3 scripts/build-site.py [repository root]

Needs npm on the PATH, and the packages of docs-site/ installed (`npm ci` there).
"""

import html
import re
import shutil
import subprocess
import sys
from pathlib import Path

LINK = re.compile(r"""(?:href|src)\s*=\s*["']([^"']+)["']""")
ID = re.compile(r"""\bid\s*=\s*["']([^"']+)["']""")
# The pages the book had at the root of the site, before it moved under docs/ (November
# 2026). A page written since never had another address, and gets no redirect.
MOVED = [
    "api-reference", "audit", "benchmarks", "building", "de-identification", "dicom-files",
    "dicom-network", "dicomweb", "dimse", "interop", "introduction", "login", "print",
    "quickstart", "releases", "rest-api", "rest-guide", "roadmap", "running", "two-ways",
    "web-interface",
]


def redirect(target):
    """A page that sends its visitor to `target`, a path relative to it, keeping the fragment."""
    escaped = html.escape(target, quote=True)
    return f"""<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>This page has moved</title>
<link rel="canonical" href="{escaped}">
<meta http-equiv="refresh" content="0; url={escaped}">
<script>location.replace({target!r} + location.hash);</script>
</head>
<body>
<p>This page has moved to <a href="{escaped}">{escaped}</a>.</p>
</body>
</html>
"""


def check_links(page, site):
    """The broken links of one page of the site, each a line of text."""
    problems = []
    text = page.read_text()
    for target in LINK.findall(text):
        if re.match(r"[a-z][a-z0-9+.-]*:", target) or target.startswith("//"):
            continue  # another site
        if target.startswith("/"):
            continue  # written by the site's builder from its own list of pages
        path, _, fragment = target.partition("#")
        linked = (page.parent / path).resolve() if path else page
        if linked.is_dir():
            linked = linked / "index.html"
        name = page.relative_to(site)
        if site.resolve() not in linked.parents and linked != site.resolve():
            problems.append(f"{name}: {target}: outside the site")
        elif not linked.is_file():
            problems.append(f"{name}: {target}: no such file")
        elif fragment and fragment not in ID.findall(linked.read_text()):
            problems.append(f"{name}: {target}: no id {fragment!r} on that page")
    return problems


def check_book(book, site):
    """The broken links of the book's pages: what a page of docs/ says of another."""
    pages = sorted(book.glob("*.html"))
    if not pages:
        raise SystemExit(f"{book}: no page")
    return [problem for page in pages for problem in check_links(page, site)]


def assemble(root, book, site):
    """Puts the homepage, the book, its redirects and the sample study together in `site`."""
    home = root / "site"
    if not (home / "index.html").is_file():
        raise SystemExit("site/index.html: missing")
    if not (book / "index.html").is_file():
        raise SystemExit(f"{book}: no book there; `npm run build` in docs-site first")
    if site.exists():
        shutil.rmtree(site)
    shutil.copytree(home, site)
    shutil.copytree(book, site / "docs")
    redirected = []
    for name in MOVED:
        page = f"{name}.html"
        if (site / page).exists():
            raise SystemExit(f"{page}: a page that moved and a file of site/ share this name")
        # a page the book no longer has keeps no redirect: it would lead nowhere
        if (book / page).is_file():
            (site / page).write_text(redirect(f"docs/{page}"))
            redirected.append(page)
    # GitHub Pages serves /404.html for what is not there; the book's has its own frame
    shutil.copyfile(book / "404.html", site / "404.html")
    sample = root / "docs" / "sample"
    if not any(sample.glob("*.dcm")):
        raise SystemExit("docs/sample: no sample study")
    shutil.copytree(sample, site / "sample")
    return redirected


def main(arguments):
    if len(arguments) > 1:
        print(__doc__, file=sys.stderr)
        return 2
    root = (Path(arguments[0]) if arguments else Path(__file__).resolve().parent.parent).resolve()
    builder = root / "docs-site"
    if not (builder / "node_modules").is_dir():
        raise SystemExit("docs-site/node_modules: missing; run `npm ci` in docs-site first")
    subprocess.run(["npm", "run", "build"], cwd=builder, check=True)
    site = root / "target" / "site"
    redirected = assemble(root, root / "target" / "book", site)
    problems = check_links(site / "index.html", site) + check_book(site / "docs", site)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print(f"target/site: the homepage, the book under docs/, {len(redirected)} redirects")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
