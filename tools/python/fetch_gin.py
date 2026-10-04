"""Download neo's test data (GIN `NeuralEnsemble/ephy_testing_data`) into data/raw, and check it.

Usage (from the workspace root):
    python3 tools/python/fetch_gin.py --format blackrock     # every blackrock set in testdata.toml
    python3 tools/python/fetch_gin.py --all                  # every GIN set in testdata.toml
    python3 tools/python/fetch_gin.py --check [--format F]   # local files against the digests
    python3 tools/python/fetch_gin.py <path in the repo> [<dest root>]   # any GIN path

`tools/python/testdata.toml` lists the sets the comparisons (tools/python/compare) use: their GIN
path, local folder under data/raw, file count, size and digest. Walks GIN's web listing (its API is
not served for this repo); annexed files come through `/media/master/…`, plain git files through
`/raw/master/…`. Existing files are kept (delete one to fetch it again). Standard library only
(Python ≥ 3.11 for tomllib).
"""

import argparse
import hashlib
import os
import re
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

BASE = "https://gin.g-node.org/NeuralEnsemble/ephy_testing_data"
ROW = re.compile(r'<td class="name">\s*<span class="octicon octicon-([\w-]+)"></span>\s*<a href="([^"]+)"', re.S)
TESTDATA = Path(__file__).with_name("testdata.toml")
RAW = Path("data/raw")


def get(url: str) -> bytes:
    with urllib.request.urlopen(url, timeout=120) as r:
        return r.read()


def walk(path: str):
    """Files under the GIN path (a folder), as repo paths; nothing for a file."""
    html = get(f"{BASE}/src/master/{urllib.parse.quote(path)}").decode("utf-8", "replace")
    for kind, href in ROW.findall(html):
        child = urllib.parse.unquote(href.split("/src/master/", 1)[1])
        if kind == "file-directory":
            yield from walk(child)
        elif kind.startswith("file"):
            yield child


def download(repo_path: str, dest: Path) -> None:
    if dest.exists():
        print(f"  kept {repo_path}")
        return
    dest.parent.mkdir(parents=True, exist_ok=True)
    part = dest.with_name(dest.name + ".part")
    for kind in ("media", "raw"):
        try:
            with urllib.request.urlopen(f"{BASE}/{kind}/master/{urllib.parse.quote(repo_path)}", timeout=600) as r, open(part, "wb") as out:
                while chunk := r.read(1 << 20):
                    out.write(chunk)
            # Missing annexed files come back as an HTML page on `media`
            if part.read_bytes()[:15].lstrip().lower().startswith(b"<!doctype html"):
                part.unlink()
                continue
            break
        except urllib.error.HTTPError as e:
            if e.code != 404:
                raise
    if not part.exists():
        raise FileNotFoundError(f"{repo_path}: not found on GIN")
    os.replace(part, dest)
    print(f"  got  {repo_path} ({dest.stat().st_size} B)")


def fetch(gin: str, local: Path) -> None:
    """The GIN folder or file `gin` into `local` (a folder's files keep their relative paths)."""
    files = list(walk(gin))
    if not files:  # a single file
        download(gin, local)
        return
    for f in files:
        download(f, local / Path(f).relative_to(gin))


def digest(p: Path) -> tuple[int, int, str]:
    """(files, bytes, SHA-256 over "<relative path>\\0<file sha256>\\n" lines sorted by path)."""
    files = [p] if p.is_file() else sorted(f for f in p.rglob("*") if f.is_file() and not f.name.endswith(".part"))
    h, total = hashlib.sha256(), 0
    for f in files:
        fh = hashlib.sha256()
        with open(f, "rb") as fd:
            for chunk in iter(lambda: fd.read(1 << 20), b""):
                fh.update(chunk)
        rel = f.relative_to(p).as_posix() if p.is_dir() else f.name
        h.update(f"{rel}\0{fh.hexdigest()}\n".encode())
        total += f.stat().st_size
    return len(files), total, h.hexdigest()


def sets(fmt: str | None) -> list[dict]:
    all_sets = tomllib.loads(TESTDATA.read_text())["set"]
    return [s for s in all_sets if fmt in (None, s["format"])]


def main() -> int:
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("path", nargs="?", help="a GIN path (instead of --format / --all)")
    p.add_argument("dest", nargs="?", default=str(RAW), help="destination root for a GIN path")
    p.add_argument("--format", help="the sets of one format in testdata.toml")
    p.add_argument("--all", action="store_true", help="every GIN set in testdata.toml")
    p.add_argument("--check", action="store_true", help="compare local sets with their digests (no download)")
    a = p.parse_args()

    if a.path and not (a.format or a.all or a.check):
        fetch(a.path.strip("/"), Path(a.dest) / a.path.strip("/"))
        return 0
    if not (a.format or a.all or a.check):
        p.print_help()
        return 2
    bad = 0
    for s in sets(a.format):
        local = RAW / s["local"]
        if a.check:
            if not local.exists():
                print(f"missing   {s['local']}")
                bad += 1
                continue
            if "digest" not in s:
                print(f"present   {s['local']} (no digest)")
                continue
            ok = digest(local) == (s["files"], s["bytes"], s["digest"])
            print(f"{'ok' if ok else 'DIFFERS':9} {s['local']}")
            bad += not ok
        elif s["source"] == "gin":
            print(f"{s['format']}: {s['gin']} → {local}")
            fetch(s["gin"], local)
        else:
            print(f"{s['format']}: {s['local']} is not on GIN ({s.get('origin', '')}); skipped")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
