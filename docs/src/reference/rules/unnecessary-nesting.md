# `unnecessary-nesting`

Flag an `if` whose entire body is a second `if`—the two could be a single `if` with the conditions joined by `&&`, dropping a needless level of nesting. It fires only when neither `if` has an `else` (an `else` on either side changes what runs) and the inner `if` is the sole statement of the outer one.

The fix joins the conditions with `&&`, parenthesizing each non-primary condition so the grouping is preserved. It is unsafe (collapsing dedents the body, so a reformat may follow) and withheld when it would drop a comment.

This rule is **enabled by default**.

An `if` whose only body is another `if` can be a single `if`:

```r
if (a) {
  if (b) {
    do_thing()
  }
}
```

```text
warning: unnecessary-nesting
 --> example.R:2:3
  |
2 |   if (b) {
  |   ^^ this `if` is nested in another `if` with no `else`
  = help: Combine the two conditions with `&&` to drop a level of nesting.
```
