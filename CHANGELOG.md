# Changelog

## [0.23.0](https://github.com/jolars/arity/compare/v0.22.0...v0.23.0) (2026-09-06)

### Features
- **config:** publish schema ([`7180652`](https://github.com/jolars/arity/commit/7180652f028a203e43ea2527fdd064fa3301a44f))
- **lint:** add description text format rule ([`080b46c`](https://github.com/jolars/arity/commit/080b46ceeabc5dedea1907fb96f7a646fa1434b5))

### Bug Fixes
- bump `rust-version` to 1.89 and CI job ([`6683330`](https://github.com/jolars/arity/commit/6683330fe806effe4ecd0eee4b5dfbda040c3aef))

## [0.22.0](https://github.com/jolars/arity/compare/v0.21.0...v0.22.0) (2026-08-31)

### Features
- verify formatter preserves syntax ([`b144f7d`](https://github.com/jolars/arity/commit/b144f7dc58e945b12c1484822b524ed403338bb7))

### Bug Fixes
- **formatter:** preserve else after comments ([`bdf224d`](https://github.com/jolars/arity/commit/bdf224de85a1dd4977d79b8675d7ec2945d50162)), fixes [#129](https://github.com/jolars/arity/issues/129)
- **formatter:** ignore comment trailing whitespace ([`4d924d0`](https://github.com/jolars/arity/commit/4d924d050a8d043903118113d139597fef14f0ae)), fixes [#128](https://github.com/jolars/arity/issues/128)
- **formatter:** keep bare if consequences ([`bd63cc8`](https://github.com/jolars/arity/commit/bd63cc8ff9976b4d9697d9229db0ed0909ceacb7)), fixes [#125](https://github.com/jolars/arity/issues/125)
- **formatter:** preserve comment order ([`44626da`](https://github.com/jolars/arity/commit/44626dad0cc4da45700fa2a7ffda05c89e8da375)), fixes [#123](https://github.com/jolars/arity/issues/123)

### Dependencies
- updated crates/arity-formatter to v0.7.0
- updated crates/arity-parser to v0.5.3

## [0.21.0](https://github.com/jolars/arity/compare/v0.20.0...v0.21.0) (2026-08-26)

### Features
- **format:** report outdated directives ([`eaa48e2`](https://github.com/jolars/arity/commit/eaa48e2791da1f88751dd0a2078135fa6e71a02c))
- **lsp:** refresh cached config ([`6d82ca0`](https://github.com/jolars/arity/commit/6d82ca064d060cf3c5226f947e68bf3ae4b5bfb8))
- **format:** align trailing comments ([`58d854c`](https://github.com/jolars/arity/commit/58d854c510c5ff90963bafcecd68d13b8f68ac89))

### Bug Fixes
- **lint:** gate local promise arguments ([`2ec3e8b`](https://github.com/jolars/arity/commit/2ec3e8b9fab3ec41ff5b06e8c1a52b6f7a7b6265))
- **format:** preserve commas before comments ([`d529b97`](https://github.com/jolars/arity/commit/d529b97a951f0b00d37d0a0798e3d04f33d8c125)), closes [#117](https://github.com/jolars/arity/issues/117)

### Performance Improvements
- **lint:** reuse analyzed source for rendering ([`6cab16e`](https://github.com/jolars/arity/commit/6cab16eab1ca8d891b6a53cd9ddce17f930646cc))

### Dependencies
- updated crates/arity-formatter to v0.6.0

## [0.20.0](https://github.com/jolars/arity/compare/v0.19.1...v0.20.0) (2026-08-21)

### Features
- **lint:** add description encoding rule ([`f4f0ba3`](https://github.com/jolars/arity/commit/f4f0ba39327a572ba3f2a795826abb99ec5e4638))
- **lint:** flag unknown DESCRIPTION fields ([`1f52925`](https://github.com/jolars/arity/commit/1f5292522a89c028fa5a320c6227b788061a648e))
- **roxygen:** honor projector markdown default ([`533fc49`](https://github.com/jolars/arity/commit/533fc496f54cae49697ad8d8a5c1ee692a934367))
- **lint:** add pipe-return rule ([`18ab02a`](https://github.com/jolars/arity/commit/18ab02abe0eb0f2f6581b0cd4d6a09ea80682313))
- **lint:** add base call rewrites ([`a04dd36`](https://github.com/jolars/arity/commit/a04dd3624509780de2aa6152cb0e6d69983221ce))
- **lint:** flag assignments in return ([`72f7a77`](https://github.com/jolars/arity/commit/72f7a770f558530b85e1a4cecfff6223845970d0))
- **lint:** add all-equal rule ([`eebc4f7`](https://github.com/jolars/arity/commit/eebc4f73506f0f043ff341ce6365025d51a3c649))
- **lint:** add sprintf rule ([`a04b37c`](https://github.com/jolars/arity/commit/a04b37c2148dd4c973cbb38277ff4bfdcb311fa7))
- **lint:** add rep-times-ignored rule ([`efcf3ca`](https://github.com/jolars/arity/commit/efcf3ca9f2805bbec571046ac309b1559c0e27f3))
- **lint:** add missing-argument rule ([`594d564`](https://github.com/jolars/arity/commit/594d56474c2bafc13d3a609f43acb9a311ca5fa5))
- **lint:** detect NaN and NULL comparisons ([`88da964`](https://github.com/jolars/arity/commit/88da9641af33fe1e82131286533d3f4bdd5eddff))

### Bug Fixes
- **lsp:** skip discovery on unchanged membership reseed ([`9c92a00`](https://github.com/jolars/arity/commit/9c92a000a6612344d4a249b0ade525c989ed45b9))
- **lsp:** complete dependency names without faulting index in ([`b31b7db`](https://github.com/jolars/arity/commit/b31b7dbf235c488586604a1fc805353c33459dc0))
- **lsp:** reload package index on cache root or meta change ([`79a86c3`](https://github.com/jolars/arity/commit/79a86c3a5eb976f2a375b7fa6f14bc8291091936))
- **rindex:** derive package views from a single read ([`92608d5`](https://github.com/jolars/arity/commit/92608d5d97ce3a742e96c1ede8b6cc203585ddfb))
- **lsp:** stop leaking project snapshots per metadata change ([`8cf322f`](https://github.com/jolars/arity/commit/8cf322fc027b4d2376bbe745004d34e7a23b3194)), fixes [#116](https://github.com/jolars/arity/issues/116)
- **lsp:** record query log only during observation ([`9959129`](https://github.com/jolars/arity/commit/995912972850d056e71fbfa1964f0abb144ef68f))
- **parser:** match read.dcf duplicate lookup ([`55f7b68`](https://github.com/jolars/arity/commit/55f7b6881fa5111c05307cb6250e99e65b5d7a37))
- **parser:** match read.dcf empty-line folding ([`7ce1789`](https://github.com/jolars/arity/commit/7ce17894fc73e6b5b2c7e5f7075180f331494d44))
- **lint:** recognize string-based reads ([`b2b7a23`](https://github.com/jolars/arity/commit/b2b7a238d40b9e8194c1533b8847fd8da25689a1)), closes [#115](https://github.com/jolars/arity/issues/115)
- **parser:** span ordinary roxygen comments ([`592a5fe`](https://github.com/jolars/arity/commit/592a5fe17db1756fd6a8dde0d70236d102fa1cff))
- **lint:** address future corpus false positives ([`5615b43`](https://github.com/jolars/arity/commit/5615b436a38e72c5148463cc98552026847a0261))

### Performance Improvements
- **lsp:** load package index lazily, once per cache root ([`de39edf`](https://github.com/jolars/arity/commit/de39edf9a291c7482a66171cea24982976721807))
- **format:** bound formatting line diffs ([`c3f9bc5`](https://github.com/jolars/arity/commit/c3f9bc5ec0353d5e4f43cfb11fd19f82f0cb5593))

### Dependencies
- updated crates/arity-formatter to v0.5.0
- updated crates/arity-parser to v0.5.2

## [0.19.1](https://github.com/jolars/arity/compare/v0.19.0...v0.19.1) (2026-08-17)

### Bug Fixes
- **roxygen:** classify Rd block macros by name ([`49f2f40`](https://github.com/jolars/arity/commit/49f2f40771d38496a46af29d6326515c28838dd3)), closes [#106](https://github.com/jolars/arity/issues/106)
- accept non-ASCII letters in syntactic R names ([`2c29b2f`](https://github.com/jolars/arity/commit/2c29b2f94215dac1adfe86ea1643ba7b935f5dc6)), refs [#108](https://github.com/jolars/arity/issues/108)

### Performance Improvements
- **linter:** seed the project scope once, not per document ([`f12344c`](https://github.com/jolars/arity/commit/f12344cb140631f79ab568933d1d48a4639e6abb))
- **project:** share the per-file facts instead of copying them twice ([`ebb1d4f`](https://github.com/jolars/arity/commit/ebb1d4f127896240f966547e0352fbf71be19ee1))
- **linter:** hand the salsa database teardown to the pool ([`210f8fc`](https://github.com/jolars/arity/commit/210f8fc92641e7fa4527330115f792abeae25bca))
- **linter:** read the prologue's files in parallel ([`938367b`](https://github.com/jolars/arity/commit/938367b9b9a8220bcab7a75a0769bad5fe747db7))
- **project:** stop reading DESCRIPTION twice per package root ([`262e460`](https://github.com/jolars/arity/commit/262e4604580582aa84da808e08a8243da28bfdf8))
- **project:** walk to a package root once per directory ([`7b75a3c`](https://github.com/jolars/arity/commit/7b75a3c1461350bc319f3d5c4339bb6d556eee19))
- **project:** share one NAMESPACE's sets across a package's members ([`21d1738`](https://github.com/jolars/arity/commit/21d17383ae01521439dcc90563633743a2efd8d5))
- **project:** answer the file-set relations from one shared member set ([`2777621`](https://github.com/jolars/arity/commit/277762165fe6fc748d780d1593205e095cca898f))
- **project:** derive visibility from shared layers, not per-member sets ([`36005a0`](https://github.com/jolars/arity/commit/36005a0fd114dc228433756d40187f40f14944fa))
- **project:** fold a package clique once, not per ordered pair ([`e1ecf4e`](https://github.com/jolars/arity/commit/e1ecf4ec9b8023ecb93d0c49a6247e6fce0524bb))
- **linter:** warm every fold input before the parallel passes ([`6c0ee06`](https://github.com/jolars/arity/commit/6c0ee06e87905f069a5a6766e0f47be241ea76b9))
- **lsp,incremental:** verify edit slices without applying them ([`7654bbd`](https://github.com/jolars/arity/commit/7654bbd6276891d09bbbc9ab458addc9621f9543))
- **text,lsp:** back document text with Arc<str> ([`e80c63a`](https://github.com/jolars/arity/commit/e80c63ac068635b33723ca50d534e646da6ed306))
- **rindex:** load the lint index lazily, per package ([`f472c69`](https://github.com/jolars/arity/commit/f472c69ecbd1fd3239004d976cd4b24c73e4c66a))
- **linter:** find a directive's comment by offset ([`0151f48`](https://github.com/jolars/arity/commit/0151f48471e6936e673629d52542e8ca95d66774))

### Dependencies
- updated crates/arity-formatter to v0.4.1
- updated crates/arity-parser to v0.5.1

## [0.19.0](https://github.com/jolars/arity/compare/v0.18.0...v0.19.0) (2026-08-14)

The most prominent feature of this release is that Arity now handles the `DESCRIPTION`
file in R projects: it lints it, formats it, and provides language-server features like
inlay hints for loaded versions.

### Breaking changes
- **deps:** bump `rowan` to 0.17 ([`64ad43b`](https://github.com/jolars/arity/commit/64ad43be2125e8713e0656307d094a8e0a0601a4))

### Features
- **lint:** add deprecated-suppression ([`41c37c0`](https://github.com/jolars/arity/commit/41c37c062d812c7326cfa6866a04a6604084f8cb))
- **lint:** add region directives and lint the new forms ([`fdd1e5e`](https://github.com/jolars/arity/commit/fdd1e5e89cb7461e22a478bb2a47ad43742bba41))
- **lsp:** send line-scoped formatting edits ([`406b5ab`](https://github.com/jolars/arity/commit/406b5abc7a34c7dbacb4e87ddfbb0fe84f1084fd))
- **lsp:** add inlay hints for DESCRIPTION deps ([`7aa282a`](https://github.com/jolars/arity/commit/7aa282a008861c40806b126de4898013153b6e6e))
- **lint:** add `description-empty-person` ([`41c7786`](https://github.com/jolars/arity/commit/41c77868b6f7a9dd8849f4dd91bb42439b3c3c13))
- **lint:** add `description-authors-at-r` ([`21c4597`](https://github.com/jolars/arity/commit/21c4597dbddb9893391c76ce6cc3cdf07de7a3de))
- **lint:** add `description-malformed-maintainer` ([`f813cb5`](https://github.com/jolars/arity/commit/f813cb538927e66faf9f1d7dc4b1dacefd655808))
- **lint:** add `description-malformed-version` ([`31fcb76`](https://github.com/jolars/arity/commit/31fcb765d659c8f00d615f34abccb96aa6a3aa96))
- **lint:** add `description-malformed-name` ([`3347825`](https://github.com/jolars/arity/commit/3347825cca35ddecc36ad951f2425bd3dcd7d106))
- **lint:** add `description-package-in-multiple-fields` ([`41cd5af`](https://github.com/jolars/arity/commit/41cd5af4f8c792116eded921bd5c7f4dc4cf00f7))
- format package DESCRIPTION files (#104) ([`40583bf`](https://github.com/jolars/arity/commit/40583bf0caf22499453080456dfac9e12e8c239d))
- **lsp:** hover a DESCRIPTION dependency ([`5655f64`](https://github.com/jolars/arity/commit/5655f647824827b0f1b75ee2afe9b06400943d26))
- **lsp:** complete package names in DESCRIPTION dependency fields ([`76f339f`](https://github.com/jolars/arity/commit/76f339f69174642cef22c0d795f4187ebf537247))
- **lsp:** make an open DESCRIPTION authoritative in salsa ([`2537f39`](https://github.com/jolars/arity/commit/2537f398558c17a2e99f3423291a37fc397860c7))
- **lsp:** publish DESCRIPTION diagnostics ([`9b433a5`](https://github.com/jolars/arity/commit/9b433a52c0d34411ade474027831737b8de573f9))
- **lsp:** route DESCRIPTION documents away from the R pipeline ([`23a61fc`](https://github.com/jolars/arity/commit/23a61fc327464e8b885d13f0ed8d4cf02b616686))
- **linter:** add unused-dependency ([`b6d73cf`](https://github.com/jolars/arity/commit/b6d73cf7f7a6701791dc5f1584a88ad5b89cce05))
- **linter:** add undeclared-dependency ([`d35c7fc`](https://github.com/jolars/arity/commit/d35c7fc13efdffe407b534a0d0be3ea970e46e21))
- **linter:** add three DESCRIPTION rules ([`7ac50e4`](https://github.com/jolars/arity/commit/7ac50e4024343398c3f1d41da462029eccbd6592))
- **linter:** discover and lint DESCRIPTION files ([`18c9366`](https://github.com/jolars/arity/commit/18c9366cc7bb413bcd3fcc88c5d8c2065cbb74ce))
- **linter:** add a rule trait for the DCF grammar ([`f3779d5`](https://github.com/jolars/arity/commit/f3779d50de5052a0985bb9bd0e7dac8adbeb6565))
- **rindex:** treat declared dependencies as referenced ([`256d4c8`](https://github.com/jolars/arity/commit/256d4c8b7f2a831dd4ab958823b5fb75b301482a))
- **project:** defer the import(pkg) verdict to the library index ([`6f580bf`](https://github.com/jolars/arity/commit/6f580bf27e75c5314b8a250ab99618c3f95a189e))
- **project:** resolve against declared Depends ([`d999290`](https://github.com/jolars/arity/commit/d9992901c758845e83dd53671c3c36bdc40edf2e))
- **incremental:** make DESCRIPTION a salsa input ([`3340923`](https://github.com/jolars/arity/commit/33409232acd5d931f123efdf870564ad9c069bc1))
- **parser:** add a lossless DCF CST parser ([`42ca268`](https://github.com/jolars/arity/commit/42ca26849d41bff20c8e4189c2a73b9ad9a166a1))
- **lsp:** widen call hierarchy edge attribution ([`00a798d`](https://github.com/jolars/arity/commit/00a798dd4d7dcb62e1c3d67bb35cd8defab4501a))
- **formatter:** honor skip-file in a DESCRIPTION ([`f737e8b`](https://github.com/jolars/arity/commit/f737e8b03aec62b72e3a15f59f843ab38d42e07d))
- **formatter:** honor arity-format directives ([`922813c`](https://github.com/jolars/arity/commit/922813c86fccb2b1204c62b3c1b52ba6f786c268))
- **parser:** record a directive's prefix range ([`72d8536`](https://github.com/jolars/arity/commit/72d853656683116b4f5716c67dfc805ec76ebe3e))
- **parser:** add the shared arity directive grammar ([`39651b0`](https://github.com/jolars/arity/commit/39651b0789e6bd6de24fcc741d24077cb79e3902))
- **ast:** add `RoxygenTag::value_text` ([`5b35b6d`](https://github.com/jolars/arity/commit/5b35b6dd190c9a41cd4f0cd2dbbe2b28fe7ceb09))
- **parser:** re-export the dependency-field helpers from dcf ([`fbb8183`](https://github.com/jolars/arity/commit/fbb8183efa0bfd4140b3b82ad40049514d86b6dc))
- **dcf:** parse structured dependency entries ([`e5ee844`](https://github.com/jolars/arity/commit/e5ee8448186e949c14fd8839816846b2be9b935d))
- **vscode:** gate inlay hints on the feature toggle ([`dd6fde8`](https://github.com/jolars/arity/commit/dd6fde821a63fa24a48517cf95472f035264e520))
- **code:** claim DESCRIPTION for the language server ([`0fe3077`](https://github.com/jolars/arity/commit/0fe30773c0c30d8f16f61f333c0fca6b47acef8b))

### Bug Fixes
- **linter:** skip `coalesce` in the `%||%` definition ([`2114c88`](https://github.com/jolars/arity/commit/2114c88ede126119788f924aebd26f4632eca0c2))
- **semantic:** mark every def a closure read reaches ([`32e98e2`](https://github.com/jolars/arity/commit/32e98e2f1f3930105c470033e14310ce5f7ea65e))
- **semantic:** resolve names bound by `useDynLib` ([`c72462a`](https://github.com/jolars/arity/commit/c72462a6d28d3aa5f826c69e5d817e3ba3640e70))
- **semantic:** make defusing operators unquote-aware ([`dd2ce0a`](https://github.com/jolars/arity/commit/dd2ce0a96fad2aa3297b6aeeab2a171d2a16d564))
- **roxygen:** read projector topic names whole ([`f7b6d3e`](https://github.com/jolars/arity/commit/f7b6d3e4688ed706f4df0f2ac098150e42540cec))
- **lint:** resolve roxygen topics across the package ([`7e6e7ce`](https://github.com/jolars/arity/commit/7e6e7ce82071926a53ed6f4add86e28b6cdf582a))
- **lint:** exempt S3 methods from `unused-binding` ([`a107859`](https://github.com/jolars/arity/commit/a1078598efa1cd279bd31490d2c358ff757f3c40))
- **lint:** judge roxygen topics, not single blocks ([`3357e83`](https://github.com/jolars/arity/commit/3357e83a9da4b8baf4f3b92f8fb864f3ba88c36c))
- **cli:** give `lint --fix` the project scope ([`85d6d1e`](https://github.com/jolars/arity/commit/85d6d1e22abb00ace742ee538c2aabecc31f310e))
- **lint:** skip `roxygen-param` on `@noRd` blocks ([`0b842d9`](https://github.com/jolars/arity/commit/0b842d9d24df2ec6c5e1b40693261dfa20b57ecf))
- **semantic:** mask the `.External2` routine name ([`c090566`](https://github.com/jolars/arity/commit/c090566e9e114a5e324e5a3d458c67895a91007a))
- **semantic:** never bind the walrus operator ([`d1251b9`](https://github.com/jolars/arity/commit/d1251b9d440e792b325e5c43559de1297938ddd9))
- **semantic:** match backticked names to NAMESPACE ([`078d0e0`](https://github.com/jolars/arity/commit/078d0e040e67096303b1f4f4fd451730e1fe6985))
- **lint:** name the actual range in `seq` messages ([`5d4cbab`](https://github.com/jolars/arity/commit/5d4cbab256b1ac3e359e31bc8e1c551c189f891c))
- **lint:** skip roxygen topic rules on S3 methods ([`029af86`](https://github.com/jolars/arity/commit/029af86114891fa2b3f55fc8a41ba7c2e50bae38))
- **lsp:** refresh the graph when a DESCRIPTION disappears ([`f987130`](https://github.com/jolars/arity/commit/f98713095c2980c23c2ee8b56123d8671ab3343a))
- **linter:** skip a fixture package's DESCRIPTION when walking ([`0d70adb`](https://github.com/jolars/arity/commit/0d70adb29157f91b0a879f1b6bfa8580f9ef531a))
- **semantic:** resolve backtick-quoted names ([`cc3d1db`](https://github.com/jolars/arity/commit/cc3d1dbfa7a0bbf9d10bd4c61a476e2d48f57358))
- **rindex:** keep only the packages import() names ([`205957c`](https://github.com/jolars/arity/commit/205957ca2fd6ebfdf042007e6238748d6621bac9))
- **formatter:** give an Rd `\item` its own line ([`481b9ac`](https://github.com/jolars/arity/commit/481b9ac1ccfa72f2db0496b475bf94012d1fa465))
- **formatter:** flush a block Rd macro's opener line ([`bd27d5c`](https://github.com/jolars/arity/commit/bd27d5c35f327475facea9b8128e0a3ce418b23d))

### Performance Improvements
- **linter:** keep the standalone attach set lazy ([`a238d64`](https://github.com/jolars/arity/commit/a238d648de42c02357b51e3b42e06a06c9c0dec3))
- **lsp:** invalidate package metadata by path ([`629ac9c`](https://github.com/jolars/arity/commit/629ac9cd01dc3487a875dc25100bf24b7d433472))
- **formatter:** answer both prepasses in one green-tree walk ([`6f8a444`](https://github.com/jolars/arity/commit/6f8a4440d3efb68e65058044d5581de051412de6))

### Dependencies
- updated crates/arity-formatter to v0.4.0
- updated crates/arity-parser to v0.5.0

## [0.18.0](https://github.com/jolars/arity/compare/v0.17.0...v0.18.0) (2026-08-11)

### Features
- **cli:** read stdin from `-`, not a bare terminal ([`ae70cbb`](https://github.com/jolars/arity/commit/ae70cbb3c86455fcf368c73503c774fe65ceff39))
- **text:** add TextBuffer pairing text with its line index ([`bff3617`](https://github.com/jolars/arity/commit/bff3617575cb920d0f520c5a39fb5235b14c5b1e))
- **text:** patch the line index across an edit ([`06c10c5`](https://github.com/jolars/arity/commit/06c10c598103ebb49faeade850bcdb0d73872eea))
- **semantic:** mask data.table subsets, gate masking verbs ([`aa7a8aa`](https://github.com/jolars/arity/commit/aa7a8aa7b56f51d620d3ec66a4de0780652c7b2f))
- **lsp:** support folder renames ([`00151ee`](https://github.com/jolars/arity/commit/00151ee19e749c31482d1de4112318a8d540b151))
- **parser:** make brace-less system Rd macros sticky ([`aed2df0`](https://github.com/jolars/arity/commit/aed2df0dcccd8633eac694e0b1ce0d6e86a1f61a))
- **parser:** stop unknown Rd macros consuming a group ([`374879e`](https://github.com/jolars/arity/commit/374879e0002373f955554ed651f98725a63268e9))
- **parser:** expose an `is_single_expression` predicate ([`93cae60`](https://github.com/jolars/arity/commit/93cae60d35c342b62a82df4737971fe8e6b1722d))

### Bug Fixes
- **lsp:** drop file renames that leave the workspace scope ([`237daea`](https://github.com/jolars/arity/commit/237daea34f724f94661bd37260848cfddf64e25b))
- **lsp:** follow a renamed workspace root through a rename ([`1de3fb2`](https://github.com/jolars/arity/commit/1de3fb2efd4994646a55e85dde5aaab1f54ed6d9))
- **roxygen:** keep a trailing `;` in a code span as `\code` ([`a1fcff9`](https://github.com/jolars/arity/commit/a1fcff97d632179cd03b3b9e909ad4e909de3521)), closes [#99](https://github.com/jolars/arity/issues/99)
- **formatter:** keep blank line after a comment ([`e50f7fe`](https://github.com/jolars/arity/commit/e50f7fef855b33152f9f0f948f9efe14072185ae))

### Performance Improvements
- **lsp:** answer position reads off the shared index ([`e53de52`](https://github.com/jolars/arity/commit/e53de527dea8df5b1459f215a052e57757226b01))
- **lsp:** answer whole-document reads off the shared index ([`a354814`](https://github.com/jolars/arity/commit/a3548145f5c4ac3eb5308c51cda0d68d41a0021e))
- **lsp:** thread the shared buffer into the lint request ([`9b610a0`](https://github.com/jolars/arity/commit/9b610a090a372091fd905a43f3a969077790cd6c))
- **text:** scan for line starts with memchr ([`6aa8b3b`](https://github.com/jolars/arity/commit/6aa8b3bed8ae6c4d1f67762f9a8dd422a8cfbbb5))

### Dependencies
- updated crates/arity-formatter to v0.3.1
- updated crates/arity-parser to v0.4.0

## [0.17.0](https://github.com/jolars/arity/compare/v0.16.0...v0.17.0) (2026-08-07)

### Features
- **parser:** let Rd macros win over literal backticks ([`c9a88c4`](https://github.com/jolars/arity/commit/c9a88c46558d137f839f7923bffa6b2d0ecbef22))
- **parser:** model zero-arity Rd user macros ([`884448e`](https://github.com/jolars/arity/commit/884448eadf47998aeb1e28644f10b90aaf1e982b))
- **parser:** model per-macro Rd argument arity ([`b0b3f34`](https://github.com/jolars/arity/commit/b0b3f345d31a01f190e2264197c5235e91da41d6))
- **roxygen:** expand R system Rd user macros ([`4867f69`](https://github.com/jolars/arity/commit/4867f694a6d68d8c97f82c310b917ed2fb4d6c97))
- **parser:** model md blocks in block-macro bodies ([`26aaa75`](https://github.com/jolars/arity/commit/26aaa75ad0e4a8ed998388ed0b01840736cf77e6))
- **parser:** model multi-line Rd macro arguments ([`b3b88e4`](https://github.com/jolars/arity/commit/b3b88e400f75b31ee7bfd8fd91415e6454399045))
- **roxygen:** collapse within-block same-head sections ([`104da98`](https://github.com/jolars/arity/commit/104da989ff98f525a753578573d5438764de3261))
- **roxygen:** project block-form verbatim macro bodies per-line ([`c5e5f8b`](https://github.com/jolars/arity/commit/c5e5f8b7b580efecdaad0e56a574f801a450c8ad))
- **parser:** model `\eqn` and `\deqn` optional second argument ([`3051dd4`](https://github.com/jolars/arity/commit/3051dd4013f591292118eb61a9d8f0693aed2afc))
- **roxygen:** model demoted md code spans via fragile gating ([`30f0127`](https://github.com/jolars/arity/commit/30f0127031e9b4f4fc477f1f021a2cea945304bc))
- **roxygen:** model parse_Rd brace recovery for kept md sections ([`6121854`](https://github.com/jolars/arity/commit/612185494977522608b474656ea74cfdb15bc520))
- **lint:** add `r-compat` and `roxygen2-compat` rules ([`9098185`](https://github.com/jolars/arity/commit/90981859908f88a520914e219f8a91c902ab9ab6))
- **config:** add `[compat]` minimum-version floors ([`4152c80`](https://github.com/jolars/arity/commit/4152c80a9c4093832464c8265a0ab93e5743c48e))
- **roxygen:** model roxygen2 8.0.0 grammar additions ([`501180c`](https://github.com/jolars/arity/commit/501180ce824f95c0187beb05d0a8a8ff4b61ac1b))
- **roxygen:** track roxygen2 8.0.0 as the parity oracle ([`925411f`](https://github.com/jolars/arity/commit/925411f02952ed145bca189a5ae00b1135cbceaa))
- **linter:** add `outdated-suppression` ([`e017cbf`](https://github.com/jolars/arity/commit/e017cbf0b67d3d2ceb4dae78396976fdc3e08033))
- **linter:** add `unexplained-suppression` ([`b84eea9`](https://github.com/jolars/arity/commit/b84eea93fe7de6153bfd74fb0750c5006d6e9f56))
- **linter:** add `blanket-suppression` ([`cba661e`](https://github.com/jolars/arity/commit/cba661ebc6d1545bb906f20f7784925c89628699))
- **linter:** add `misnamed-suppression` ([`404baa7`](https://github.com/jolars/arity/commit/404baa70f84f332fa0ad7645cf1d205abfe7cc58))
- **linter:** add `duplicated-function-definition` ([`f3ec2fc`](https://github.com/jolars/arity/commit/f3ec2fc7a08f7199838a0cd8b8534a898e21a348))
- **linter:** add `unused-function` ([`740bf81`](https://github.com/jolars/arity/commit/740bf810ea8a19fe98630b477e3f43234cfcf84b))
- **linter:** add `internal-function` ([`6632b06`](https://github.com/jolars/arity/commit/6632b061aec49f72b303ca5dd35b611309d9c666))
- **cli:** suppress `format --check` diff under `--quiet` ([`3865fb0`](https://github.com/jolars/arity/commit/3865fb0c07b7db4de41f2a9f94155af4fb111af0))
- **linter:** add `for-loop-index`/`for-loop-dup-index` ([`e75efad`](https://github.com/jolars/arity/commit/e75efadbdbc08c005d4a956d1307f44e20c07435))
- **linter:** add `download-file` ([`bea9893`](https://github.com/jolars/arity/commit/bea98930dfe9d45c2f370a8408ae6d1ab2da18bd))
- **linter:** add per-rule config and `undesirable-function` ([`4f7a112`](https://github.com/jolars/arity/commit/4f7a112069b284fb034029e8d52c0bcbf8183df8))
- **formatter:** classify `@prop` and `@R6method` tags ([`a0e81d0`](https://github.com/jolars/arity/commit/a0e81d09890d0935c36d219c640e6cab679f411e))
- **parser:** model block-macro tails and adjacent second arg groups ([`f7413a7`](https://github.com/jolars/arity/commit/f7413a7537cd6be7f9946881325157afbf20da12))
- **parser:** model parse_Rd verbatim args and Rd fragment reparse ([`903f500`](https://github.com/jolars/arity/commit/903f5006f932293b4b3f89f9651d146f20879343))
- **parser:** roxygen2 8.0.0 tag grammar ([`f6d3647`](https://github.com/jolars/arity/commit/f6d3647be8cd0d64cec61105856800d58d197b91))
- **vscode:** add per-feature enable toggles ([`e8370b0`](https://github.com/jolars/arity/commit/e8370b096f2b5883c94271af399ab5f44b028136))

### Bug Fixes
- **roxygen:** treat a wide tag separator as prose ([`d9f6a1b`](https://github.com/jolars/arity/commit/d9f6a1be7125da36513b8463bb322e27ae5563c3)), closes [#96](https://github.com/jolars/arity/issues/96)
- **formatter:** keep marker-less remainder after a block macro ([`40a89cb`](https://github.com/jolars/arity/commit/40a89cb78a342281ecce8e674302788ab4be7c4f))
- **parser:** fold deep-indented block starts into an open paragraph ([`72ef2f1`](https://github.com/jolars/arity/commit/72ef2f1568dc6231f0f7337a783a6f33a8063be9))

### Dependencies
- updated crates/arity-formatter to v0.3.0
- updated crates/arity-parser to v0.3.0

## [0.16.0](https://github.com/jolars/arity/compare/v0.15.0...v0.16.0) (2026-08-06)

### Features
- **lsp:** thread roxygen markdown default through salsa and lint ([`6bd4801`](https://github.com/jolars/arity/commit/6bd4801a2804e8c01f59f25689589142edf8d2ab))
- **format:** honor package roxygen markdown default ([`5606f61`](https://github.com/jolars/arity/commit/5606f61ec5326469643902c7ae793050d8d4442a))
- **project:** discover roxygen markdown default statically ([`be86db1`](https://github.com/jolars/arity/commit/be86db1af2fb8d20f9af016acd1c15b0f192d5df))
- **rindex:** opt-in `search()`-diff attach probe ([`deecf3e`](https://github.com/jolars/arity/commit/deecf3e74496dea03788bb904cf88e7e00862791))
- **rindex:** capture attach sets at harvest ([`4c6e7d8`](https://github.com/jolars/arity/commit/4c6e7d89ee9362930e027a1f8fe033dcf70b2238))
- **lint:** gate undefined-symbol on harvested attach sets ([`ed92f87`](https://github.com/jolars/arity/commit/ed92f8769a3f435394a0215ef88be32d899a2d1d))
- **rindex:** resolve members from harvested attach sets ([`492cfd0`](https://github.com/jolars/arity/commit/492cfd05db1798d13758735e7e68d902f0f04e04))
- **rindex:** add `attaches` to the index schema ([`731c4db`](https://github.com/jolars/arity/commit/731c4db8fcf62770b5cf28435f89c0a1871ac141))
- **formatter:** re-export `rowan` for embedders ([`81a8d02`](https://github.com/jolars/arity/commit/81a8d028ef7522dd3c7cc1ccc710b584bd9f3bb7))
- **formatter:** options-taking `format_with_options` entry ([`76ab583`](https://github.com/jolars/arity/commit/76ab583c1f07efb504995a8e2578f9791513c9ea))
- **formatter:** add serde and schema features ([`341e173`](https://github.com/jolars/arity/commit/341e173a76dbef8d5aa998b716d8f2262b4ed603))
- **parser:** caller-set roxygen markdown default ([`6367b0b`](https://github.com/jolars/arity/commit/6367b0bc291504fe6073edba31da486ea95a0c47)), closes [#94](https://github.com/jolars/arity/issues/94)

### Bug Fixes
- **lsp:** expand meta-package members in `packages_to_build` ([`a5b7466`](https://github.com/jolars/arity/commit/a5b7466d015dee11d6830b7f87369e26db5896f7))
- **linter:** simulate R argument matching for model-frame calls ([`7eb78e0`](https://github.com/jolars/arity/commit/7eb78e096d9c53d187666e410c4a3a2f93f2a563))

### Dependencies
- updated crates/arity-formatter to v0.2.0
- updated crates/arity-parser to v0.2.0

## [0.15.0](https://github.com/jolars/arity/compare/v0.14.0...v0.15.0) (2026-08-05)

### Features
- **build:** add installation scripts ([`b620c58`](https://github.com/jolars/arity/commit/b620c58bab56738f2ce7101be711e04629fe543d))
- **linter:** mask model-frame args in `undefined-symbol` ([`1fef3ea`](https://github.com/jolars/arity/commit/1fef3ea9bfa649f49e0c241b14b43ddc963e28b5))
- **lsp:** make nested functions call-hierarchy items ([`5e3eb7f`](https://github.com/jolars/arity/commit/5e3eb7f9c447accbf64f82fe7a8db7aa5d2afc32))
- **lsp:** trigger signature help on `=` ([`1021a4b`](https://github.com/jolars/arity/commit/1021a4b39733fa99e3cd84cbe19c398e5491f094))
- **linter:** add `unnecessary-nesting` rule ([`501f451`](https://github.com/jolars/arity/commit/501f4510056c42ca353e0a72519394ee222e9137))
- **lint:** add implicit-assignment rule ([`64a3f5e`](https://github.com/jolars/arity/commit/64a3f5e2928ede3f736e501a9101fda29935ddb5))
- **lint:** add empty-assignment rule ([`77efa97`](https://github.com/jolars/arity/commit/77efa97b898b8310aa81b280dad2a5f6ab9d228c))
- **lsp:** thread precise edits into span mapping ([`09101b9`](https://github.com/jolars/arity/commit/09101b9788a2d5c72379d007ffc2bb98efa6db96))
- **lsp:** thread precise edits into incremental reparse ([`78ce7d1`](https://github.com/jolars/arity/commit/78ce7d17cdf00fb385012cd639f907e1a78e64f7))
- **lsp:** use incremental text document sync ([`0e4b9ce`](https://github.com/jolars/arity/commit/0e4b9ce828c138b08177c3083a967658b462190d))
- **linter:** cut undefined-symbol FPs from opaque binders ([`e1e5264`](https://github.com/jolars/arity/commit/e1e5264454ac757a5798f2241f307475534cd578))
- **linter:** add `coalesce` rule ([`b7ac1d9`](https://github.com/jolars/arity/commit/b7ac1d93ba3a7cdcbd24142ed7153d0b2a9824e3))
- **linter:** add browser rule ([`fe7a5ba`](https://github.com/jolars/arity/commit/fe7a5ba78216d62c053d97cd5e64db2d26a2cb0b))
- **linter:** add if-always-true rule ([`96c136a`](https://github.com/jolars/arity/commit/96c136a30881ac8545c42d545b89975fd9ae5bb6))
- **semantic:** add per-region control-flow graph ([`a912b3c`](https://github.com/jolars/arity/commit/a912b3cf2eaae0713cd96f969961a140d205b12b))
- **lsp:** sharpen references/rename off def-use edges ([`6204c54`](https://github.com/jolars/arity/commit/6204c54493f808feade25380b0b48e8405df246b))
- **format:** cache already-formatted files for --check ([`acc46fc`](https://github.com/jolars/arity/commit/acc46fcdb1bf1d075b98692f9c33708859db1b9f))
- **lsp:** content-derived pull resultId ([`f7ecb99`](https://github.com/jolars/arity/commit/f7ecb992790e8167b8fd128df38c819f3f84a1ec))
- **lsp:** negotiate positionEncoding (UTF-8) ([`7a9af95`](https://github.com/jolars/arity/commit/7a9af95f7441ce2f3baebc4aa5362a40e6bc84e3))
- **lsp:** work-done progress for background jobs ([`12fa210`](https://github.com/jolars/arity/commit/12fa2106000b8c0d85894e941f8286b3fcf997c4))
- **lsp:** static `$`/`@` completion + label details ([`a5b72b8`](https://github.com/jolars/arity/commit/a5b72b8839c63beb926719eefdc857d4c4445627))

### Bug Fixes
- **npm:** fall back to musl when glibc build fails ([`f45beb7`](https://github.com/jolars/arity/commit/f45beb75e17ef2ecedfcd2fc23e533d0f45a13ea))
- **linter:** withhold unused-binding fix on chain ([`b3d42f6`](https://github.com/jolars/arity/commit/b3d42f6a29decafac8280d0360b3a539eedfa9c9))
- **linter:** cut unused-binding FPs from lazy reads ([`17dbaf8`](https://github.com/jolars/arity/commit/17dbaf88a881436e6f540bf97b7dacbf877991d1))
- **linter:** skip non-UTF-8 files instead of aborting ([`89bacc4`](https://github.com/jolars/arity/commit/89bacc42f77629cd929c254ae75b642b148bdc32))
- **formatter:** handle mid-line roxygen as trailing comment ([`41140fa`](https://github.com/jolars/arity/commit/41140fa8556a4e44eff427473aa1312cb6f8899a))
- **formatter:** flatten binary chains with lhs-trailing comment ([`ab99d52`](https://github.com/jolars/arity/commit/ab99d52994826ede43e953718ae41d4299fae9f1))
- **formatter:** handle roxygen comment on assignment rhs ([`0e29c0e`](https://github.com/jolars/arity/commit/0e29c0e610fedbf7329d373db408cb6b6a0ae30e)), closes [#89](https://github.com/jolars/arity/issues/89)
- **formatter:** handle comment trailing binary lhs ([`6953947`](https://github.com/jolars/arity/commit/6953947140e65ef60dcc219a604ff72aab5be31d))
- **parser:** left-associate extract ops with postfix ([`29c7416`](https://github.com/jolars/arity/commit/29c74166e019bc1e33eeb7a50715823be7419796))
- **linter:** resolve for-body reads in enclosing frame ([`023cc57`](https://github.com/jolars/arity/commit/023cc5703282e6050db7d4055094ad0864fe9c0c))
- **linter:** close three undefined-symbol gaps ([`9b4634d`](https://github.com/jolars/arity/commit/9b4634d483f190385ce5a3537010a5b77aa3577f))
- **parser:** group loose roxygen in arg lists ([`f22b0e6`](https://github.com/jolars/arity/commit/f22b0e6234d816d905dda122e57f68dc1548373f))
- **parser:** continue exprs across newlines in brackets ([`c603475`](https://github.com/jolars/arity/commit/c603475ddd48fa52d08048f8b9cb18ffbd45323b))
- **lexer:** accept all raw string delimiter forms ([`178dca1`](https://github.com/jolars/arity/commit/178dca184b6878cb70db29979b793c652efade76))
- **parser:** continue expr across trailing comment in brackets ([`31b23ea`](https://github.com/jolars/arity/commit/31b23eaeeac938aac3f37ec9f5c85d4618d479ce))

## [0.14.0](https://github.com/jolars/arity/compare/v0.13.0...v0.14.0) (2026-07-30)

### Features
- **semantic:** add def-use reverse index ([`89c8ab4`](https://github.com/jolars/arity/commit/89c8ab48846394722da018d2a53253fda91a38df))
- **lsp:** react to on-disk changes via didChangeWatchedFiles ([`3fa343b`](https://github.com/jolars/arity/commit/3fa343b5f42f7bdd48fe685cd853ae4ec419799d))
- **ci:** add smoke-test-triage skill, sharpen corpus scan (#80) ([`bb8906e`](https://github.com/jolars/arity/commit/bb8906e7459621857c606b5cdd1bd2ee5b2bfdad))
- **lsp:** add request cancellation and stale-read gating ([`9f0d51a`](https://github.com/jolars/arity/commit/9f0d51a28782d9e3d6f28f8fa4b91131c322d474))
- **lsp:** guard threads against handler panics ([`1b0e329`](https://github.com/jolars/arity/commit/1b0e3292ffe5fcb36160e7f71b4acee0085d6c3c))
- **formatter:** treat trailing comments as zero-width ([`cd38a47`](https://github.com/jolars/arity/commit/cd38a473d0a1bf9284a6c9733c1c5be11720866d))
- **roxygen:** merge same-`@name` blocks into one topic ([`31df16a`](https://github.com/jolars/arity/commit/31df16ac9b45940bc39c5eed53e1038d7b04c378))
- **roxygen:** drop blocks past the last top-level expr ([`a7a264b`](https://github.com/jolars/arity/commit/a7a264bc9593d6fb30dc41b2345213b042cee8ed))
- **roxygen:** knitr chunk fence class carries the language ([`85fbb37`](https://github.com/jolars/arity/commit/85fbb37102e5897b6bc4cb218c70384fd398e574))
- **roxygen:** newline ends unquoted html attribute value ([`af07249`](https://github.com/jolars/arity/commit/af07249ad3e2e3ef80eb64e9291b6f066d1c0c81))
- **roxygen:** demote md image after odd backslash run ([`579414b`](https://github.com/jolars/arity/commit/579414b7918a7a95a52056f56b70c370e80f355f))
- **roxygen:** inline-link destination crosses soft breaks ([`78b7f79`](https://github.com/jolars/arity/commit/78b7f793828316e59d346ef9bac7d9e177710024))
- **roxygen:** raw fence info string can drop the section ([`ef21a51`](https://github.com/jolars/arity/commit/ef21a51006212fa1c76f5bef788cb66ce1fa77cd))
- **roxygen:** demote md macro after odd backslash run ([`85cc88d`](https://github.com/jolars/arity/commit/85cc88d086680952cf21b4e759ef1a55b8204ea0))
- **roxygen:** block-level reparse of leaked linkref defs ([`ab90044`](https://github.com/jolars/arity/commit/ab9004470b613826ebe40fb9fdd885d97da3e9f5))
- **roxygen:** escaped-close link labels and markdown leak parsing ([`41df574`](https://github.com/jolars/arity/commit/41df574b3f49b8e074eb016893cbee82a6b1aa6d))
- **roxygen:** link-ref defs inside block quotes ([`27dbbe3`](https://github.com/jolars/arity/commit/27dbbe3f35400a1612eb11a2f45da0fb18f270fb))
- **roxygen:** same-line HTML block in a list item ([`122b450`](https://github.com/jolars/arity/commit/122b4504c7c6ed2acafbffbd163d7f5d1b679765))
- **roxygen:** field-edge Unicode trim and fence-info entities ([`2fdc85c`](https://github.com/jolars/arity/commit/2fdc85cbf745842c9db29d258498fdcc7fe98062))
- **roxygen:** setext column gate and per-piece rdComplete drop ([`8e095f5`](https://github.com/jolars/arity/commit/8e095f52d3b5f2d6ab153bde33659969d5c5adf6))
- **roxygen:** thematic-break block edges ([`c6c011d`](https://github.com/jolars/arity/commit/c6c011d36c018690a76c502650e9ddb1debfb708))
- **roxygen:** code-span backtick runs and `\verb` fallback ([`965049e`](https://github.com/jolars/arity/commit/965049e2fa016dd546e95fd58fe384138ec5d28b))
- **roxygen:** consume link-ref defs inside list items ([`25400f6`](https://github.com/jolars/arity/commit/25400f6d86b905dd28ba16e641f5716a1bfbe70e))
- **parser:** same-line code fence in a list item ([`e51a6c9`](https://github.com/jolars/arity/commit/e51a6c9bf4b9f376cfdafb2ed45e692d169f951b))
- **roxygen:** strip setext-title link defs, field-wide refmap ([`3f197dd`](https://github.com/jolars/arity/commit/3f197ddec820495d9324d9c5eac5ca53a21ad758))
- **roxygen:** model roxygen2's trailing-empty-heading raw fallback ([`4e2bb91`](https://github.com/jolars/arity/commit/4e2bb91095d518e716ee593dc309ec9e79e9088b))
- **parser:** expand tabs to 4-column stops in roxygen markdown ([`df95b3a`](https://github.com/jolars/arity/commit/df95b3a51410210d985d3506c89e5252ff93af1c))
- **parser:** headings inside list items with in-list H1 hoist ([`82e8c32`](https://github.com/jolars/arity/commit/82e8c32fda265b3eed94285d64b949a5b5b72fc6))
- **parser:** block quote opening on a list-marker line ([`6fb3838`](https://github.com/jolars/arity/commit/6fb3838c77090e7109ee576a3eae7c26fba348b7))
- **parser:** list-item content-indent start conditions ([`56c1aec`](https://github.com/jolars/arity/commit/56c1aeca51cd8234f6bccc9c4a0555b6cf1177f4))
- **parser:** nest same-line consecutive list markers ([`531664b`](https://github.com/jolars/arity/commit/531664bf3b7cdcfca4a40cd774317372cfd024f8))
- **parser:** list siblings pair by CommonMark indent window ([`aa8f33f`](https://github.com/jolars/arity/commit/aa8f33fc7361847673c08c98dec2f6cd334bf083))
- **lint:** add `sort` rule ([`de7de18`](https://github.com/jolars/arity/commit/de7de18b518b69571fd34795ae39f3cf688de042))

### Bug Fixes
- **formatter:** don't let a paren's trailing comment eat the ')' ([`5df242d`](https://github.com/jolars/arity/commit/5df242dd3c864062c23808da14a8315bdbbc669d))
- **formatter:** don't let an if-branch trailing comment force a break ([`86846c4`](https://github.com/jolars/arity/commit/86846c45cbd09a96706ef76d853d8e470cb5419b)), closes [#70](https://github.com/jolars/arity/issues/70)
- **parser:** don't bind an operator to a comment atom ([`4856f9d`](https://github.com/jolars/arity/commit/4856f9d830a591ff6c729c548fb1c22c6d4a05b2)), closes [#71](https://github.com/jolars/arity/issues/71)
- **formatter:** brace both if branches when a comment braces one ([`d65d573`](https://github.com/jolars/arity/commit/d65d573c763560dac850c52d1db41ea83db9b4ac)), closes [#73](https://github.com/jolars/arity/issues/73)
- **formatter:** lay out a comment before an assignment RHS ([`0f722b3`](https://github.com/jolars/arity/commit/0f722b3658f7bff5761393af67145b37b0660afb))
- **formatter:** measure breaks at the real column (#82) ([`82e83fe`](https://github.com/jolars/arity/commit/82e83fe8b15618cd10acbe5de968fc9c8808320d)), closes [#67](https://github.com/jolars/arity/issues/67)
- **config:** tolerate a missing anchor in discovery (#81) ([`c627e2f`](https://github.com/jolars/arity/commit/c627e2f3cbc4ab0331241efcea42f9bb5433a4ec))
- **parser:** reject juxtaposed statements, fix two lexer gaps (#79) ([`e98b877`](https://github.com/jolars/arity/commit/e98b8770a9e5189157a5813c767f1b64417cf1fb)), closes [#68](https://github.com/jolars/arity/issues/68)
- guard force-exclude paths by `has_root` ([`0a87100`](https://github.com/jolars/arity/commit/0a871002604c1b70e277ad6c397eddfd36109978))

## [0.13.0](https://github.com/jolars/arity/compare/v0.12.0...v0.13.0) (2026-07-20)

### Features
- **cli:** add `--force-exclude` for explicit paths ([`0b600ca`](https://github.com/jolars/arity/commit/0b600cadfbc9913c2178937b0162f457e6f50574))
- **parser:** link-reference definitions parse at the block level ([`86b4a9d`](https://github.com/jolars/arity/commit/86b4a9d54638a55b592bf3db442e5c21c7f13143))
- **parser:** reference-image resolution parity ([`3504005`](https://github.com/jolars/arity/commit/3504005432de9949bcdee0b54811353e87334f97))
- **parser:** autolink span wins over the bracket carve ([`787bcbb`](https://github.com/jolars/arity/commit/787bcbbeff79ee519b100b8261799e94ac985d69))
- **parser:** refmap-aware reference-link chain pairing ([`bf519eb`](https://github.com/jolars/arity/commit/bf519ebff63f6d899cdcf9ff12e7350d25a4df6c))
- **parser:** escaped open bracket is link-label content ([`7a03ad1`](https://github.com/jolars/arity/commit/7a03ad1076acafdf517d3db24588d28f64a39a7c))
- **parser:** invalid link-ref labels never define or link ([`ebc4dd4`](https://github.com/jolars/arity/commit/ebc4dd4761f051c4795f010d70fe2fc3fb14fec9))
- **parser:** cmark-parity link-ref label normalization ([`e0893d2`](https://github.com/jolars/arity/commit/e0893d23382288b9888b1dd4c6879d123dc899fd))
- **parser:** cmark-parity inline link destinations ([`27f7b5e`](https://github.com/jolars/arity/commit/27f7b5eda483f0dc9fb2b6c560fb4441ddf31ad6))
- **parser:** block quote laziness state and xml_text flatten ([`e5f03aa`](https://github.com/jolars/arity/commit/e5f03aa01018b681e492e7da645c0fa2998f4808))
- **parser:** block quote folds into a markdown list item ([`3a37044`](https://github.com/jolars/arity/commit/3a37044ae1226d796842ac8f418759951a24ca0d))
- **parser:** collapsed reference links under roxygen @md ([`6bc059e`](https://github.com/jolars/arity/commit/6bc059eca11ee3acbb2fda0f25ebf31c419c6aac))
- **parser:** tilde code fences and CommonMark closer matching ([`06b0b46`](https://github.com/jolars/arity/commit/06b0b46ea0e47be1cd3502cd0f4c6f07ceea692e))

## [0.12.0](https://github.com/jolars/arity/compare/v0.11.0...v0.12.0) (2026-07-11)

### Features
- **lint:** add `is-numeric` and `class-equals` rules ([`b3c8443`](https://github.com/jolars/arity/commit/b3c84434ac7568e92381707fc6c12e1c7b261ad9))
- **lint:** add `seq` rule ([`d3cf745`](https://github.com/jolars/arity/commit/d3cf74576e01da7d25c03b6eb1f39ae0c5a6c202))
- **lint:** add `nzchar` rule ([`e903216`](https://github.com/jolars/arity/commit/e903216f7eb164660401a40d0d262a90d1c34e58))
- **lint:** add `lengths` rule ([`94b8f67`](https://github.com/jolars/arity/commit/94b8f677c6a984f7c6bb512cccfa1cc160a56a9a))
- **bench:** add linter and project benchmarks ([`9852172`](https://github.com/jolars/arity/commit/98521720335845c2a9e45b1f3ec67a5754147a6c))
- **roxygen:** split fenced code body into per-line VERBs ([`5f27595`](https://github.com/jolars/arity/commit/5f27595f9763d4cb062a52113eb65ba1942741d8))
- **parser:** resolve user-defined markdown image refs ([`aa59fd4`](https://github.com/jolars/arity/commit/aa59fd4e3bd1c277ff417456552fd58678ea0bf9))
- **docs:** add benchmark page with plots ([`e17bcf9`](https://github.com/jolars/arity/commit/e17bcf9592948d259fa72f7cfcd273470138f511))
- **parser:** resolve shortcut and reference markdown images ([`9c0f625`](https://github.com/jolars/arity/commit/9c0f6251ab6862443abf0cf19156b03d275613e4))
- **parser:** drop section on trailing-backslash link dest ([`355f6c8`](https://github.com/jolars/arity/commit/355f6c8b4b6501857fa5dd3388e9bed688b1dbde))
- **parser:** reject invalid inline link destinations ([`3054434`](https://github.com/jolars/arity/commit/30544340cd9d7c28b1395a7e00c354b6fc6b2889))
- **roxygen:** drop inline link title from href destination ([`b34e06d`](https://github.com/jolars/arity/commit/b34e06d7f32aa47352af8163142098528f4a12c1))
- **parser:** fold setext underline into block quote lazily ([`2d40e13`](https://github.com/jolars/arity/commit/2d40e1331588c4bc5bdb644f592a7830ac849b00))
- **parser:** fold block Rd macro into list item under `@md` ([`27caa31`](https://github.com/jolars/arity/commit/27caa31ca65aa236297588afbade01a25b181226))
- **parser:** fold GFM table into list item under `@md` ([`5cb6589`](https://github.com/jolars/arity/commit/5cb6589a675f3ff6344ca2b5e879c8b70a5eba6f))
- **rindex:** harvest lazy-data symbols ([`5f35fab`](https://github.com/jolars/arity/commit/5f35fabfa72713a1a285f82ca5e5bcd77c5cee30))
- **ast:** add Expr union and HasArgList trait ([`6c11cf5`](https://github.com/jolars/arity/commit/6c11cf5926754a1f8ccb82c0b2c88b95eed84683))
- **ast:** add AstToken layer and complete node wrappers ([`9167928`](https://github.com/jolars/arity/commit/9167928a6b6a3c5ac844c65dbfcec095cc42eaf6))
- **parser:** fold indented code block into list item ([`c22ae83`](https://github.com/jolars/arity/commit/c22ae834fbc3e4f3c4b253a9fa1196f619a59eb7))
- **parser:** fold fenced code block into list item ([`bf274b3`](https://github.com/jolars/arity/commit/bf274b37e4a436d242bb367a88eb9e6d18d43455))

### Bug Fixes
- **parser:** make operator lexing UTF-8-safe ([`2e5e9d9`](https://github.com/jolars/arity/commit/2e5e9d99fdffdaf59b48bb5d625e25ad9d8d1ea7))
- **roxygen:** don't markdown-process code-tag bodies under @md ([`ceb0dba`](https://github.com/jolars/arity/commit/ceb0dba9e150f63693d7f3dbbbf5ef5612107624))
- **parser:** flag stray closing delimiter at top level ([`51f3023`](https://github.com/jolars/arity/commit/51f30235f06c6eea7ad3addb592f0185b2b927dc))
- **formatter:** honor line-ending in format_range ([`5800f16`](https://github.com/jolars/arity/commit/5800f1677a6609aed47f2673c1d6c6fd4f680257))

### Performance Improvements
- use mimalloc as the global allocator ([`fb60bf0`](https://github.com/jolars/arity/commit/fb60bf0b7aa2030c608578ccd257bb7527754240))
- **lint:** parallelize project lint over salsa db clones ([`820ca43`](https://github.com/jolars/arity/commit/820ca43be869dfc2fc6ab5723a3c3e688d538dce))
- **rindex:** load lint index names-only and in parallel ([`915bd80`](https://github.com/jolars/arity/commit/915bd807f2d38d100de1db26a075ac73a97e1605))
- **semantic:** index scope bindings by name ([`bccb478`](https://github.com/jolars/arity/commit/bccb4780d08ae9fddceada97d255b1311b1d2538))
- **project:** bucket top-level events in one model pass ([`c9f668a`](https://github.com/jolars/arity/commit/c9f668a6d0c1f78d812b467a11014254a877321f))
- **lint:** render pretty snippets from a line window ([`11a4558`](https://github.com/jolars/arity/commit/11a4558c530ca47ebaa3100e44fd8981791ae991))

## [0.11.0](https://github.com/jolars/arity/compare/v0.10.0...v0.11.0) (2026-07-08)

### Features
- **linter:** add string-boundary and fixed-regex rules ([`bddd07b`](https://github.com/jolars/arity/commit/bddd07bee82b47e531e5f8e7d2161098cafd3282))
- **linter:** add crossprod rule for %*% with t() ([`de48640`](https://github.com/jolars/arity/commit/de486400a234f5094508010629e08ca36fcac1c5))
- **parser:** fold blank-separated list item paragraphs ([`92772ce`](https://github.com/jolars/arity/commit/92772ce78c18051b5a394dfe95ff373056a5a1a7))
- **parser:** split roxygen md lists on no-blank marker-type change ([`f65195d`](https://github.com/jolars/arity/commit/f65195d9223d9b3494e7ae600fa8d9944a073273))
- **lsp:** add document color support ([`3f8b048`](https://github.com/jolars/arity/commit/3f8b0486526ceea67d67b902ef30636bfa13a493))
- **lsp:** add selection ranges ([`43dd077`](https://github.com/jolars/arity/commit/43dd07717152161aefab34097cf68b051dd43681))
- **lsp:** add roxygen skeleton code action ([`8cac459`](https://github.com/jolars/arity/commit/8cac459d0b16d8a26b321a003b0ac7b792a76b74))
- **lsp:** add type hierarchy for S4/R6/RefClass ([`b4c3133`](https://github.com/jolars/arity/commit/b4c313363e5f06c13540998f34b654c8f0a2f1c3))
- **lsp:** add document link support ([`b0f5a92`](https://github.com/jolars/arity/commit/b0f5a92911cdffa443f821e1fe3c89e47c4253be))
- **roxygen:** project sticky brace-less RCODE/VERB swallow ([`917fca5`](https://github.com/jolars/arity/commit/917fca558cb2294aa8c878b328d7010cc6a1203f))
- **roxygen:** project brace-less `\item` as UNKNOWN node ([`0b3f1cf`](https://github.com/jolars/arity/commit/0b3f1cf5a9a2dfee85408bda426b486df4bfba74))
- **roxygen:** group bare braces in md heading titles as LIST ([`a479f00`](https://github.com/jolars/arity/commit/a479f0057ce5b6726ce1b87ba0db2035d890ca38))
- **roxygen:** group bare braces in macro args as Rd LIST ([`1143364`](https://github.com/jolars/arity/commit/1143364985f7671d502279f687cf0d89c54dea9f))
- **config:** honor excludes in LSP, sibling, and index walks ([`66e7680`](https://github.com/jolars/arity/commit/66e76805f4659a918e050b251e0595e4e46c0d16))
- **parser:** reparse non-braced top-level statements ([`81124a5`](https://github.com/jolars/arity/commit/81124a53c199d726bb73d2af68ab6009c20ef36f))
- **roxygen:** project md bare brace groups as Rd LIST ([`9edfcfa`](https://github.com/jolars/arity/commit/9edfcfa4ec97485c90275361e8ffa575c6b0ebc5))
- **roxygen:** project bare prose brace groups as Rd LIST ([`0e2cbd7`](https://github.com/jolars/arity/commit/0e2cbd76e230a21fe3bee18430e1ccf27820fce7))
- **parser:** gate in-arg macro carve on backslash parity ([`b5fa27f`](https://github.com/jolars/arity/commit/b5fa27f4e35ad2a429ff4842073da8ea083b2a44))
- **roxygen:** resolve Rd-string escapes in macro args ([`8d08c7a`](https://github.com/jolars/arity/commit/8d08c7ab8a423b52327f6676d6d4cf84439b48af))
- **roxygen:** render md escaped braces bare in TEXT ([`146da14`](https://github.com/jolars/arity/commit/146da14cb17669e0852bdd47d47f03dafb6e6da6))
- **roxygen:** drop brace-less known Rd macros in projection ([`3eba086`](https://github.com/jolars/arity/commit/3eba086499f11536868833afe00d359c7da9c050))
- **parser:** parity-gate Rd macro carves on backslash runs ([`ba1a9e0`](https://github.com/jolars/arity/commit/ba1a9e0bc93f56497774450d915994d2b8da7793))
- **parser:** resolve multi-line code spans in the inline pass ([`c3575f5`](https://github.com/jolars/arity/commit/c3575f552777ab01ef2068ce41c44eb98d001e3e))
- **parser:** resolve multi-line inline HTML in the inline pass ([`62ca1f3`](https://github.com/jolars/arity/commit/62ca1f34e78d3bc8a26f40a0a675b41bd04208af))
- **parser:** merge blank-separated same-type markdown lists ([`164d519`](https://github.com/jolars/arity/commit/164d51922e70f181bf6ea3c8a8005b697cbcc783))
- **parser:** fold lazy continuations into markdown list items ([`54f56f9`](https://github.com/jolars/arity/commit/54f56f9695a366c693936b387ac17e07a0e7c017))
- **linter:** add roxygen documentation rules ([`167ae38`](https://github.com/jolars/arity/commit/167ae3853a1709e7f1d26e89b64bc989bd2b0742))
- **ast:** add roxygen tag and prose accessors ([`792ec6b`](https://github.com/jolars/arity/commit/792ec6b4f2cb8c9cd31a7cb9406437dac638d73d))
- **parser:** block quote, thematic break from tag value ([`ff3f647`](https://github.com/jolars/arity/commit/ff3f647bc52f82a756a91a95b3c9afe4159c6198))

### Bug Fixes
- **formatter:** indent nested operand of binary chain ([`ef14a99`](https://github.com/jolars/arity/commit/ef14a99d8755f587b7b0a3f9aa954192fcf34837))
- **roxygen:** keep md section on trailing \% swallow ([`4425971`](https://github.com/jolars/arity/commit/4425971db505387458e70b5a21b1bf21032465db))
- **roxygen:** resolve multi-backslash md brace runs at cmark stage ([`756aacd`](https://github.com/jolars/arity/commit/756aacde2c753c1a911195f8804dcb2820913794))
- **roxygen:** keep fragile-macro escaped braces in md drop scan ([`0674bc7`](https://github.com/jolars/arity/commit/0674bc7f3bb5c44c7a1670f5b0211b64392c2411))
- **roxygen:** scan raw text for md-off rdComplete drop ([`f291ab1`](https://github.com/jolars/arity/commit/f291ab13cc059a90384ae3cd1626003dd3e894d9))
- **formatter:** bail folded roxygen tags on structured lines ([`b251e40`](https://github.com/jolars/arity/commit/b251e40b5128a0bfe6d393ac6010a9fbfedc8ff7))
- **formatter:** reflow roxygen prose containing pipes ([`6b17cad`](https://github.com/jolars/arity/commit/6b17cad8dda09cec9039cea8780232b28f8df613))

## [0.10.0](https://github.com/jolars/arity/compare/v0.9.0...v0.10.0) (2026-07-05)

### Features
- **parser:** markdown block starts from same-line tag value ([`62e976c`](https://github.com/jolars/arity/commit/62e976cce1f771262a2eccbf45b4491875347c7f))
- **parser:** markdown HTML block from same-line tag value ([`f343419`](https://github.com/jolars/arity/commit/f34341964e3bf8a28e53a2bd3655cd8b3291b19d))
- **parser:** markdown HTML block condition 7 ([`1d6c837`](https://github.com/jolars/arity/commit/1d6c83728add061538a25156768769412612c6c7))
- **parser:** inline markdown HTML comment, PI, decl, CDATA ([`b1b08b9`](https://github.com/jolars/arity/commit/b1b08b91afd57755f29fc3f3f46088a56e9e1b2e))
- **parser:** markdown HTML block conditions 2-5 under @md ([`04819f4`](https://github.com/jolars/arity/commit/04819f4e74e21e86c08ec91c66b55ce1def00514))

### Bug Fixes
- **formatter:** break associative binary chains uniformly ([`2c32f0a`](https://github.com/jolars/arity/commit/2c32f0afa21e615c0080d5f9f1d7a91252e5692c))
- **lint:** sort rendered diagnostics by source offset ([`950d223`](https://github.com/jolars/arity/commit/950d2237cac77e1000d0244a0f74049bf7243fba))
- **lint:** surface parse errors instead of swallowing them ([`9afb39f`](https://github.com/jolars/arity/commit/9afb39ffc4f655696febddf9621bbbb8e96dc00d))

## [0.9.0](https://github.com/jolars/arity/compare/v0.8.0...v0.9.0) (2026-07-04)

### Features
- **parser:** markdown HTML block condition 1 under @md ([`dcc35a2`](https://github.com/jolars/arity/commit/dcc35a279e49c891a72141436ccb423b0a3c0b3f))
- **parser:** markdown indented code blocks under @md ([`9308021`](https://github.com/jolars/arity/commit/93080214f532360535948abc14d7df331ec68b63))
- **parser:** setext heading from same-line tag value ([`6725dbe`](https://github.com/jolars/arity/commit/6725dbe4046b22a14fba0543bec11c17e7c34fba))
- **parser:** fold block-quote lazy continuation lines ([`d3d5583`](https://github.com/jolars/arity/commit/d3d5583b55e4ba9f1e1394f92ee14251478ccd22))

## [0.8.0](https://github.com/jolars/arity/compare/v0.7.0...v0.8.0) (2026-07-02)

### Features
- **roxygen:** glue block-quote text onto adjacent prose ([`8410550`](https://github.com/jolars/arity/commit/8410550b8befd4b0daf427b47729cf4162b3824e))
- **parser:** recognize single-dash setext H2 underlines ([`b6a56bf`](https://github.com/jolars/arity/commit/b6a56bf03193fa3d35d3cd77c1f7b6bce079d987))
- **parser:** recognize `@md` thematic breaks ([`176cd31`](https://github.com/jolars/arity/commit/176cd311f3e0da8865a929a7f7c5527f480a623e))
- **parser:** recognize `@md` block quotes ([`f1b02aa`](https://github.com/jolars/arity/commit/f1b02aa779a30243f45be79e2f6f4c8f595d4825))
- **parser:** recognize `@md` setext headings ([`b630a8e`](https://github.com/jolars/arity/commit/b630a8e213517eecc23025708a0519270d3ecd95))
- **parser:** hoist @md ATX headings into \section/\subsection ([`55bf690`](https://github.com/jolars/arity/commit/55bf690459fa46d272038c17845bfa7a5414655e))
- **parser:** recognize GFM tables in @md prose ([`89d9ec4`](https://github.com/jolars/arity/commit/89d9ec4bbfc6df32e397aa3ae42101c58fce2043))
- **parser:** recognize CommonMark email autolinks ([`bb988c8`](https://github.com/jolars/arity/commit/bb988c8e3da6b09a5bb6831af7ac6651adccb07c))
- **parser:** reject link-ref def with trailing content ([`1a728f5`](https://github.com/jolars/arity/commit/1a728f5a984f7958d65bfe52919cb287a5b3a951))
- **parser:** decode HTML entities in @md prose ([`9c8828e`](https://github.com/jolars/arity/commit/9c8828e250bcf09865d352c1b6373de6894e863b))
- **parser:** fold prose tag continuation into the tag ([`7179469`](https://github.com/jolars/arity/commit/7179469ddbc546670051a3de9733e6dda8914377))
- **parser:** swallow @md prose percent comments ([`4c03eea`](https://github.com/jolars/arity/commit/4c03eea172f46fcf0659d2b98e64eb162333cc74))
- **formatter:** canonicalize roxygen tag layout by class ([`bc9f3cb`](https://github.com/jolars/arity/commit/bc9f3cbc4b39c96802bc73bdf8d3c9cf1c43a8ad))
- **formatter:** hang list Rd macros under prose tags ([`6b1d6a4`](https://github.com/jolars/arity/commit/6b1d6a499ea3cc99a24eda1a02c845e0fbdcbcb9))
- **parser:** collapse @md prose backslash runs ([`a0cf8a7`](https://github.com/jolars/arity/commit/a0cf8a7b08d2b07ad743aee3521a77c71b8b2612))
- **parser:** emphasis span crosses inline Rd macro ([`b282810`](https://github.com/jolars/arity/commit/b282810f8ba7ac488ff6c622afad6ef373971187))
- **parser:** emphasis across nested macro in structural md arg ([`ab7ab90`](https://github.com/jolars/arity/commit/ab7ab90f833bd8f1901d9e181a20dbd1cf3f2c4b))
- **parser:** markdown in structural Rd macro args ([`7808c50`](https://github.com/jolars/arity/commit/7808c505ec52d502d2bb5d51d4e41b1f8609e789))
- **parser:** pure-macro link displays drop/keep, not literal `[]` ([`65eea34`](https://github.com/jolars/arity/commit/65eea34738473510cee7718ae710cadf49f71811))
- **parser:** markdown in non-fragile Rd macro args ([`e337667`](https://github.com/jolars/arity/commit/e3376677c3e2fa3c3d6ea9e83a07caab00bedd42))
- **parser:** backslash words in markdown link displays ([`e303a9d`](https://github.com/jolars/arity/commit/e303a9d72411c8c220c4f058c744b1a56b6c5bca))
- **roxygen:** multi-line and entity-decoded link-ref defs ([`02e1a15`](https://github.com/jolars/arity/commit/02e1a15a6aff5884a10ef1cc7328a3d3c1d1edf0))
- **roxygen:** whole-field link-ref poisoning demotion ([`5d43db3`](https://github.com/jolars/arity/commit/5d43db3aa1af33097e974049ed2cf93db7da3dd9))
- **roxygen:** whole-field refmap for in-list undefined links ([`a31927c`](https://github.com/jolars/arity/commit/a31927c031a299bb12742b086398bcdf5ca28a80))
- **roxygen:** resolve user link-refs across list items ([`798783a`](https://github.com/jolars/arity/commit/798783a9790ece6d09448f65bcc9aa74ec807c85))
- **roxygen:** run full link-ref pipeline in @section arm ([`98be39b`](https://github.com/jolars/arity/commit/98be39bf5ed6ea4569391aa1b8161d8eebd9d608))
- **roxygen:** resolve user link-reference definitions ([`6bcb008`](https://github.com/jolars/arity/commit/6bcb0081d1bebbcc3dce5d7c4ef5c9b1443e1d00))
- **roxygen:** nodeify same-line markup reference links ([`c5e3fc3`](https://github.com/jolars/arity/commit/c5e3fc36987b0b3a814a68c06808945fa07acd21))
- **roxygen:** drop shortcut links with non-plain display ([`e56d66b`](https://github.com/jolars/arity/commit/e56d66b29319ee35fe847206abfb101c5110fbc3))
- **roxygen:** demote undefined-label shortcut/ref links ([`4d52240`](https://github.com/jolars/arity/commit/4d52240d942de7cd64e16354e5ebf737b3ca82a1))
- **parser:** drop `@section` to NA on md-off brace imbalance ([`30669fc`](https://github.com/jolars/arity/commit/30669fc6c1bbea1e10e4d4a15b218a56ff5ab1f9))
- **parser:** drop `@field`/`@slot` tag on raw brace imbalance ([`8c8c0e5`](https://github.com/jolars/arity/commit/8c8c0e583b6455231396cda7980eb9abde828dfe))
- **parser:** drop md-off prose sections on brace imbalance ([`497f3f7`](https://github.com/jolars/arity/commit/497f3f71526b00db64f1d22d4b7459368cd0a75d))
- **parser:** resolve nested md links with opener deactivation ([`859006d`](https://github.com/jolars/arity/commit/859006dfb608bf2a013ea577bb091357e577dade))
- **parser:** drop md sections on `rdComplete` imbalance ([`bca67f5`](https://github.com/jolars/arity/commit/bca67f572eb545a0e165a05594a7a1a637b796d1))
- **parser:** lex @rawRd body as Rd, never markdown ([`81bdf09`](https://github.com/jolars/arity/commit/81bdf094f11480950442be964229781075689a94))
- **roxygen:** leak opaque nested-bracket inline-link inner candidate ([`46a6110`](https://github.com/jolars/arity/commit/46a611050a234d37f6dab649be68b1aebad66722))
- **roxygen:** leak get_md_linkrefs defs for image alt-text ([`1c51798`](https://github.com/jolars/arity/commit/1c5179837d7162342f42179aba1ab0e1d00aaf1b))
- **roxygen:** leak get_md_linkrefs defs for inline-link candidates ([`8185a92`](https://github.com/jolars/arity/commit/8185a9264776d07d3b89ea27c9ed08d69f11828a))
- **roxygen:** leak get_md_linkrefs defs in field/section bodies ([`ee65a6c`](https://github.com/jolars/arity/commit/ee65a6ce7016b750e35ed8432da12e8fa584edd7))
- **roxygen:** model mixed valid+invalid link-ref poisoning ([`97dcd47`](https://github.com/jolars/arity/commit/97dcd47519651862e40669620e9d026742361d66))
- **roxygen:** leak get_md_linkrefs defs for escaped-close `[text\]` ([`7829072`](https://github.com/jolars/arity/commit/782907253622ff8ceeb0685074fb341b584de386))
- **parser:** resolve cross-line @md shortcut links ([`81e2747`](https://github.com/jolars/arity/commit/81e2747bfb25d4a38eb2f1e32d007d88329a37f9))
- **parser:** escaped @md brackets \[ \] are not link delimiters ([`70d95b4`](https://github.com/jolars/arity/commit/70d95b4f577b54a912ad7bc3ce3956a8fb833284))
- **parser:** resolve cross-line @md reference links ([`04325a1`](https://github.com/jolars/arity/commit/04325a12ff20c4c8ba85ffc0825a7d672c22dab3))
- **parser:** preserve Unicode whitespace in roxygen norm_ws ([`d935f05`](https://github.com/jolars/arity/commit/d935f05490a9000eda7f7555b67086da20bcfd23))
- **parser:** resolve @md inline links across soft line breaks ([`8310e0e`](https://github.com/jolars/arity/commit/8310e0e7a90386f7ae972e54d84ca8cbf3be790d))
- **parser:** resolve @md inline link text on the delimiter stack ([`188ce50`](https://github.com/jolars/arity/commit/188ce5023a47b6488067a8b7f35ec5dcad5bde4f))
- **parser:** underscore-leading code span renders \verb not \code ([`c4b3174`](https://github.com/jolars/arity/commit/c4b3174d8d731261093504ec321fc5082dd6871a))
- **parser:** empty md list item can't interrupt a paragraph ([`250eee0`](https://github.com/jolars/arity/commit/250eee046e1801514ef9bf6bef7321a3fcae3ba4))
- **parser:** resolve @md emphasis across soft line breaks ([`67e5003`](https://github.com/jolars/arity/commit/67e500332d2dbc749287f794f1bfc45859348154))
- **parser:** resolve @md emphasis via CommonMark delimiter stack ([`dbc9989`](https://github.com/jolars/arity/commit/dbc9989aefb49b30e3039397d66464e898895734))
- **roxygen:** project non-md Rd % as a line comment ([`b0b6be2`](https://github.com/jolars/arity/commit/b0b6be2018ec059b32f32777657fd136973c4aef))
- **parser:** model mid-prose \preformatted block-macro opener ([`c554c14`](https://github.com/jolars/arity/commit/c554c14057686078d7fdad32b172def36eb5c1d2))
- **roxygen:** project \preformatted as a verbatim block macro ([`aa92fe6`](https://github.com/jolars/arity/commit/aa92fe6ca4eb5167ad8a17afd5d7374eb0929874))
- **parser:** model nested markdown lists by indentation ([`b313550`](https://github.com/jolars/arity/commit/b313550902c8501ec6aa230e7d1b7bb6a05c30de))
- **parser:** model nested Rd block macros (\itemize in \enumerate) ([`bfe7e32`](https://github.com/jolars/arity/commit/bfe7e3224a84d952e858a65b9c89dcc7924897ff))
- **roxygen:** project @rawRd as bare top-level Rd nodes ([`fd76c95`](https://github.com/jolars/arity/commit/fd76c956f0b113762ed179d2b4a8f227cac39ec2))
- **parser:** model @md block raw HTML as \if{html}{\out} ([`6192663`](https://github.com/jolars/arity/commit/6192663b9a84b7a9bd27e9b2b0a9dc99e02a112f))

### Bug Fixes
- **roxygen:** honor soft-wrap line for %-swallow ([`b67b966`](https://github.com/jolars/arity/commit/b67b9668a43b87302f06bb3c8f737a793e447c3b))
- **linter:** only flag function shadows in shadowed-builtin ([`f8a30ce`](https://github.com/jolars/arity/commit/f8a30ced5da0c9de2cfce84d3df961f7c02de902))
- **linter:** exempt c() from duplicated-arguments ([`6f0db6d`](https://github.com/jolars/arity/commit/6f0db6df83a6f5f76282738e440744f7b9e30719))
- **linter:** attach testthat implicitly for test files ([`95067fa`](https://github.com/jolars/arity/commit/95067fa9c323db36ae100b47600ddaf0b78a8130))
- **linter:** keep excluded package files in scope ([`c79953a`](https://github.com/jolars/arity/commit/c79953a2ac65bd5968d42b1fd8811776c1f6a64d))
- **linter:** exempt parameters from shadowed-builtin ([`de10514`](https://github.com/jolars/arity/commit/de1051437a43bf2ce2d64433619b243181bceca8))
- **linter:** count infix operator use as a read ([`e3bc112`](https://github.com/jolars/arity/commit/e3bc112d5efcaef6f107cd28f5128526c7dd0718))
- **linter:** dont flag super-assignment as unused ([`8835944`](https://github.com/jolars/arity/commit/8835944c63de57c8a37e633ba0448a505656b1d3))
- **linter:** count pkg:::name as cross-file use ([`d72b5b4`](https://github.com/jolars/arity/commit/d72b5b48d302eb01b0938552d96d021ae8a88745))
- **linter:** dont flag builtin shadow used only in own rhs ([`22243b1`](https://github.com/jolars/arity/commit/22243b151c0bda46326d30032b07f4be6159ff55))
- **formatter:** reflow roxygen prose past inline list markers ([`5798c5e`](https://github.com/jolars/arity/commit/5798c5e553a1a0c80b989a08a315bd8c67aaf6f2))
- **formatter:** keep roxygen link-ref defs unjoined ([`877d7c6`](https://github.com/jolars/arity/commit/877d7c66ee130c437f31baeaf647373b0a9e839f))
- **formatter:** don't reflow non-md roxygen prose across % ([`584f489`](https://github.com/jolars/arity/commit/584f489461d3d2f52cd487f9a6eb5042b250365a))

## [0.7.0](https://github.com/jolars/arity/compare/v0.6.0...v0.7.0) (2026-06-24)

### Features
- **parser:** model @md inline raw HTML as \if{html}{\out} ([`a2f5cc9`](https://github.com/jolars/arity/commit/a2f5cc9790904199ad0af4154f725545558872c9))
- **roxygen:** sub-render inline-link code-span text to \verb/\code ([`585b096`](https://github.com/jolars/arity/commit/585b0963249280b0cb154ce83dedb3b42b7ee80b))
- **parser:** model @md URL autolinks and empty-dest links as \url ([`7bc2879`](https://github.com/jolars/arity/commit/7bc28793a495fa020a9d852c5f6c2e67a3cf0785))
- **parser:** model @md fenced code blocks, project to \preformatted ([`06509b3`](https://github.com/jolars/arity/commit/06509b3d19e264e0384329eed9ba6371a4c9c2d4))
- **parser:** project brace-less unknown Rd macros to UNKNOWN ([`50572be`](https://github.com/jolars/arity/commit/50572be0efd9bfa446012d09ca8eafbf305530ae))
- **roxygen:** project @section body inlines with GRP-wrap ([`95dbbfb`](https://github.com/jolars/arity/commit/95dbbfb11d1fee842e9e638b6232c46b6f1e3eab))
- **roxygen:** aggregate multiple @examples into one \examples ([`37357fb`](https://github.com/jolars/arity/commit/37357fb2fac4b378d9deb65a5fb1d920e85beebf))
- **parser:** lex Rd macro names with digits (\linkS4class) ([`856be51`](https://github.com/jolars/arity/commit/856be518330fa83e45282dac5aded25f86941238))
- **parser:** project markdown images and Rd \figure to \figure ([`0afed9a`](https://github.com/jolars/arity/commit/0afed9a652695a66e153652a2a1b4021fdd700d6))
- **roxygen:** split intro into title/description/details by paragraph ([`fb2c926`](https://github.com/jolars/arity/commit/fb2c9267aaf733795dafd3ecbbfec603a7a4be12))
- **parser:** resolve @md reference and shortcut links to \link ([`fd8b6d2`](https://github.com/jolars/arity/commit/fd8b6d234be89b91429386feb87598267987d6d6))
- **parser:** project @md inline links [text](url) to \href ([`4cd8cfa`](https://github.com/jolars/arity/commit/4cd8cfab2fb2110b5a93f69db0020ef34e3031d4))
- **roxygen:** aggregate @slot/@field into \section{Slots,Fields} ([`19f326e`](https://github.com/jolars/arity/commit/19f326e6a74fd751fdbe64069d04858a220abb0d))
- **parser:** model \href as two-arg macro with verbatim URL arg ([`a129a7a`](https://github.com/jolars/arity/commit/a129a7a0dd941c2968895592583aa015fa2a9223))
- **roxygen:** project \code body as verbatim RCODE ([`5ec9cf3`](https://github.com/jolars/arity/commit/5ec9cf333d5c3577787dd7f2eca8cc5e264078a9))
- **roxygen:** suppress sections with NULL sentinel value ([`be2d579`](https://github.com/jolars/arity/commit/be2d5791aeebd6da152ef4e6c66743fd494e7d5e))
- **roxygen:** project title as description fallback ([`e9f2471`](https://github.com/jolars/arity/commit/e9f247123359b3e25ee6497b5e0329a8392e185d))
- **parser:** model @md markdown lists as \itemize/\enumerate ([`1cf5da9`](https://github.com/jolars/arity/commit/1cf5da99468faf727b1bd2fe84e6ec1b7068d75c))
- **parser:** model @md inline markdown (emph/strong/code) ([`6e617ca`](https://github.com/jolars/arity/commit/6e617ca33b2e61f6bf1917e477ae684a01dff0a3))
- **parser:** model `\tabular{fmt}{ … \tab … \cr }` block macro ([`8cfbdbe`](https://github.com/jolars/arity/commit/8cfbdbe94fbc1f480baa8e56296bfa725064e1c6))
- **parser:** model two-arg \item{term}{def} in \describe ([`c1b30a9`](https://github.com/jolars/arity/commit/c1b30a9ec231dd1f2fa4f9eceb8c5c4c4f7d3520))
- **linter:** suppress undefined-symbol for data-masked columns ([`d1e382d`](https://github.com/jolars/arity/commit/d1e382dea2bb24ee3c9867c30f1ac1272d6920a1))
- **linter:** resolve meta-package attaches for undefined-symbol ([`d64cd31`](https://github.com/jolars/arity/commit/d64cd31be8a69224fec90da6b9465e408528b471))
- **parser:** model \itemize/\enumerate block Rd macros ([`dc8a38b`](https://github.com/jolars/arity/commit/dc8a38bbe542a3bdb028d0782642772a63c1cbe4))
- **parser:** model inline Rd macros as nested CST nodes ([`be0521b`](https://github.com/jolars/arity/commit/be0521b64f85534f07cb79843214f3c09840cac1))
- **roxygen:** bulk-pin projector-eligible harvested corpus ([`58ad5e4`](https://github.com/jolars/arity/commit/58ad5e42079e376b6bce62d2a5468b570b954f42))
- **roxygen:** add CST->Rd projector + pinned parity gate ([`7473f2f`](https://github.com/jolars/arity/commit/7473f2f997351f60153ffc3d0cf0ffbdfbdabea9))

### Bug Fixes
- **semantic:** mask operands of opaque custom infix operators ([`479c3dd`](https://github.com/jolars/arity/commit/479c3dd0719d65e5b4dff3e7095e01766654e64f))
- **parser:** model string-named call/subscript args as named args ([`4e2fdbf`](https://github.com/jolars/arity/commit/4e2fdbf79b1cdb41aff669c3c8d55e1ae9bde739))
- **formatter:** don't treat mid-prose years as list markers ([`d90e7d5`](https://github.com/jolars/arity/commit/d90e7d5c05c9b3c47e2b7bed5db4392aaa1305c9))

## [0.6.0](https://github.com/jolars/arity/compare/v0.5.0...v0.6.0) (2026-06-22)

### Features
- **cli:** add global --color, --quiet, --verbose flags ([`1bef2ae`](https://github.com/jolars/arity/commit/1bef2ae5a673533705f4e6a78cd750e8bf7d5951))
- **cli:** add completions and init subcommands ([`ea055bc`](https://github.com/jolars/arity/commit/ea055bcae6b235f78163b37923d43e799ad15137))
- **cli:** lint reads stdin, drop no-op --check flag ([`5237c95`](https://github.com/jolars/arity/commit/5237c957a4f73ab4c8fb0e2bb130b590eb0d8d2b))
- **config:** add top-level exclude/default-exclude file filtering ([`8a3c661`](https://github.com/jolars/arity/commit/8a3c6615da9352c55480f8a4dfc95e70fd691e67))
- **config:** add format.line-ending (auto/lf/crlf/native) ([`033937a`](https://github.com/jolars/arity/commit/033937a05e4b77ce5ba091cac3bbbd75fcaa3508))
- **linter:** add unreachable-code rule (after return()/stop()) ([`52eb74b`](https://github.com/jolars/arity/commit/52eb74b2381c014cd6c74009e90937307448bb56))
- **linter:** add any-duplicated rule (any(duplicated(x)) -> anyDuplicated(x) > 0) ([`e3d6348`](https://github.com/jolars/arity/commit/e3d63484fac7ca37f785e88946fac3ab658a8bfe))
- **linter:** add any-is-na rule (any(is.na(x)) -> anyNA(x)) ([`3d8590d`](https://github.com/jolars/arity/commit/3d8590d8e32e57c2a76cfb17aa3b83f1d02ea6e9))
- **linter:** add comparison-negation and outer-negation rules ([`caeda1a`](https://github.com/jolars/arity/commit/caeda1a83337dd0e8fcc412efd11abbae002fdd4))
- **roxygen:** format embedded R in @examples bodies ([`8715ad4`](https://github.com/jolars/arity/commit/8715ad4c32a8bae37c87cb4e9f3789d6052540b6))
- **roxygen:** hanging-indent reflow of tag prose ([`8e52c62`](https://github.com/jolars/arity/commit/8e52c62103ae6f7ed326e7947ab7b2250059c116))
- **roxygen:** reflow prose to line width ([`b5e92c5`](https://github.com/jolars/arity/commit/b5e92c59c239a3bd5c8c87598e7ad251e6b1926e))
- **linter:** add vector-logic rule (&/| -> &&/|| in conditions) ([`2e74063`](https://github.com/jolars/arity/commit/2e74063337ea09f77c657e51824794dadc09d80f))
- **linter:** add repeat rule (while (TRUE) -> repeat) ([`93cff91`](https://github.com/jolars/arity/commit/93cff91a15bf5964a8b77744bc40197db5eb62e8))
- **formatter:** normalize roxygen marker + single space ([`52b96f6`](https://github.com/jolars/arity/commit/52b96f67cca167a81ef8aad653d8148b6ccc47de))
- **parser:** parse roxygen doc comments into the CST ([`23fe61f`](https://github.com/jolars/arity/commit/23fe61f634398d499539ed61ed292e8413696022))
- **linter:** generate rule docs from rule metadata ([`e8ef3db`](https://github.com/jolars/arity/commit/e8ef3dba090acb6d2b9bf89fa969f829633f676e))
- **lsp:** add pull diagnostics support ([`e8da220`](https://github.com/jolars/arity/commit/e8da220f84a5210095e85d2efdc6c579d0c5b531))
- **lsp:** add call hierarchy support ([`24d52f3`](https://github.com/jolars/arity/commit/24d52f3acfa2e02c750ee18324996b1a5db77ed1))
- **lint:** add true-false-symbol rule ([`1a37cfa`](https://github.com/jolars/arity/commit/1a37cfa96be2952653c482b5bbf09b3058d039f6))

### Bug Fixes
- **formatter:** preserve trailing blank lines in roxygen examples ([`7ca4fd1`](https://github.com/jolars/arity/commit/7ca4fd17dc1fe501145fe580f3e1c7e16098baac))
- **parser:** bind unary `!` looser than comparison operators ([`a386bc4`](https://github.com/jolars/arity/commit/a386bc4489aaf3bb6f7357e09b9add8cdaadc41a))
- **formatter:** handle comments in if/while conditions ([`5ba6f02`](https://github.com/jolars/arity/commit/5ba6f02ee2af6a6e3e01f9176bef8f315089c39f)), fixes [#37](https://github.com/jolars/arity/issues/37)

## [0.5.0](https://github.com/jolars/arity/compare/v0.4.0...v0.5.0) (2026-06-18)

### Features
- **lsp:** add semantic tokens (full) ([`7f578d2`](https://github.com/jolars/arity/commit/7f578d2a159e82285edf7f52d8648987accb2cbc))
- **lsp:** rename binding-only reads instead of refusing (B2.4) ([`b4643d8`](https://github.com/jolars/arity/commit/b4643d8a6a74cc794c19947e106ca403e37c29be))
- **lsp:** add workspace/symbol fuzzy name search ([`56ba0dc`](https://github.com/jolars/arity/commit/56ba0dc8325814c15d3edf1862fd833a289e52bb))
- **lint:** wire CLI --select/--ignore flags ([`a3f7844`](https://github.com/jolars/arity/commit/a3f7844cd2102b76b58604d71b07a55579e11631))
- **lint:** add resolves_to_base namespace-confirmation helper ([`2bfb17f`](https://github.com/jolars/arity/commit/2bfb17fc54f1c2a9448d23c9aa191d141784526a))
- **lint:** add §I1 matchers and first Phase 1 rule batch ([`e39e6ba`](https://github.com/jolars/arity/commit/e39e6ba3311cb3a54ab1332c49252a46067cef0d))
- **lsp:** narrow dynamic-source refusal in cross-file rename ([`93f601b`](https://github.com/jolars/arity/commit/93f601b16fb1fc912a2a6782fbe4997e7445618c))
- **index:** add opt-in downloadable CRAN symbol sidecar ([`84da1fb`](https://github.com/jolars/arity/commit/84da1fb13ef8cf3b8cb6625e0d38634eb100975f))
- **lsp:** add signature help ([`85efe66`](https://github.com/jolars/arity/commit/85efe66f9caa5917285bb9f6a3697b532a6e4b57))
- **lsp:** add completion ([`59d01fa`](https://github.com/jolars/arity/commit/59d01fa4fa763c108b0d3af78f3dc9a3f3f66826))
- **lsp:** add folding range support ([`824ab9d`](https://github.com/jolars/arity/commit/824ab9d013106e43e0c229cb7ce38179d50d0556))
- **lsp:** add file rename support ([`81f4752`](https://github.com/jolars/arity/commit/81f47523b7429fcd0ed08886b2aea83141d4d61f))
- **lsp:** load-order resolution for cross-file rename/references ([`617ccb0`](https://github.com/jolars/arity/commit/617ccb0bb6dc36693db51e9f0ee65264a71eb9fe))
- **lsp:** scope-aware cross-file rename and references ([`1aa7f8c`](https://github.com/jolars/arity/commit/1aa7f8c01ee32f5fc656ecca358b456f87f0e0e6))
- **lsp:** cross-file symbol rename ([`3b12b12`](https://github.com/jolars/arity/commit/3b12b12f386dd5d5273fe5c461460424d60dec8b))
- **cli:** add diff print to `--check` ([`16ebdf3`](https://github.com/jolars/arity/commit/16ebdf311411fa1a867b292a43feb735e8d9d50c))

### Bug Fixes
- **lsp:** bind reader body reads to final scope (B2.4) ([`0f367d5`](https://github.com/jolars/arity/commit/0f367d5f257533282bbcf0b8934978159d2e62d0))
- **project:** classify source() path relativity host-independently ([`751406e`](https://github.com/jolars/arity/commit/751406eb44a1a4425090cb08cf250e9ef06ca4a1))
- **semantic:** don't record reserved constants as reads ([`9848162`](https://github.com/jolars/arity/commit/98481622f1e82b9429b36692b42bd460944378ef))
- **index:** probe .libPaths() for launcher-injected libraries ([`ed729f1`](https://github.com/jolars/arity/commit/ed729f1eed8489811e1a7288cecda5995ca8f765))
- **linter:** only flag shadowed-builtin on call-position reads ([`fca8ef7`](https://github.com/jolars/arity/commit/fca8ef708757db4363a17036d11e0a0ea0e14a43))
- **formatter:** don't break out `[[` unnecessarily ([`81543c8`](https://github.com/jolars/arity/commit/81543c846d92636986923003851116f0e1d4f786))
- **ci:** default cran-symbols window to 30 days ([`c581c4b`](https://github.com/jolars/arity/commit/c581c4b90939ad949db7b66e24acae8d2e4e1788))
- format trailing comments after binary and pipe operators ([`e9d41b2`](https://github.com/jolars/arity/commit/e9d41b29d42a8efb2fe4c1ecdd7449706058a8a6)), closes [#29](https://github.com/jolars/arity/issues/29) and [#30](https://github.com/jolars/arity/issues/30)
- **parser:** lex ...-prefixed names as one identifier ([`c3a42b2`](https://github.com/jolars/arity/commit/c3a42b2bbfba73d2c809df5ec97067ed272b2575))
- **parser:** lex dot-leading numeric literals ([`e5923fa`](https://github.com/jolars/arity/commit/e5923fa0a1e308e05c669846d0d747a286a9e8e8)), closes [#27](https://github.com/jolars/arity/issues/27)
- **formatter:** format := (walrus) assignment operator ([`e80a0fc`](https://github.com/jolars/arity/commit/e80a0fc25d1c579d6641abcba6d92597cf182254)), closes [#26](https://github.com/jolars/arity/issues/26), [#28](https://github.com/jolars/arity/issues/28), [#31](https://github.com/jolars/arity/issues/31), and [#32](https://github.com/jolars/arity/issues/32)
- **formatter:** break over-width if condition onto its own line ([`0b26da6`](https://github.com/jolars/arity/commit/0b26da6afa0f21794a2b49f5c33abd398dcfd77e))
- **formatter:** don't break `[[` group ([`89e3eb7`](https://github.com/jolars/arity/commit/89e3eb7b3eed1bdc828ad2dd13d7077c9bb12f30))
- **parser:** parse function parameter defaults as expressions ([`5cd36e1`](https://github.com/jolars/arity/commit/5cd36e119a06af8ab4de1d4ab63d42babc80b92d))
- **parser:** lex backtick-quoted and bare-dot names ([`2a6252a`](https://github.com/jolars/arity/commit/2a6252ab2ab1fb080453af5b0616eff4240cf0a7))
- **linter:** eliminate unused-binding false positives ([`c941fcb`](https://github.com/jolars/arity/commit/c941fcb9d638920b5dbfb1e1b19ef57f280bf922))

### Performance Improvements
- **incremental:** make workspace_project pure via PackageGraph input ([`0409a6a`](https://github.com/jolars/arity/commit/0409a6aca8515c218532c85f99a003b5d0618d2d))

## [0.4.0](https://github.com/jolars/arity/compare/v0.3.0...v0.4.0) (2026-06-12)

### Features
- add npm distribution (arity-cli) ([`ea0658b`](https://github.com/jolars/arity/commit/ea0658b3d386fd24f0df528b3784384a14e3eeaa))

## [0.3.0](https://github.com/jolars/arity/compare/v0.2.0...v0.3.0) (2026-06-12)

### Features
- add VS Code / Open VSX extension ([`4501ec4`](https://github.com/jolars/arity/commit/4501ec4119b949518874024b1b2f8fbbb19cb8e6))
- **lsp:** index default R packages for hover ([`3bf0927`](https://github.com/jolars/arity/commit/3bf0927d666ade67aca70502583b91848f703f2a))
- build man pages and completion and cli docs ([`660d947`](https://github.com/jolars/arity/commit/660d9475f246ebf75e625e1de37f955a1075df7f))

## [0.2.0](https://github.com/jolars/arity/compare/v0.1.0...v0.2.0) (2026-06-12)

### Breaking changes
- rename package to arity ([`979371b`](https://github.com/jolars/arity/commit/979371bcd97a49c96660cd0d38ee2f82b85bfa27))

### Features
- **lsp:** document symbols outline ([`b203c2f`](https://github.com/jolars/arity/commit/b203c2f0e831f04d94e8c8c9fe2e1ddde2a02f54))
- **lsp:** find-references + document highlight ([`12e7566`](https://github.com/jolars/arity/commit/12e75665dd1f94a4fce259450c6f6129dde803f9))
- **lsp:** add go-to-definitions ([`f74a087`](https://github.com/jolars/arity/commit/f74a08742e8a298d8e698644c899864b67e9f6a2))
- **lsp:** cross-edit node refs + intra-file rename ([`64533de`](https://github.com/jolars/arity/commit/64533de823c3991ac84e6241b322bad2d7d9c7e9))
- **project:** workspace-wide symbol/reference index ([`8920e6f`](https://github.com/jolars/arity/commit/8920e6f3752c7094c2747033abc64154966080fa))
- **incremental:** model the package index as a HIGH-durability salsa input ([`d04b2cb`](https://github.com/jolars/arity/commit/d04b2cb629fe2b369a43bd3ccbfb26575e69c73b))
- **symbols:** bundle top-500 CRAN export lists ([`0c2a59b`](https://github.com/jolars/arity/commit/0c2a59b8736a7d92456fe18c426364c276424334))
- **parser:** incremental token/block reparse ([`231e0f3`](https://github.com/jolars/arity/commit/231e0f37ef8ec8f7819418bec0f97e2fd244541e))
- **lsp:** wrap project scope as tracked salsa queries ([`5006a04`](https://github.com/jolars/arity/commit/5006a042236bf495dbc1e9794828a12614b6f0eb))
- **lsp:** add range formatting ([`c0dafe1`](https://github.com/jolars/arity/commit/c0dafe1694a5b39a7969c64bb88199eb2aa17430))
- **lsp:** preempt in-flight lint on a fresher edit ([`81b2e71`](https://github.com/jolars/arity/commit/81b2e71a53386636e0a1da899fe0918b2a71dddf))
- **lsp:** reuse salsa db for hover/format/code-action reads ([`e90dd68`](https://github.com/jolars/arity/commit/e90dd6893c9300d2888c3b2cee4e7acc9bd9e368))
- **lsp:** add init options ([`3e6757b`](https://github.com/jolars/arity/commit/3e6757ba7fa267d6d2a28ea241d15770411f91ea))
- **lsp:** cross-file resolution for the active document ([`1e7f046`](https://github.com/jolars/arity/commit/1e7f046570a83d8bffb0914b1d6dee7dd808e8b8))
- **linter:** honor NAMESPACE exports and imports ([`7177f47`](https://github.com/jolars/arity/commit/7177f47a40575e23157266d57f2d50d04bc03051))
- **linter:** resolve bindings across files in a project ([`44c1938`](https://github.com/jolars/arity/commit/44c1938a8f1eb1ef849e9bb0781a062ad8115c96))
- **project:** add source() edge and file-export extractors ([`cc5ee3d`](https://github.com/jolars/arity/commit/cc5ee3dd4b567339f15e769f72d2eeb16ea17def))
- **lsp:** lint off a persistent salsa database ([`54afbe1`](https://github.com/jolars/arity/commit/54afbe124d3bd8f0a4d276e73c62109264cfa640))
- **incremental:** cache parse tree + semantic model in salsa ([`8651d0c`](https://github.com/jolars/arity/commit/8651d0cd659de5156a274a4b8ebd353a48769286))
- **formatter:** hug trailing block past an unbreakable atom ([`cfc892c`](https://github.com/jolars/arity/commit/cfc892cbc491ace472561eb8aef38004a3db7097))
- **formatter:** rest-aware group fit; break inner hug on overflow ([`e493d0d`](https://github.com/jolars/arity/commit/e493d0ddf557e0142a1e0f080254ddfc720c242a))
- **formatter:** line width wins over hug; explode instead ([`579621a`](https://github.com/jolars/arity/commit/579621aba2e9ccfeb096476265a389312b33654f))
- add space after `=` in `fn(NULL =)` ([`e311b0d`](https://github.com/jolars/arity/commit/e311b0d71c8ddd6a52d80278290f39758038c90d))
- **formatter:** adopt air's always-brace for control flow ([`c583583`](https://github.com/jolars/arity/commit/c583583c0e5fca669504f0eec9f57f2a30e74ace))
- inline `{{ x }}` syntax ([`accf8a8`](https://github.com/jolars/arity/commit/accf8a81af399f623d841c71de3ab3574ba28091))
- **formatter:** pack leading argument holes inline ([`cd78c06`](https://github.com/jolars/arity/commit/cd78c061398a0e41c0c7af3cc2a2425b6a062e89))
- **lsp:** hover with indexed package help ([`e6e0b66`](https://github.com/jolars/arity/commit/e6e0b66a8e4ad58a9d609b0e16dd978e35cd671c))
- **rindex:** harvest full Rd help bodies as markdown ([`8833905`](https://github.com/jolars/arity/commit/883390557d110725b98d0c278f0e3e473189908a))
- **lsp:** load the index and lazily build missing packages ([`f1852c4`](https://github.com/jolars/arity/commit/f1852c434c03260e3519fcbc83d92936864604bf))
- **linter:** enable undefined-symbol behind an all-indexed gate ([`8a5861d`](https://github.com/jolars/arity/commit/8a5861d7901ab8c56db05f50cc1dbb594b7ad850))
- **rindex:** harvest function formals from lazy-load DBs ([`5693f7c`](https://github.com/jolars/arity/commit/5693f7c8a6e46bf17697edf763586bfe4757fda4))
- **rindex:** on-disk R-introspection sidecar (`ravel index`) ([`36c8aae`](https://github.com/jolars/arity/commit/36c8aae3a3f1d209e7cbd4809d15fc7a04d56b47))
- **linter:** autofix infrastructure, --fix, and LSP code actions ([`76dcb44`](https://github.com/jolars/arity/commit/76dcb442b73cbba6e2276f5b714181488a0ef150))
- **linter:** semantic foundation, suppression directives, and LSP diagnostics ([`2f121a2`](https://github.com/jolars/arity/commit/2f121a2e2d59fe449f8df9cdd484ffddc9892f62))
- **lsp:** add initial lsp with formatting capabilities ([`e8e2a73`](https://github.com/jolars/arity/commit/e8e2a738920cb0247cff30440efc73df230d3d0b))
- add a config ([`058b8af`](https://github.com/jolars/arity/commit/058b8afcfa311e10325a84dc9d115b865ad5d27c))
- **formatter:** support `@` slot extraction as sticky operator ([`c46b761`](https://github.com/jolars/arity/commit/c46b7618b4d79e41b8d01fc1b6274723818393e0))
- support the walrus operator ([`b2549b6`](https://github.com/jolars/arity/commit/b2549b6abbd867ba85ac2cc9ba0dbd6228e75599))
- **parser:** lex `**` as exponentiation synonym for `^` ([`fa6d90a`](https://github.com/jolars/arity/commit/fa6d90a8efc77ac29cadb77beaf79cb9ff66de38))
- **parser:** handle help operators ([`d7e9ab7`](https://github.com/jolars/arity/commit/d7e9ab72d132a17700cb2a60c06bc2f905265a2c))
- **parser:** lex imaginary suffix as complex literal ([`4732bb3`](https://github.com/jolars/arity/commit/4732bb3ddd1065d6214d7a32030c53b79b7be7f0))
- **parser:** recognize unary tilde formula operator ([`edc90bc`](https://github.com/jolars/arity/commit/edc90bcbe5d22a1e5926e96e1aeac993c944a1a4))
- **formatter:** conditional group for break-aware fits ([`410dd48`](https://github.com/jolars/arity/commit/410dd48fbc0edd418d658815cb9a74fde8d9ed23))
- **formatter:** handle nested expressions/curly-curly ([`e041594`](https://github.com/jolars/arity/commit/e0415941cde49a97ee44d8dc0b736ce0563d5943))
- **formatter:** handle magrittr pipes ([`8bb6a56`](https://github.com/jolars/arity/commit/8bb6a56d4c98eb817fe6f3a6a56c611266c18c7c))
- **cli:** format in place if file provided ([`eac1fe3`](https://github.com/jolars/arity/commit/eac1fe3e953706701c43fffc944512dfbaf9a3b8))
- **formatter:** handle binary operators ([`0bd8db6`](https://github.com/jolars/arity/commit/0bd8db6011605e06b7c0fc9a390fb50868c70cf5))
- **formatter:** handle complex comment cases ([`340ced7`](https://github.com/jolars/arity/commit/340ced7c6ce5e7bcf0627366b69461578d725b75))
- **formatter:** handle curly-curlies and whitespace ([`1c4d189`](https://github.com/jolars/arity/commit/1c4d189ad47617f16739cfc8e173e4579b53c588))
- **formatter,parser:** handle complex if-else cases ([`09beb1e`](https://github.com/jolars/arity/commit/09beb1e4e4efbb747936aa9ab817bd5d0805de62))
- **formatter:** handle repeat constructs ([`2425148`](https://github.com/jolars/arity/commit/2425148f78a2f84a87d06cf910eb51259c558ed4))
- **formatter:** handle empty functions ([`a2d04c9`](https://github.com/jolars/arity/commit/a2d04c9bb27b8f368b2a42687dc3551971aa32cd))
- **formatter:** handle some parenthesized expressions ([`27bf2fd`](https://github.com/jolars/arity/commit/27bf2fd2517b659d4bda87bb9fa084fc03f0d5d7))
- **parser,formatter:** handle comment prefixed args ([`630df1c`](https://github.com/jolars/arity/commit/630df1ce5cc78fba55343f82d3666bfbec759748))
- **formatter:** handle more complex subsetting cases ([`a1efad0`](https://github.com/jolars/arity/commit/a1efad069f8eb34336665029fe998c914c56451e))
- **formatter:** handle comment in subset ([`a6be358`](https://github.com/jolars/arity/commit/a6be35807c9cb7046f028399a7b54f14494d7777))
- **formatter:** add support for subsetting with holes ([`790f63d`](https://github.com/jolars/arity/commit/790f63d349007c02404cfe16724e298e6f332c09))
- **formatter:** handle basic subsetting and holes ([`000b3f7`](https://github.com/jolars/arity/commit/000b3f79c7e15803abe239a2a8664c0cd1bea44a))
- **formatter:** flatten simple functions ([`0881304`](https://github.com/jolars/arity/commit/0881304641de36a4070c24d0c5e61d54d30cf204))
- **formatter:** handle more function cases ([`20033bb`](https://github.com/jolars/arity/commit/20033bbaea95b016f22b1bd93dcd08f845c81fe4))
- **formatter,parser:** handle comments in function defs ([`e844b23`](https://github.com/jolars/arity/commit/e844b23a99db0347d786eceb15382e22927056db))
- **formatter:** handle lambdas ([`3aecd57`](https://github.com/jolars/arity/commit/3aecd57ea4dfcd15203a190376bf49a172ef89aa))
- **formatter:** support `[[` subsetting ([`52f32f2`](https://github.com/jolars/arity/commit/52f32f24d5d07a3feabc743a285bc31979e2f8f1))
- **formatter:** harden support around hugging and calls ([`961d748`](https://github.com/jolars/arity/commit/961d748427baa6c1650402713ce9032a8c26db13))
- **formatter:** add call hugging support ([`e92401f`](https://github.com/jolars/arity/commit/e92401f4d5d43be4d4ec50ae5127d6124791ba88))
- **formatter:** handle blanklines between function arguments ([`2398618`](https://github.com/jolars/arity/commit/23986182cd30b71ac8d8bb326cf88823af907c8b))
- **formatter:** handle curly-curly ([`3d47fe1`](https://github.com/jolars/arity/commit/3d47fe1fce1124641709d4a19b4f3f55d7ddc0eb))
- **formatter,parser:** handle empty RHS ([`978828c`](https://github.com/jolars/arity/commit/978828cf51d79f04a754551f9c619d9dd2e93b75))
- **formatter:** handle comments and continuations in calls ([`d534d66`](https://github.com/jolars/arity/commit/d534d66672fac77a559eb6e8d3e4e970f28bee77))
- **parse,formatter:** handle lambdas and trailing functions ([`5ad396b`](https://github.com/jolars/arity/commit/5ad396b2defd01b25c5f44a3da6b353aa7ea2adb))
- **formatter:** handle training braces ([`3103ac3`](https://github.com/jolars/arity/commit/3103ac3dcfa118bfc41fd7e8c0f758289c6f6b3e))
- **formatter:** handle comments after holes ([`310bde3`](https://github.com/jolars/arity/commit/310bde366bd021fb44db2fa635afd5db551b09ec))
- **formatter:** handle comments inside holes ([`17e7b12`](https://github.com/jolars/arity/commit/17e7b12246c833c7f98d57272d0ff274294c9c1c))
- **formatter:** tackle holes in calls ([`b57fc73`](https://github.com/jolars/arity/commit/b57fc73cceeec1a95b07e05ee330060156138f59))
- **formatter:** support calls and handle line breaks ([`f1ae921`](https://github.com/jolars/arity/commit/f1ae921a3572b36d696f8e4042febc827c788728))
- **formatter:** support basic calls ([`b95eeac`](https://github.com/jolars/arity/commit/b95eeac54ab06868dff913fb057122b22aaf138f))
- **formatter:** support while statements ([`4e7d3e5`](https://github.com/jolars/arity/commit/4e7d3e53cf3db287081858d559758b05eb748243))
- **formatter:** support for statements ([`0f4f836`](https://github.com/jolars/arity/commit/0f4f836ad1fe9258e3a94fd2fb69d28d7d471406))
- **formatter:** handle blanklines and functions ([`363fbd8`](https://github.com/jolars/arity/commit/363fbd8e8146560cbc195a36994b56aa39de5d87))
- **formatter:** don't add spaces for carets ([`461f329`](https://github.com/jolars/arity/commit/461f329379f1c604b4474b676b30ef9c3a73e429))
- refactor formatter ([`a7d5600`](https://github.com/jolars/arity/commit/a7d5600c9cbb155ceb3e7ddf87718817a84347b9))
- **parser:** wrap up air test parity ([`9e5ca51`](https://github.com/jolars/arity/commit/9e5ca51341189a2d829666b5b3896ea3b443bc78))
- **parser:** handle double colon namespace error ([`99c2259`](https://github.com/jolars/arity/commit/99c22593708633d957284b5c97ac3d85a6bae0db))
- **parser:** handle dots and repeats ([`84c790c`](https://github.com/jolars/arity/commit/84c790cf27e0a0b27d5d7fff4abb785f8cb69b72))
- **parser:** extend to air fixtures ([`d411968`](https://github.com/jolars/arity/commit/d411968a951866a5a77d15ef7fe15e59dd87e8ca))
- **parser:** extend to subset assignment ([`f19572a`](https://github.com/jolars/arity/commit/f19572a771bd37ebe5e8f95b04cc63544f600c8d))
- **parser:** suppor subsetting brackets ([`c80a021`](https://github.com/jolars/arity/commit/c80a021e81b5a7c9747c8f6e0c842f8b1bda0451))
- **parser:** add multiple new operators ([`c2ca22c`](https://github.com/jolars/arity/commit/c2ca22c54f548e31981470d0f4a4a27eb5f3d627))
- **parser:** add semicolon, comma ([`4f4e76f`](https://github.com/jolars/arity/commit/4f4e76f0932eb4a628ecb08fa51d0f59c5c52625))
- **parser:** add newline-sensitive boundaries ([`fd490f4`](https://github.com/jolars/arity/commit/fd490f480ae1ac961d9ad80ec341275ce6ea7607))
- **parser:** add slash, minus, colon ([`d45cb7e`](https://github.com/jolars/arity/commit/d45cb7e53dd92cf8a49644462050b4921d7d5fbb))
- **parser:** expand operator coverage ([`939667e`](https://github.com/jolars/arity/commit/939667e334a62edc97b59c9daaf8ab53fd735b26))
- **parser:** improve coverage and add operator precedence ([`085e59b`](https://github.com/jolars/arity/commit/085e59b08b797238e42074978a0008a717b9c05b))
- add a linter ([`72519c1`](https://github.com/jolars/arity/commit/72519c1bd7906f5fc822932ab6d16f5f7431e571))
- add more parsing cases ([`4982f1c`](https://github.com/jolars/arity/commit/4982f1c160ab7e8b0fd986cbe8e9fb976b06bc5f))
- lay groundwork for formatter and parser ([`76005cd`](https://github.com/jolars/arity/commit/76005cd13a533a5030c07aedc09d5851835856c6))
- **parser:** expand to handle more cases ([`2783429`](https://github.com/jolars/arity/commit/278342974bc180d7c585843481a32b8a1bb78c76))
- add cli, with parse command ([`8a2b5e9`](https://github.com/jolars/arity/commit/8a2b5e9ca25477363cbe3700dc1c144bca9774a6))
- add a minimal parser ([`1c0289c`](https://github.com/jolars/arity/commit/1c0289ccd4f5ed80343e95f135f9b7b23a770620))
- setup basic package infrastructure ([`835dad5`](https://github.com/jolars/arity/commit/835dad5c4294fd9bbf14fcc745ba1a573a811b73))

### Bug Fixes
- **ci:** avoid SIGPIPE in CRAN ranking script ([`c8703ad`](https://github.com/jolars/arity/commit/c8703adc33735216e303d259c00e725f01ade17c))
- **formatter:** render comment-free if/else as native IR ([`bd9e8c6`](https://github.com/jolars/arity/commit/bd9e8c614d4a5bcbf55547cbd29d98bbfbde3818))
- **formatter:** indent pipe RHS so broken calls nest correctly ([`af16e75`](https://github.com/jolars/arity/commit/af16e75632782ea8547ae3d74f5f2c87635264d4))
- **lsp:** handle URIs properly on windows ([`130c741`](https://github.com/jolars/arity/commit/130c74137a68e5f0202be22e98481ab704b31e13))
- **formatter:** keep else-if chains flat, brace consistently ([`e16aa86`](https://github.com/jolars/arity/commit/e16aa86761daadd1bac27011927d4adcbd27ec01))
- **parser:** skip comments around if-clause boundaries ([`da34806`](https://github.com/jolars/arity/commit/da34806e9a5f21f81c3893ab33be412373de1a22))
- **formatter:** handle comments between if-body and else ([`9336fa1`](https://github.com/jolars/arity/commit/9336fa1e8a176a27313e776db5fe146648024f76))
- **parser:** handle newline between subset args ([`c9e41d1`](https://github.com/jolars/arity/commit/c9e41d1ab1ae2add25ec45035d19be5dd629fd71))
- **parser:** newline terminates complete top-level expressions ([`23a63c3`](https://github.com/jolars/arity/commit/23a63c3710de5107ecfe89beabc41598fb43a0c9))
- properly handle CLRF ([`f147ad6`](https://github.com/jolars/arity/commit/f147ad6377ddc102d64923b6d92884beba5fd265))

### Performance Improvements
- **rindex:** parallelize package harvest across rayon ([`8852209`](https://github.com/jolars/arity/commit/885220936b8b7c3c9a0d41e2a1e110473b35585d))
