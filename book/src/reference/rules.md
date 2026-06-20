# Lint rules

Each rule's reference page is generated from the rule's own metadata by running
the linter on worked examples. Regenerate with `cargo run --example docgen`.

## Correctness

- [`undefined-symbol`](rules/undefined-symbol.md)
- [`unused-binding`](rules/unused-binding.md)
- [`duplicate-formal`](rules/duplicate-formal.md)
- [`duplicated-arguments`](rules/duplicated-arguments.md)
- [`equals-na`](rules/equals-na.md)

## Suspicious

- [`assignment-in-condition`](rules/assignment-in-condition.md)
- [`shadowed-builtin`](rules/shadowed-builtin.md)
- [`redundant-equals`](rules/redundant-equals.md)
- [`redundant-ifelse`](rules/redundant-ifelse.md)

## Readability

- [`true-false-symbol`](rules/true-false-symbol.md)
