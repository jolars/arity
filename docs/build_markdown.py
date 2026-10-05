"""Extract clean Markdown variants from the rendered mdBook content."""

from pathlib import Path
import re
import subprocess
import sys


CONTENT = re.compile(r"<main\b[^>]*>(.*?)</main>", re.DOTALL | re.IGNORECASE)
HEADER_LINK = re.compile(
    r'<a\b[^>]*class="header"[^>]*>(.*?)</a>', re.DOTALL | re.IGNORECASE
)


def build(book: Path) -> int:
    count = 0
    for page in sorted(book.rglob("*.html")):
        relative = page.relative_to(book)
        if relative.as_posix() in {"404.html", "print.html", "toc.html"}:
            continue
        html = page.read_text(encoding="utf-8")
        if 'http-equiv="refresh"' in html:
            continue
        match = CONTENT.search(html)
        if match is None:
            raise ValueError(f"no main content in {page}")
        content = HEADER_LINK.sub(r"\1", match.group(1))
        markdown = subprocess.run(
            ["pandoc", "--from=html", "--to=gfm-raw_html", "--wrap=none"],
            input=content,
            text=True,
            capture_output=True,
            check=True,
        ).stdout
        destination = book / "_markdown" / relative.with_suffix(".md")
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(markdown, encoding="utf-8")
        count += 1
    return count


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: build_markdown.py <book-dir>")
    print(f"built {build(Path(sys.argv[1]))} Markdown pages")
