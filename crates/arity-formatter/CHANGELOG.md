# Changelog

## [0.3.1](https://github.com/jolars/arity/compare/arity-formatter-v0.3.0...arity-formatter-v0.3.1) (2026-08-11)

### Bug Fixes
- **formatter:** keep blank line after a comment ([`e50f7fe`](https://github.com/jolars/arity/commit/e50f7fef855b33152f9f0f948f9efe14072185ae))

### Dependencies
- updated crates/arity-parser to v0.4.0

## [0.3.0](https://github.com/jolars/arity/compare/arity-formatter-v0.2.0...arity-formatter-v0.3.0) (2026-08-07)

### Features
- **formatter:** classify `@prop` and `@R6method` tags ([`a0e81d0`](https://github.com/jolars/arity/commit/a0e81d09890d0935c36d219c640e6cab679f411e))

### Bug Fixes
- **formatter:** keep marker-less remainder after a block macro ([`40a89cb`](https://github.com/jolars/arity/commit/40a89cb78a342281ecce8e674302788ab4be7c4f))
- **roxygen:** treat a wide tag separator as prose ([`d9f6a1b`](https://github.com/jolars/arity/commit/d9f6a1be7125da36513b8463bb322e27ae5563c3)), closes [#96](https://github.com/jolars/arity/issues/96)

### Dependencies
- updated crates/arity-parser to v0.3.0

## [0.2.0](https://github.com/jolars/arity/compare/arity-formatter-v0.1.0...arity-formatter-v0.2.0) (2026-08-06)

### Features
- **formatter:** re-export `rowan` for embedders ([`81a8d02`](https://github.com/jolars/arity/commit/81a8d028ef7522dd3c7cc1ccc710b584bd9f3bb7))
- **format:** honor package roxygen markdown default ([`5606f61`](https://github.com/jolars/arity/commit/5606f61ec5326469643902c7ae793050d8d4442a))
- **formatter:** options-taking `format_with_options` entry ([`76ab583`](https://github.com/jolars/arity/commit/76ab583c1f07efb504995a8e2578f9791513c9ea))
- **formatter:** add serde and schema features ([`341e173`](https://github.com/jolars/arity/commit/341e173a76dbef8d5aa998b716d8f2262b4ed603))

### Dependencies
- updated crates/arity-parser to v0.2.0
