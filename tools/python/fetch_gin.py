"""Download a folder of neo's test data (GIN `NeuralEnsemble/ephy_testing_data`) into data/raw.

Usage:
    python3 tools/python/fetch_gin.py <path in the repo> [<dest root>]
    python3 tools/python/fetch_gin.py spikeglx/onebox            # → data/raw/spikeglx/onebox

Walks the web listing (GIN's API is not served for this repo) and fetches annexed files
through `/media/master/…`, plain git files through `/raw/master/…`. Existing files are kept
(delete one to fetch it again).
Standard library only.
"""

import os
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

BASE = "https://gin.g-node.org/NeuralEnsemble/ephy_testing_data"
ROW = re.compile(r'<td class="name">\s*<span class="octicon octicon-([\w-]+)"></span>\s*<a href="([^"]+)"', re.S)


def get(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def walk(path: str):
    html = get(f"{BASE}/src/master/{path}").decode("utf-8", "replace")
    for kind, href in ROW.findall(html):
        child = href.split("/src/master/", 1)[1]
        if kind == "file-directory":
            yield from walk(child)
        elif kind.startswith("file"):
            yield child


def fetch(path: str, dest_root: str) -> None:
    for f in walk(path):
        # Listing links are URL-encoded (`%28` for `(`): decode for the file name only
        dest = os.path.join(dest_root, urllib.parse.unquote(f))
        if os.path.exists(dest):
            print(f"  kept {f}")
            continue
        os.makedirs(os.path.dirname(dest), exist_ok=True)
        for kind in ("media", "raw"):
            try:
                with urllib.request.urlopen(f"{BASE}/{kind}/master/{f}", timeout=600) as r, open(dest + ".part", "wb") as out:
                    while chunk := r.read(1 << 20):
                        out.write(chunk)
                break
            except urllib.error.HTTPError as e:
                if e.code != 404 or kind == "raw":
                    raise
        os.replace(dest + ".part", dest)
        print(f"  got  {f} ({os.path.getsize(dest)} B)")


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    fetch(sys.argv[1].strip("/"), sys.argv[2] if len(sys.argv) > 2 else "data/raw")
