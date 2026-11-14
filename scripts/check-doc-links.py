#!/usr/bin/env python3
"""Checks the links of the documentation: README.md and the mdBook in docs/.

Every relative link must name a file that exists, every `#fragment` a heading of the page it
points to, a page of the book must stay inside docs/ (mdBook publishes nothing else), and every
page in docs/ must be listed in docs/SUMMARY.md (mdBook leaves out the others). Links to other
sites are not fetched. Exits 1 with one line per problem.

    python3 scripts/check-doc-links.py [repository root]
"""

import re
import sys
from pathlib import Path

LINK = re.compile(r"\[[^\]]*\]\(\s*<?([^)\s>]+)>?[^)]*\)")
HEADING = re.compile(r"^#{1,6}\s+(.*?)\s*#*\s*$")
FENCE = re.compile(r"^\s*(```|~~~)")
INLINE_CODE = re.compile(r"`[^`]*`")


def prose_lines(text):
    """The lines outside fenced code blocks, with their numbers."""
    fenced = False
    for number, line in enumerate(text.splitlines(), start=1):
        if FENCE.match(line):
            fenced = not fenced
        elif not fenced:
            yield number, line


def slug(heading):
    """The id mdBook and GitHub give a heading: lower case, spaces to hyphens, punctuation gone."""
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", heading).replace("`", "")
    kept = (c for c in text.lower() if c.isalnum() or c in "_- ")
    return "".join(kept).replace(" ", "-")


def anchors(path):
    return {slug(m.group(1)) for _, line in prose_lines(path.read_text()) if (m := HEADING.match(line))}


def links(path):
    for number, line in prose_lines(path.read_text()):
        for match in LINK.finditer(INLINE_CODE.sub("", line)):
            yield number, match.group(1)


def check(root):
    """The problems found under `root`, each a line of text; empty when the links hold."""
    root = root.resolve()
    docs = root / "docs"
    summary = docs / "SUMMARY.md"
    readme = root / "README.md"
    for needed in (readme, summary):
        if not needed.is_file():
            return [f"{needed.relative_to(root)}: missing"]

    problems = []
    pages = sorted(docs.rglob("*.md"))
    for page in [readme, *pages]:
        name = page.relative_to(root)
        for number, target in links(page):
            if re.match(r"[a-z][a-z0-9+.-]*:", target):
                continue  # another site, or mailto:
            file, _, fragment = target.partition("#")
            linked = (page.parent / file).resolve() if file else page
            if not linked.exists():
                problems.append(f"{name}:{number}: {target}: no such file")
            elif page != readme and docs not in linked.parents:
                problems.append(f"{name}:{number}: {target}: outside docs/, so not in the book")
            elif fragment and (linked.suffix != ".md" or fragment not in anchors(linked)):
                problems.append(f"{name}:{number}: {target}: no heading #{fragment}")

    listed = {(docs / target.partition("#")[0]).resolve() for _, target in links(summary)}
    for page in pages:
        if page != summary and page not in listed:
            problems.append(f"{page.relative_to(root)}: not in docs/SUMMARY.md, so not in the book")
    return problems


def main(arguments):
    if len(arguments) > 1:
        print(__doc__, file=sys.stderr)
        return 2
    root = Path(arguments[0]) if arguments else Path(__file__).resolve().parent.parent
    problems = check(root)
    for problem in problems:
        print(problem, file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
