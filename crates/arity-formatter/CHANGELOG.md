# Changelog

## [0.7.0](https://github.com/jolars/arity/compare/arity-formatter-v0.6.0...arity-formatter-v0.7.0) (2026-08-31)

### Features
- **formatter:** add verified formatting ([`a740ade`](https://github.com/jolars/arity/commit/a740aded663c232670bf094b78b46596fe8e7c23))

### Bug Fixes
- **formatter:** preserve else after comments ([`bdf224d`](https://github.com/jolars/arity/commit/bdf224de85a1dd4977d79b8675d7ec2945d50162)), fixes [#129](https://github.com/jolars/arity/issues/129)
- **formatter:** ignore comment trailing whitespace ([`4d924d0`](https://github.com/jolars/arity/commit/4d924d050a8d043903118113d139597fef14f0ae)), fixes [#128](https://github.com/jolars/arity/issues/128)
- **formatter:** keep bare if consequences ([`bd63cc8`](https://github.com/jolars/arity/commit/bd63cc8ff9976b4d9697d9229db0ed0909ceacb7)), fixes [#125](https://github.com/jolars/arity/issues/125)
- **formatter:** preserve comment order ([`44626da`](https://github.com/jolars/arity/commit/44626dad0cc4da45700fa2a7ffda05c89e8da375)), fixes [#123](https://github.com/jolars/arity/issues/123)

### Dependencies
- updated crates/arity-parser to v0.5.3

## [0.6.0](https://github.com/jolars/arity/compare/arity-formatter-v0.5.0...arity-formatter-v0.6.0) (2026-08-26)

### Features
- **format:** report outdated directives ([`eaa48e2`](https://github.com/jolars/arity/commit/eaa48e2791da1f88751dd0a2078135fa6e71a02c))
- **formatter:** format tribble calls as tables ([`cd62e90`](https://github.com/jolars/arity/commit/cd62e903ba5adc87aedc3fa9170eae680b01231a))
- **format:** align trailing comments ([`58d854c`](https://github.com/jolars/arity/commit/58d854c510c5ff90963bafcecd68d13b8f68ac89))

### Bug Fixes
- **formatter:** align Quarto annotations ([`12e8695`](https://github.com/jolars/arity/commit/12e869526cb0bfbeb8561265e4002d6462d18e88))
- **formatter:** preserve Quarto annotations ([`02678e5`](https://github.com/jolars/arity/commit/02678e5dea41bd2819a8f547b4b7441603e52165))
- **format:** preserve commas before comments ([`d529b97`](https://github.com/jolars/arity/commit/d529b97a951f0b00d37d0a0798e3d04f33d8c125)), closes [#117](https://github.com/jolars/arity/issues/117)

## [0.5.0](https://github.com/jolars/arity/compare/arity-formatter-v0.4.1...arity-formatter-v0.5.0) (2026-08-21)

### Features
- **lint:** flag unknown DESCRIPTION fields ([`1f52925`](https://github.com/jolars/arity/commit/1f5292522a89c028fa5a320c6227b788061a648e))

### Bug Fixes
- **parser:** match read.dcf duplicate lookup ([`55f7b68`](https://github.com/jolars/arity/commit/55f7b6881fa5111c05307cb6250e99e65b5d7a37))
- **parser:** match read.dcf empty-line folding ([`7ce1789`](https://github.com/jolars/arity/commit/7ce17894fc73e6b5b2c7e5f7075180f331494d44))
- **parser:** span ordinary roxygen comments ([`592a5fe`](https://github.com/jolars/arity/commit/592a5fe17db1756fd6a8dde0d70236d102fa1cff))

### Dependencies
- updated crates/arity-parser to v0.5.2

## [0.4.1](https://github.com/jolars/arity/compare/arity-formatter-v0.4.0...arity-formatter-v0.4.1) (2026-08-17)

### Bug Fixes
- **roxygen:** classify Rd block macros by name ([`49f2f40`](https://github.com/jolars/arity/commit/49f2f40771d38496a46af29d6326515c28838dd3)), closes [#106](https://github.com/jolars/arity/issues/106)

### Dependencies
- updated crates/arity-parser to v0.5.1

## [0.4.0](https://github.com/jolars/arity/compare/arity-formatter-v0.3.1...arity-formatter-v0.4.0) (2026-08-14)

### Breaking changes
- **deps:** bump `rowan` to 0.17 ([`64ad43b`](https://github.com/jolars/arity/commit/64ad43be2125e8713e0656307d094a8e0a0601a4))

### Features
- **formatter:** honor skip-file in a DESCRIPTION ([`f737e8b`](https://github.com/jolars/arity/commit/f737e8b03aec62b72e3a15f59f843ab38d42e07d))
- **formatter:** honor arity-format directives ([`922813c`](https://github.com/jolars/arity/commit/922813c86fccb2b1204c62b3c1b52ba6f786c268))
- format package DESCRIPTION files (#104) ([`40583bf`](https://github.com/jolars/arity/commit/40583bf0caf22499453080456dfac9e12e8c239d))

### Bug Fixes
- **formatter:** give an Rd `\item` its own line ([`481b9ac`](https://github.com/jolars/arity/commit/481b9ac1ccfa72f2db0496b475bf94012d1fa465))
- **formatter:** flush a block Rd macro's opener line ([`bd27d5c`](https://github.com/jolars/arity/commit/bd27d5c35f327475facea9b8128e0a3ce418b23d))

### Performance Improvements
- **formatter:** answer both prepasses in one green-tree walk ([`6f8a444`](https://github.com/jolars/arity/commit/6f8a4440d3efb68e65058044d5581de051412de6))

### Dependencies
- updated crates/arity-parser to v0.5.0

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
