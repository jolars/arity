# `misnamed-suppression`

Flags a `# arity-ignore` directive whose rule ID is not a rule arity ships. Such a directive suppresses nothing, and does so silently — the failure mode of a suppression is that no output appears, which is also what success looks like. When exactly one shipped rule ID is an unambiguous near-match, the fix rewrites the ID and leaves the reason text alone; otherwise the finding is report-only. Note that `syntax-error` is not a lint rule: parse errors are reported before any rule runs and cannot be suppressed.

This rule is **enabled by default**.

The rule ID is misspelled, so the directive suppresses nothing:

```r
# arity-ignore unusd-binding: leftover from a refactor
x <- 1
```

```text
warning: misnamed-suppression
 --> example.R:1:16
  |
1 | # arity-ignore unusd-binding: leftover from a refactor
  |                ^^^^^^^^^^^^^ `unusd-binding` is not an arity lint rule, so this directive suppresses nothing
  = help: did you mean `unused-binding`?
```

After applying the fix:

```r
# arity-ignore unused-binding: leftover from a refactor
x <- 1
```

A comma-separated list is not supported — write one directive per rule:

```r
# arity-ignore browser, repeat: debugging
x <- 1
```

```text
warning: misnamed-suppression
 --> example.R:1:16
  |
1 | # arity-ignore browser, repeat: debugging
  |                ^^^^^^^^ `browser,` is not an arity lint rule, so this directive suppresses nothing
  = help: a directive names one rule; write a separate `# arity-ignore` per rule
```
