# `download-file`

Flag a `download.file()` call whose `mode` is not portable.

The default `mode = "w"` is text mode: on Windows it translates line endings, corrupting any binary payload, while the same call works on Unix. R recommends `mode = "wb"` (or `"ab"` to append), so the rule reports an omitted `mode`, an explicit `mode = "w"` / `"a"`, and a `mode` supplied next to `method = "curl"` / `"wget"` (which shell out and ignore it).

Arguments are matched the way R matches them, so a positional or partially-named `method`/`mode` is understood. The callee must resolve to base R, and a `mode`/`method` that is not a string literal is skipped rather than guessed at. There is no autofix: the shapes need an argument inserted or deleted, not rewritten.

This rule is **enabled by default**.

Relying on the default `mode = "w"`, which corrupts a binary download on Windows:

```r
download.file(url, destfile)
```

```text
warning: download-file
 --> example.R:1:1
  |
1 | download.file(url, destfile)
  | ^^^^^^^^^^^^^ `download.file()` relies on the default `mode = "w"`, which corrupts binary downloads on Windows
  = help: Pass `mode = "wb"` (or `mode = "ab"` to append).
```

`mode` is ignored by `method = "curl"` and `method = "wget"`:

```r
download.file(url, destfile, method = "curl", mode = "wb")
```

```text
warning: download-file
 --> example.R:1:47
  |
1 | download.file(url, destfile, method = "curl", mode = "wb")
  |                                               ^^^^^^^^^^^ `mode` is ignored by `download.file(method = "curl")`
  = help: Drop the `mode` argument, or use a `method` that honors it (`"curl"` shells out to an external downloader).
```
