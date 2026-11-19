#!/usr/bin/env python3
"""Every page is reachable from the root, and every link points at something.

Two failures this catches, both of which ship silently otherwise:

  orphan   a file was published that nothing links to, so nobody can find it
  broken   a link points at a file that was never published

Walks the built site the way a visitor would: start at index.html, follow every
local href and src, and keep going. Then compare what was reached against what
is actually there.

Usage: check-site-links.py <site directory>
"""
import os
import re
import sys
from html.parser import HTMLParser

LINK = re.compile(r'(?:href|src)="([^"]+)"')
# Files a visitor never navigates to and nothing is expected to link.
IGNORED = {"robots.txt", "favicon.ico", ".nojekyll"}


class Links(HTMLParser):
    def __init__(self):
        super().__init__()
        self.found = []

    def handle_starttag(self, tag, attrs):
        for name, value in attrs:
            if name in ("href", "src") and value:
                self.found.append(value)


def to_path(root, page, link):
    """Where a link points, as a file on disk. None when it leaves the site."""
    if re.match(r"^[a-zA-Z][a-zA-Z0-9+.-]*:", link) or link.startswith("//"):
        return None  # another site, or mailto:, or data:
    link = link.split("#")[0].split("?")[0]
    if not link:
        return None
    if link.startswith("/"):
        target = os.path.normpath(os.path.join(root, link.lstrip("/")))
    else:
        target = os.path.normpath(os.path.join(os.path.dirname(page), link))
    # A request for a directory is served the index.html inside it, which is what
    # the edge function does in front of S3.
    if link.endswith("/") or os.path.isdir(target):
        target = os.path.join(target, "index.html")
    return target


def walk(root):
    start = os.path.normpath(os.path.join(root, "index.html"))
    if not os.path.isfile(start):
        return set(), [("", "/", "there is no index.html at the root")]

    reached, broken, pending = {start}, [], [start]
    while pending:
        page = pending.pop()
        if not page.endswith(".html"):
            continue
        parser = Links()
        with open(page, encoding="utf-8") as handle:
            parser.feed(handle.read())
        for link in parser.found:
            target = to_path(root, page, link)
            if target is None:
                continue
            shown = os.path.relpath(page, root).replace(os.sep, "/")
            if not os.path.isfile(target):
                broken.append((shown, link, "points at nothing"))
            elif target not in reached:
                reached.add(target)
                pending.append(target)
    return reached, broken


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    # Normalised once, because the reached set is built with normpath and a mismatch
    # between the two separators Windows accepts would report every file an orphan.
    root = os.path.normpath(sys.argv[1])
    reached, broken = walk(root)

    published = set()
    for here, _, files in os.walk(root):
        for name in files:
            if name not in IGNORED:
                published.add(os.path.normpath(os.path.join(here, name)))

    orphans = sorted(published - reached)
    for page, link, why in broken:
        print(f"broken: {page} -> {link} ({why})")
    for orphan in orphans:
        print(f"orphan: {os.path.relpath(orphan, root)} is published but nothing links to it")

    if broken or orphans:
        print(f"\n{len(broken)} broken, {len(orphans)} orphaned")
        return 1
    print(f"{len(reached)} files, all reachable from the root, no broken links")
    return 0


if __name__ == "__main__":
    sys.exit(main())
