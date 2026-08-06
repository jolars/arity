# Lint rules

Each rule's reference page is generated from the rule's own metadata by running
the linter on worked examples. Regenerate with `cargo run --example docgen`.

## Correctness

- [`undefined-symbol`](rules/undefined-symbol.md)
- [`unused-binding`](rules/unused-binding.md)
- [`duplicate-formal`](rules/duplicate-formal.md)
- [`duplicated-arguments`](rules/duplicated-arguments.md)
- [`equals-na`](rules/equals-na.md)
- [`vector-logic`](rules/vector-logic.md)
- [`unreachable-code`](rules/unreachable-code.md)
- [`is-numeric`](rules/is-numeric.md)
- [`if-always-true`](rules/if-always-true.md)
- [`empty-assignment`](rules/empty-assignment.md)
- [`download-file`](rules/download-file.md)

## Suspicious

- [`assignment-in-condition`](rules/assignment-in-condition.md)
- [`implicit-assignment`](rules/implicit-assignment.md)
- [`browser`](rules/browser.md)
- [`shadowed-builtin`](rules/shadowed-builtin.md)
- [`redundant-equals`](rules/redundant-equals.md)
- [`redundant-ifelse`](rules/redundant-ifelse.md)
- [`repeat`](rules/repeat.md)
- [`undesirable-function`](rules/undesirable-function.md)
- [`for-loop-index`](rules/for-loop-index.md)
- [`for-loop-dup-index`](rules/for-loop-dup-index.md)

## Readability

- [`true-false-symbol`](rules/true-false-symbol.md)
- [`comparison-negation`](rules/comparison-negation.md)
- [`outer-negation`](rules/outer-negation.md)
- [`string-boundary`](rules/string-boundary.md)
- [`unnecessary-nesting`](rules/unnecessary-nesting.md)

## Performance

- [`any-is-na`](rules/any-is-na.md)
- [`any-duplicated`](rules/any-duplicated.md)
- [`coalesce`](rules/coalesce.md)
- [`crossprod`](rules/crossprod.md)
- [`lengths`](rules/lengths.md)
- [`nzchar`](rules/nzchar.md)
- [`seq`](rules/seq.md)
- [`class-equals`](rules/class-equals.md)
- [`fixed-regex`](rules/fixed-regex.md)
- [`sort`](rules/sort.md)

## Documentation

- [`roxygen-unknown-tag`](rules/roxygen-unknown-tag.md)
- [`roxygen-title`](rules/roxygen-title.md)
- [`roxygen-return`](rules/roxygen-return.md)
- [`roxygen-param`](rules/roxygen-param.md)
- [`roxygen-examples`](rules/roxygen-examples.md)
