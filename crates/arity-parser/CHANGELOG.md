# Changelog

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
