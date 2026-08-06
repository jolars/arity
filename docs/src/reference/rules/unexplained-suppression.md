# `unexplained-suppression`

Flags a `# arity-ignore` directive that carries no reason — the text after the `:`. A suppression is a standing claim that the linter is wrong at this spot, and without a reason the next reader cannot tell a considered exception from noise someone silenced under deadline, so it becomes permanent by default. Disabled by default, since requiring reasons is a house style rather than a defect; enable it with `select`. Report-only: writing the reason is the fix, and inventing one would fabricate a justification.

This rule is **disabled by default**; enable it with `select`.

The directive says what to silence, but not why:

```r
# arity-ignore unused-binding
x <- 1
```

```text
warning: unexplained-suppression
 --> example.R:1:1
  |
1 | # arity-ignore unused-binding
  | ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ this suppression gives no reason
  = help: add one after the rule: `# arity-ignore <rule>: <reason>`
```
