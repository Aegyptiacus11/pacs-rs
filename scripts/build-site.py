#!/usr/bin/env python3
"""Builds the site that GitHub Pages serves, into target/site.

    /                 the homepage: site/ as it is
    /docs/            the book: mdBook's build of docs/
    /<page>.html      a redirect to /docs/<page>.html, for every page of the book: the
                      addresses the book had when it was the whole site
    /sample/          the sample study, at the address the quickstart downloads it from

Then checks the homepage: every relative link must name a file of the site, and every
`#fragment` an id of the page it points to. Exits 1 with one line per problem.

    python3 scripts/build-site.py [repository root]

Needs mdbook on the PATH.
"""

import html
import re
import shutil
import subprocess
import sys
from pathlib import Path

LINK = re.compile(r"""(?:href|src)\s*=\s*["']([^"']+)["']""")
ID = re.compile(r"""\bid\s*=\s*["']([^"']+)["']""")
# files of the book that are not pages of their own, or whose root address the homepage has
NOT_REDIRECTED = {"index.html", "404.html", "toc.html"}


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


def assemble(root, book, site):
    """Puts the homepage, the book, its redirects and the sample study together in `site`."""
    home = root / "site"
    if not (home / "index.html").is_file():
        raise SystemExit("site/index.html: missing")
    if not (book / "index.html").is_file():
        raise SystemExit(f"{book}: no book there; mdbook build first")
    if site.exists():
        shutil.rmtree(site)
    shutil.copytree(home, site)
    shutil.copytree(book, site / "docs")
    redirected = []
    for page in sorted(book.glob("*.html")):
        if page.name in NOT_REDIRECTED:
            continue
        if (site / page.name).exists():
            raise SystemExit(f"{page.name}: a page of the book and a file of site/ share this name")
        (site / page.name).write_text(redirect(f"docs/{page.name}"))
        redirected.append(page.name)
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
    subprocess.run(["mdbook", "build"], cwd=root, check=True)
    site = root / "target" / "site"
    redirected = assemble(root, root / "target" / "book", site)
    problems = check_links(site / "index.html", site)
    for problem in problems:
        print(problem, file=sys.stderr)
    if problems:
        return 1
    print(f"target/site: the homepage, the book under docs/, {len(redirected)} redirects")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
