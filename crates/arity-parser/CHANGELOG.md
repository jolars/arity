# Changelog

## [0.5.3](https://github.com/jolars/arity/compare/arity-parser-v0.5.2...arity-parser-v0.5.3) (2026-08-31)

### Bug Fixes
- **parser:** attach bodies after comments ([`0de7517`](https://github.com/jolars/arity/commit/0de751795955e852db67e989f0e2b9e162d2e9dc))

## [0.5.2](https://github.com/jolars/arity/compare/arity-parser-v0.5.1...arity-parser-v0.5.2) (2026-08-21)

### Bug Fixes
- **parser:** match read.dcf duplicate lookup ([`55f7b68`](https://github.com/jolars/arity/commit/55f7b6881fa5111c05307cb6250e99e65b5d7a37))
- **parser:** match read.dcf empty-line folding ([`7ce1789`](https://github.com/jolars/arity/commit/7ce17894fc73e6b5b2c7e5f7075180f331494d44))
- **parser:** span ordinary roxygen comments ([`592a5fe`](https://github.com/jolars/arity/commit/592a5fe17db1756fd6a8dde0d70236d102fa1cff))
- **lint:** address future corpus false positives ([`5615b43`](https://github.com/jolars/arity/commit/5615b436a38e72c5148463cc98552026847a0261))
- **parser:** reject return in native pipes ([`197a091`](https://github.com/jolars/arity/commit/197a0917736ad76ee1d8253ed688d260dcf5765c))

## [0.5.1](https://github.com/jolars/arity/compare/arity-parser-v0.5.0...arity-parser-v0.5.1) (2026-08-17)

### Bug Fixes
- **parser:** stop unterminated `%` at the line end ([`96ef35b`](https://github.com/jolars/arity/commit/96ef35b5ef61721ba27a08652eb7061a13ad1d31)), closes [#107](https://github.com/jolars/arity/issues/107)
- **roxygen:** classify Rd block macros by name ([`49f2f40`](https://github.com/jolars/arity/commit/49f2f40771d38496a46af29d6326515c28838dd3)), closes [#106](https://github.com/jolars/arity/issues/106)
- **parser:** lex non-ASCII letters as identifiers ([`e9a8974`](https://github.com/jolars/arity/commit/e9a8974f8c8e88a50e62f2d4c4be2e60644c4966)), closes [#108](https://github.com/jolars/arity/issues/108)
- **parser:** diagnose invalid function formal lists ([`6b63efe`](https://github.com/jolars/arity/commit/6b63efe459bc703aefba1079346aecc2d462b1cd)), closes [#109](https://github.com/jolars/arity/issues/109)

### Performance Improvements
- **parser:** verify a staged chain without rebuilding it ([`35976b9`](https://github.com/jolars/arity/commit/35976b97e4979e16f6ea965cb2b7990f61735545))
- **parser:** borrow token text from the input ([`b439507`](https://github.com/jolars/arity/commit/b4395070a10e3ac9e64ff2057800a9efac1295a8))

## [0.5.0](https://github.com/jolars/arity/compare/arity-parser-v0.4.0...arity-parser-v0.5.0) (2026-08-14)

### Breaking changes
- **deps:** bump `rowan` to 0.17 ([`64ad43b`](https://github.com/jolars/arity/commit/64ad43be2125e8713e0656307d094a8e0a0601a4))

### Features
- **parser:** record a directive's prefix range ([`72d8536`](https://github.com/jolars/arity/commit/72d853656683116b4f5716c67dfc805ec76ebe3e))
- **parser:** add the shared arity directive grammar ([`39651b0`](https://github.com/jolars/arity/commit/39651b0789e6bd6de24fcc741d24077cb79e3902))
- **ast:** add `RoxygenTag::value_text` ([`5b35b6d`](https://github.com/jolars/arity/commit/5b35b6dd190c9a41cd4f0cd2dbbe2b28fe7ceb09))
- **parser:** re-export the dependency-field helpers from dcf ([`fbb8183`](https://github.com/jolars/arity/commit/fbb8183efa0bfd4140b3b82ad40049514d86b6dc))
- **linter:** add a rule trait for the DCF grammar ([`f3779d5`](https://github.com/jolars/arity/commit/f3779d50de5052a0985bb9bd0e7dac8adbeb6565))
- **dcf:** parse structured dependency entries ([`e5ee844`](https://github.com/jolars/arity/commit/e5ee8448186e949c14fd8839816846b2be9b935d))
- **parser:** add a lossless DCF CST parser ([`42ca268`](https://github.com/jolars/arity/commit/42ca26849d41bff20c8e4189c2a73b9ad9a166a1))

## [0.4.0](https://github.com/jolars/arity/compare/arity-parser-v0.3.0...arity-parser-v0.4.0) (2026-08-11)

### Features
- **parser:** expose an `is_single_expression` predicate ([`93cae60`](https://github.com/jolars/arity/commit/93cae60d35c342b62a82df4737971fe8e6b1722d))
- **parser:** make brace-less system Rd macros sticky ([`aed2df0`](https://github.com/jolars/arity/commit/aed2df0dcccd8633eac694e0b1ce0d6e86a1f61a))
- **parser:** stop unknown Rd macros consuming a group ([`374879e`](https://github.com/jolars/arity/commit/374879e0002373f955554ed651f98725a63268e9))

## [0.3.0](https://github.com/jolars/arity/compare/arity-parser-v0.2.0...arity-parser-v0.3.0) (2026-08-07)

### Features
- **parser:** let Rd macros win over literal backticks ([`c9a88c4`](https://github.com/jolars/arity/commit/c9a88c46558d137f839f7923bffa6b2d0ecbef22))
- **parser:** model zero-arity Rd user macros ([`884448e`](https://github.com/jolars/arity/commit/884448eadf47998aeb1e28644f10b90aaf1e982b))
- **parser:** model per-macro Rd argument arity ([`b0b3f34`](https://github.com/jolars/arity/commit/b0b3f345d31a01f190e2264197c5235e91da41d6))
- **parser:** model md blocks in block-macro bodies ([`26aaa75`](https://github.com/jolars/arity/commit/26aaa75ad0e4a8ed998388ed0b01840736cf77e6))
- **parser:** model multi-line Rd macro arguments ([`b3b88e4`](https://github.com/jolars/arity/commit/b3b88e400f75b31ee7bfd8fd91415e6454399045))
- **parser:** model block-macro tails and adjacent second arg groups ([`f7413a7`](https://github.com/jolars/arity/commit/f7413a7537cd6be7f9946881325157afbf20da12))
- **parser:** model `\eqn` and `\deqn` optional second argument ([`3051dd4`](https://github.com/jolars/arity/commit/3051dd4013f591292118eb61a9d8f0693aed2afc))
- **parser:** model parse_Rd verbatim args and Rd fragment reparse ([`903f500`](https://github.com/jolars/arity/commit/903f5006f932293b4b3f89f9651d146f20879343))
- **parser:** roxygen2 8.0.0 tag grammar ([`f6d3647`](https://github.com/jolars/arity/commit/f6d3647be8cd0d64cec61105856800d58d197b91))

### Bug Fixes
- **parser:** fold deep-indented block starts into an open paragraph ([`72ef2f1`](https://github.com/jolars/arity/commit/72ef2f1568dc6231f0f7337a783a6f33a8063be9))
- **roxygen:** treat a wide tag separator as prose ([`d9f6a1b`](https://github.com/jolars/arity/commit/d9f6a1be7125da36513b8463bb322e27ae5563c3)), closes [#96](https://github.com/jolars/arity/issues/96)

## [0.2.0](https://github.com/jolars/arity/compare/arity-parser-v0.1.0...arity-parser-v0.2.0) (2026-08-06)

### Features
- **parser:** caller-set roxygen markdown default ([`6367b0b`](https://github.com/jolars/arity/commit/6367b0bc291504fe6073edba31da486ea95a0c47)), closes [#94](https://github.com/jolars/arity/issues/94)
