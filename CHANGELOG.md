# Changelog

## [Unreleased]

### Testing

* **error_pages:** expand tests to cover all explicit status arms ([3ac2995](https://github.com/solisoft/soli_lang/commit/3ac29956b7e3f40c2a7e7f40d03a9e03a2c0d3c0))

## [0.1.1](https://github.com/solisoft/soli_lang/compare/0.1.0...0.1.1) (2026-01-27)

### Bug Fixes
* **Add license and repository to Cargo.toml for crates.io** ([9102ea7](https://github.com/solisoft/soli_lang/commit/9102ea7b318c9e25618128abcbff7beb259357dc))

### Other
* **Merge pull request #2 from solisoft/release-please--branches--main--components--solilang** ([33929ae](https://github.com/solisoft/soli_lang/commit/33929ae85b9905c14a4073475e81e60db09fd492))
* **release 0.1.1** ([984c7b3](https://github.com/solisoft/soli_lang/commit/984c7b3142ad6cb150c771d2ac1eb71920df433b))

## [0.2.0](https://github.com/solisoft/soli_lang/compare/0.1.1...0.2.0) (2026-01-28)

### Features
* **Implement nullish coalescing operator and enhance migration collection management** ([bfdcbb2](https://github.com/solisoft/soli_lang/commit/bfdcbb2c72b3445fcf81ba5fc8b5c931b265a7d3))
* **Enhance query builder with symbol-based bind variables** ([757f032](https://github.com/solisoft/soli_lang/commit/757f0325198af1c23dc61d6b9b7b5f4ecb567a4c))

### Refactoring
* **Remove JIT compilation support and clean up related code** ([7ab85bb](https://github.com/solisoft/soli_lang/commit/7ab85bbaddedb2fb3ff051200e6178940348aed0))

### Other
* **Merge pull request #3 from solisoft/release-please--branches--main--components--solilang** ([9c8720a](https://github.com/solisoft/soli_lang/commit/9c8720aea9acce5c2dbf5153aa0c18a2cc473fcd))
* **release 0.2.0** ([f3b4cc4](https://github.com/solisoft/soli_lang/commit/f3b4cc4c04f89faa642589eac688a70e8d892e84))
* **Remove package.json and package-lock.json, update context.json with LiveView features** ([03bdb66](https://github.com/solisoft/soli_lang/commit/03bdb6686f5af9a191c088e4d6f703971e8a9df4))

## [0.3.0](https://github.com/solisoft/soli_lang/compare/0.2.0...0.3.0) (2026-01-29)

### Features
* **Refactor HiddenClass methods to accept registry parameter** ([cb69911](https://github.com/solisoft/soli_lang/commit/cb69911385997164552c32955d1a2d47dd037b48))
* **Update versioning across the project** ([da46354](https://github.com/solisoft/soli_lang/commit/da463543759333c8c3d1e916386d845a9ca8f7ba))
* **Add blob storage and retrieval functions to SoliDBClient** ([6f47f68](https://github.com/solisoft/soli_lang/commit/6f47f68c3b691aadc9a41891d85a80a85f7304df))
* **Implement test DSL and assertion tracking** ([4687e57](https://github.com/solisoft/soli_lang/commit/4687e572071c7e23d907fc778f815d473fb73c0b))
* **Add Base64 utility methods and enhance documentation** ([a754f57](https://github.com/solisoft/soli_lang/commit/a754f57ee4163149058a08e56819913a7562c02a))
* **Add chainable array, hash, and string methods to documentation** ([1883788](https://github.com/solisoft/soli_lang/commit/18837886eadd97b171ff5cc74120361c51bacbc2))

### Refactoring
* **Simplify code and improve readability in various modules** ([350b58e](https://github.com/solisoft/soli_lang/commit/350b58efb7372637749f8db1479655577ceb90e9))

### Other
* **Merge pull request #4 from solisoft/release-please--branches--main--components--solilang** ([a2f3ce4](https://github.com/solisoft/soli_lang/commit/a2f3ce415ae8c8ff06cacde63aa9b423f802a856))
* **release 0.3.0** ([2d462e9](https://github.com/solisoft/soli_lang/commit/2d462e9e498106a8695e77e87e2a275d26b8ca8c))

## [0.4.0](https://github.com/solisoft/soli_lang/compare/0.3.0...0.4.0) (2026-01-30)

### Features
* **Add coverage tracking and enhance benchmarking capabilities** ([98f19ea](https://github.com/solisoft/soli_lang/commit/98f19eafbb9a899dc7664652d629dc9f1783058f))
* **Integrate IndexMap for improved hash functionality** ([be565a1](https://github.com/solisoft/soli_lang/commit/be565a15c48454ac6c7a86547ea2940521ef717b))
* **Introduce named parameters support in function and constructor calls** ([7900827](https://github.com/solisoft/soli_lang/commit/7900827648885f904542efcb9afa0eec5fb3ab8b))
* **Update feature specifications and enhance language documentation** ([bc5fde1](https://github.com/solisoft/soli_lang/commit/bc5fde1c0262ca46bde93e927a2a063cab95baca))
* **Enhance application creation with custom template support** ([d93fa01](https://github.com/solisoft/soli_lang/commit/d93fa019ef63ae58ee03e8bb6119b84b3b4110a9))
* **Add state machine functionality and documentation** ([c070c36](https://github.com/solisoft/soli_lang/commit/c070c369cffb68131348825b9ed32ecbe91077cc))
* **Implement constant declaration support in the language** ([7272dfc](https://github.com/solisoft/soli_lang/commit/7272dfc6cc9488ee7bc91665fc7f4afd5f32234d))
* **Enhance parser to support additional statement declarations** ([307c605](https://github.com/solisoft/soli_lang/commit/307c605037f82b147d6b287715fdf716404254f4))
* **Add static block support in class declarations** ([0793297](https://github.com/solisoft/soli_lang/commit/0793297d8d5d0e479858b362c20a1e0903b20861))

### Bug Fixes
* **failing specs** ([1b0ccfa](https://github.com/solisoft/soli_lang/commit/1b0ccfaad8a518bf438337567aff7e719bda1cdf))
* **Update symbolic link for stdlib and remove state_machine.sl file** ([6dcd5df](https://github.com/solisoft/soli_lang/commit/6dcd5dfa2aa4f50bd92fd47175d6d562a60be63c))

### Other
* **Merge pull request #5 from solisoft/release-please--branches--main--components--solilang** ([06465fb](https://github.com/solisoft/soli_lang/commit/06465fb1b26ee7146249fda62dc05c22ab15b360))
* **release 0.4.0** ([791f30a](https://github.com/solisoft/soli_lang/commit/791f30acd76ed0ecbf386bdd32ef4fe4b4f04008))

## [0.5.0](https://github.com/solisoft/soli_lang/compare/0.4.0...0.5.0) (2026-01-31)

### Features
* **Add static method aliases for Duration class** ([8c7c4a2](https://github.com/solisoft/soli_lang/commit/8c7c4a28abd5f7cd5b1d2a84cf69cbb1b5ad2c6a))
* **Introduce Regex class with static methods for regex operations** ([d351a4d](https://github.com/solisoft/soli_lang/commit/d351a4d24ff3e03873f4a41355018daffcec330b))
* **Enhance documentation with nested classes and module integration** ([52c9bfa](https://github.com/solisoft/soli_lang/commit/52c9bfa250d4e39a7755f29ee505680aadd9ed75))
* **Add support for nested classes and qualified name access** ([d258260](https://github.com/solisoft/soli_lang/commit/d258260c00a4edc630daf003635f4f4c6ba70d5e))

### Refactoring
* **Standardize formatting and improve readability in built-in classes** ([514574e](https://github.com/solisoft/soli_lang/commit/514574ed2c54f3c698abd572c4720e6a5f05110f))
* **Simplify class_expr handling in TypeChecker** ([b187c0d](https://github.com/solisoft/soli_lang/commit/b187c0daa4b9932d8f94c58027898968ec12b31d))
* **Replace datetime_now() with DateTime.now() across multiple files** ([99f22ea](https://github.com/solisoft/soli_lang/commit/99f22ea7d1e996a8c73b82a969608291a6626200))

### Other
* **Merge pull request #6 from solisoft/release-please--branches--main--components--solilang** ([777d167](https://github.com/solisoft/soli_lang/commit/777d167ea8ad365d34825f047fa30d284918fd86))
* **release 0.5.0** ([695951c](https://github.com/solisoft/soli_lang/commit/695951ca8d9627c257108588564cec41401277ba))

## [0.6.0](https://github.com/solisoft/soli_lang/compare/0.5.0...0.6.0) (2026-01-31)

### Features
* **Refactor JSON handling to use JSON class methods** ([f36b066](https://github.com/solisoft/soli_lang/commit/f36b066c3d2091b1b1624c4850b199bd1768de6a))
* **Update dependencies and enhance REPL functionality** ([b240781](https://github.com/solisoft/soli_lang/commit/b240781119ab5a6119c50ef47ab003d7ee955e08))

### Other
* **Merge pull request #7 from solisoft/release-please--branches--main--components--solilang** ([30cc3e5](https://github.com/solisoft/soli_lang/commit/30cc3e5a5e651ca5223ed4797459067c5df30e1c))
* **release 0.6.0** ([2370a6a](https://github.com/solisoft/soli_lang/commit/2370a6ada37a967f6456c7dc90a8ae8de3668344))

## [0.7.0](https://github.com/solisoft/soli_lang/compare/0.6.0...0.7.0) (2026-02-05)

### Features
* **Enhance HTTP response handling and improve WebSocket connection management** ([4be70a1](https://github.com/solisoft/soli_lang/commit/4be70a1bc9bfa566f4fb3ac06daf1362f90b9a9a))
* **Implement WebSocket presence tracking and related functionalities** ([c6ba399](https://github.com/solisoft/soli_lang/commit/c6ba399cebd8a734ba16f8c702e9bf4f13831a4d))
* **Refactor REPL implementation and update dependencies** ([59cda41](https://github.com/solisoft/soli_lang/commit/59cda41a549938bbcd340c4d036521d44370e12c))

### Refactoring
* **Clean up and optimize code structure across multiple files** ([22920b2](https://github.com/solisoft/soli_lang/commit/22920b2751510f079a536f92246574fea7ffbc75))

### Other
* **Merge pull request #8 from solisoft/release-please--branches--main--components--solilang** ([7ee4cb4](https://github.com/solisoft/soli_lang/commit/7ee4cb4fc011d02e043d10b349849c43a971dd3f))
* **release 0.7.0** ([f105f23](https://github.com/solisoft/soli_lang/commit/f105f23e5fd87f1f833492a71d94887467143d65))

## [0.8.0](https://github.com/solisoft/soli_lang/compare/0.7.0...0.8.0) (2026-02-05)

### Other
* **Merge pull request #9 from solisoft/release-please--branches--main--components--solilang** ([8871999](https://github.com/solisoft/soli_lang/commit/8871999efede3cffb4e81e9d99dba24023838ca0))
* **release 0.8.0** ([138db61](https://github.com/solisoft/soli_lang/commit/138db617320803f59e8bbb483d966e883f498fea))
* **release 0.8.0** ([d2c720e](https://github.com/solisoft/soli_lang/commit/d2c720e19cce895c6f4be99a386de669b327b22a))
* **trigger release 0.8.0** ([7f4d8cb](https://github.com/solisoft/soli_lang/commit/7f4d8cb49d1a3fc53b2712fecae89c4a10dc49cb))
* **trigger release** ([b274009](https://github.com/solisoft/soli_lang/commit/b274009d270767ea20b7906f35435c7bb285f3a8))
* **remove unused publish step and outputs from CI workflow** ([5573746](https://github.com/solisoft/soli_lang/commit/55737463733d6185e5e231d7db2d976fb60bca74))
* **enable wait-for-publish in release-please action** ([7d6ef59](https://github.com/solisoft/soli_lang/commit/7d6ef59974ac1af0bcb171f5de0bc664efcbc944))

## [0.9.0](https://github.com/solisoft/soli_lang/compare/0.8.0...0.9.0) (2026-02-05)

### Features
* **introduce SOAP class for web service integration** ([a10f2bc](https://github.com/solisoft/soli_lang/commit/a10f2bc2da568b6b55b8d62a3f8151ab72ee0b5f))

### Refactoring
* **simplify attribute handling and improve array value mapping** ([9e610d2](https://github.com/solisoft/soli_lang/commit/9e610d2f430dd009f733516ec92ea2a5bd272d94))

### Other
* **Merge pull request #10 from solisoft/release-please--branches--main--components--solilang** ([75eaabc](https://github.com/solisoft/soli_lang/commit/75eaabca5e19bfec0ffe0402a1c11d684814a0b6))
* **release 0.9.0** ([974f9bc](https://github.com/solisoft/soli_lang/commit/974f9bc85b1a2634034897e04e3f6c528d35e97c))
* **update publish workflow to trigger on push events and enhance release conditions** ([14e243a](https://github.com/solisoft/soli_lang/commit/14e243a2b088c91b0d19ccf7a15a34a4b885d4c4))

## [0.9.1](https://github.com/solisoft/soli_lang/compare/0.9.0...0.9.1) (2026-04-03)

### Features
* **add CLAUDE.md to new apps, fix interface docs** ([0f1549b](https://github.com/solisoft/soli_lang/commit/0f1549b9089178f6437c9e3ba741c70a6d57d3a3))
* **add graceful shutdown with SIGTERM handler for blue-green deployments** ([83ebb21](https://github.com/solisoft/soli_lang/commit/83ebb21d0a69c9628ce13fdfdb243b9a756f7592))
* **add HTTP Range request support for video/audio streaming** ([736a34b](https://github.com/solisoft/soli_lang/commit/736a34b1bde417b7f51ba8b4e8363ff66af24be9))
* **add --version and -v flags** ([974c3c5](https://github.com/solisoft/soli_lang/commit/974c3c53082eeeeff04505132b29866bc7dee2a1))
* **add migration support to deploy command** ([9954d1f](https://github.com/solisoft/soli_lang/commit/9954d1f86c6ab09eda39aeba31e4e6311e292676))
* **add soli deploy command for blue-green deployments** ([ad5b988](https://github.com/solisoft/soli_lang/commit/ad5b98841475e07e080a0bcb3954ae35541fa999))
* **add alias_method and inherited hook for metaprogramming** ([0f922bb](https://github.com/solisoft/soli_lang/commit/0f922bb3224ee46c1d790624368b11f268875687))
* **add define_method for runtime method definition** ([5ed1591](https://github.com/solisoft/soli_lang/commit/5ed15910baddea6d82767250a32e21890b5d1e21))
* **add class_eval for metaprogramming** ([825dd49](https://github.com/solisoft/soli_lang/commit/825dd49c829a9b8abed075f9fc6d7efa0bdb746f))
* **add instance_eval for metaprogramming** ([321c6a0](https://github.com/solisoft/soli_lang/commit/321c6a00905e7c64f0bbdfdbbc9877bce0d36d11))
* **add instance_variable_set metaprogramming method** ([9159f11](https://github.com/solisoft/soli_lang/commit/9159f115b4772dd2745fcf4f89ad6a0669534cf6))
* **add metaprogramming support (respond_to?, send, method_missing, etc.)** ([bce1d30](https://github.com/solisoft/soli_lang/commit/bce1d3076b6414cf57f6af01600521625b8f67b9))
* **enhance File class with new methods and update documentation** ([d1af684](https://github.com/solisoft/soli_lang/commit/d1af684032513e9dc3921f28330b860e5a63eab9))
* **implement TOTP functionality and enhance image support** ([4bd82ea](https://github.com/solisoft/soli_lang/commit/4bd82ea24bae4623c4bdb552351c705e398d3dd3))
* **enhance array and hash methods with optimized string representation** ([e69f2bb](https://github.com/solisoft/soli_lang/commit/e69f2bbc062ed46127278df33e126d1b845ad6e2))
* **implement SoliKV-backed caching functionality** ([7a13ca4](https://github.com/solisoft/soli_lang/commit/7a13ca4a2808f3da2730d592250f944ae5d1f811))
* **introduce symbol type and related functionality** ([04dc3a6](https://github.com/solisoft/soli_lang/commit/04dc3a6fe22efe10ef2c15a3de52964bdf2fd09c))
* **add translation support for model fields** ([76f9991](https://github.com/solisoft/soli_lang/commit/76f9991c703f2c18a79dccf7ccaafc41270f27bd))
* **enhance model functionality with new query and transaction features** ([048b3e8](https://github.com/solisoft/soli_lang/commit/048b3e8e2fd1bdb9596d5ee135a46e7e526439bb))
* **enhance model instance handling and query execution** ([034be44](https://github.com/solisoft/soli_lang/commit/034be4473e63dc80bc97c8c12e44302a91d7ed75))
* **integrate regex caching and enhance regex handling** ([3d7cbf7](https://github.com/solisoft/soli_lang/commit/3d7cbf7d490e34de220cc309cae17d713ba2ec5e))
* **add 'size' method to collections and enhance string methods** ([c7c2377](https://github.com/solisoft/soli_lang/commit/c7c2377bf80ef0df461d4ad8148323e0c5c044ac))
* **enhance Model includes and select functionality** ([15b27c5](https://github.com/solisoft/soli_lang/commit/15b27c573e4eea0160cf74d81cdc5608544bcfd3))
* **add JSON performance benchmarks and enhance hash operations** ([8b408bf](https://github.com/solisoft/soli_lang/commit/8b408bf01ecf7c9872c6c7763e2d9e93e97a90a0))
* **enhance completion functionality in REPL TUI** ([91c4280](https://github.com/solisoft/soli_lang/commit/91c4280dbce38b40dbd0753b7968e4b8ffb6c240))
* **update package management and add new commands** ([85d0db4](https://github.com/solisoft/soli_lang/commit/85d0db43f733f84bae5386bfad36a7245c9ee25b))
* **introduce common REPL utilities for multiline handling** ([29bd3af](https://github.com/solisoft/soli_lang/commit/29bd3af5174adf6197c17f39636e1e5fb895ffba))
* **route output expressions through core parser; auto-call no-arg methods** ([34634e9](https://github.com/solisoft/soli_lang/commit/34634e9acc3ee6e4715ccaef9e6f66107dbf5163))
* **route <% %> code blocks through the core language parser for full language support** ([38a1d4a](https://github.com/solisoft/soli_lang/commit/38a1d4aeefb3d26a6cab09e57d4fcbf94d2fd906))
* **optimize template rendering by introducing a shared interpreter for layout and view rendering, reducing allocations and improving performance; enhance environment with data hash for efficient variable lookups** ([a31ee5c](https://github.com/solisoft/soli_lang/commit/a31ee5c68833488e0f0faa8c80b5343b1e917b70))
* ... and 36 more commits

### Bug Fixes
* **reduce SIGTERM drain to 1s to avoid blocking restarts** ([c4060be](https://github.com/solisoft/soli_lang/commit/c4060be3a1fb0158334a30aa02197942fa29874a))
* **reduce graceful drain to 5s and stop returning 503 during shutdown** ([4987c0e](https://github.com/solisoft/soli_lang/commit/4987c0eb79be2f32f10c3369266a0a649977b899))
* **remove JIT closure caching in VM that caused handler collisions** ([37a3af7](https://github.com/solisoft/soli_lang/commit/37a3af739e4107a5565d9ca07eb754f157fc3b23))
* **remove NON_OOP_CONTROLLERS cache to avoid incorrect controller classification** ([1c52d56](https://github.com/solisoft/soli_lang/commit/1c52d56a25496512cc5262d3cd37af78140c784b))
* **remove handler cache to avoid stale handler lookups** ([7a85cf5](https://github.com/solisoft/soli_lang/commit/7a85cf5b99a91a2fca551a9082a5f676b71d5662))
* **namespace NON_OOP_CONTROLLERS cache by working directory** ([80cbbd1](https://github.com/solisoft/soli_lang/commit/80cbbd11a6bc3c3d5943038717ce8621ab2ae639))
* **namespace class method handler key by working directory too** ([a179f58](https://github.com/solisoft/soli_lang/commit/a179f58cb490750fdfa983117fe17a3e3c966566))
* **namespace handler cache key by working directory to avoid cross-app collisions** ([89eb5ac](https://github.com/solisoft/soli_lang/commit/89eb5acfae78eec1e3976528130fb27c4c49747f))
* **add HEAD→GET fallback in route matching** ([f91ade9](https://github.com/solisoft/soli_lang/commit/f91ade96ec2858a61883c2dee3d71017ba7aca55))
* **sort controllers alphabetically before dependency sort, not after** ([2635918](https://github.com/solisoft/soli_lang/commit/26359185975a19fc6a1bfd06166699156b059d9e))
* **deterministic controller loading order and skip registration on error** ([cae299b](https://github.com/solisoft/soli_lang/commit/cae299bb67276bbdb4c4f0b7512609a84d5f147a))
* **root path "/" returns 404 when public/ dir exists** ([b7ff72f](https://github.com/solisoft/soli_lang/commit/b7ff72f0c7d271a686a0e4e913d0060133a9cb2f))
* **resolve cross-device link error in self-update command** ([4ef8f67](https://github.com/solisoft/soli_lang/commit/4ef8f6705fa2ae30ada147d92627a9b3c4fa3a05))
* **update test assertions to use constant for score and refactor class initialization** ([330c7ab](https://github.com/solisoft/soli_lang/commit/330c7ab8cf3e18aa9d614dc3f54cc912d45cf477))
* **update class instantiation syntax for nested classes** ([2af56c7](https://github.com/solisoft/soli_lang/commit/2af56c7a0fef3f898a9774276b82650789f08ffb))
* **auto-invoke zero-arg NativeFunction on member access** ([46f23f3](https://github.com/solisoft/soli_lang/commit/46f23f34cf824bd9bb51873c6a99de20369cc3b4))
* **update comment syntax from `#` to `//` in JavaScript files for consistency; refactor conditional statements to use `else if` instead of `elsif`** ([a7009d4](https://github.com/solisoft/soli_lang/commit/a7009d461efbe8dbe2a3c161c50858e3439c5512))
* **replace instances of std::f64::consts::PI with hardcoded float values for consistency** ([0b8d141](https://github.com/solisoft/soli_lang/commit/0b8d1418bdf3f20e985f93a2b30c985b33318b62))
* **update class inheritance syntax from `extends` to `<` in Solilang examples** ([1dd64c1](https://github.com/solisoft/soli_lang/commit/1dd64c10d38641104761345e3402ced41d66a482))
* **allow auto-merge step to continue on error** ([afa4f76](https://github.com/solisoft/soli_lang/commit/afa4f76b952a8ccf927df81f50fd838b0e82050f))
* **improve auto-merge logic for release PRs** ([c2ab222](https://github.com/solisoft/soli_lang/commit/c2ab222b5e1e7219458c2fd75cc0d7dfacadc054))
* **update ControlFlow to return values in expressions** ([7128dca](https://github.com/solisoft/soli_lang/commit/7128dca232d4464fa2415e2958a21b461accab6d))

### Refactoring
* **simplify S3 client retrieval and improve error handling in tests** ([da92d62](https://github.com/solisoft/soli_lang/commit/da92d62d8d5756aea15786603496247288790b8f))
* **improve test assertion formatting for clarity** ([80f79eb](https://github.com/solisoft/soli_lang/commit/80f79eb4ee29d2a5e2f8058ea70e180788600d45))
* **update validation function to support key exclusion** ([7df11b6](https://github.com/solisoft/soli_lang/commit/7df11b6740476e8934e38a3f9a873fb75eae2b49))
* **update native method arity for Model functions** ([6406b13](https://github.com/solisoft/soli_lang/commit/6406b13e4aeb22852b94bb13cd4d2c8027111d8c))
* **streamline multiline statements and improve readability** ([5a3d4d7](https://github.com/solisoft/soli_lang/commit/5a3d4d7cd18732b25118e3a64862522f70eb91a5))
* **enhance auto-invoke logic for zero-arg functions** ([349193e](https://github.com/solisoft/soli_lang/commit/349193ea8b163cdbb7ce1ba40dc5fcb2aff337e4))
* **improve multiline detection and brace balance handling** ([31d7615](https://github.com/solisoft/soli_lang/commit/31d7615f8d666b79d69c1202b76bcf65b8f53b8c))
* **route all template expressions through core language parser** ([bf94bf1](https://github.com/solisoft/soli_lang/commit/bf94bf1a57522977c8fb69309f98121dbd581e3e))
* **implement caching for resolved handlers in production mode to improve performance; optimize single-argument function calls to reduce heap allocations** ([5847a92](https://github.com/solisoft/soli_lang/commit/5847a928a6c45bc30cdbb6b5550e7cf6c0aa6763))
* **optimize header extraction and wildcard action handling to improve performance and reduce unnecessary processing** ([69bfad5](https://github.com/solisoft/soli_lang/commit/69bfad549c68e96891eec5d90e7ff2fe905a594c))
* **change query and headers parameters to owned types in build_request_hash functions to reduce cloning and improve performance** ([70f77f0](https://github.com/solisoft/soli_lang/commit/70f77f09e3b019c348280c4d46f4d371ea697350))
* **optimize exact match storage by using a nested HashMap for method:path lookups; enhance route finding efficiency and reduce allocations in request handling** ([84b1122](https://github.com/solisoft/soli_lang/commit/84b1122938519b580150ae97392beb786069da76))
* **simplify builtins registration by removing unnecessary line breaks for cleaner code** ([34c3c6b](https://github.com/solisoft/soli_lang/commit/34c3c6b6d77fb1baa57f480c540f658c93147941))
* **streamline conditional checks and improve formatting** ([818b4c2](https://github.com/solisoft/soli_lang/commit/818b4c2c4a0b6c0d67ee2cf3d0d0a66fd0e2ea05))
* **remove .gitignore and update template references** ([b1b40ae](https://github.com/solisoft/soli_lang/commit/b1b40aebc0168f04c053a6114f95753a43071af3))
* **simplify native method arity handling in member functions** ([60f5f9e](https://github.com/solisoft/soli_lang/commit/60f5f9ed8eacddfeca9b5899873c6c551ec52908))
* **update comment syntax from `//` to `#` for consistency** ([e65c363](https://github.com/solisoft/soli_lang/commit/e65c3633c463dd14ae59eb440eda5ee884a8b7b0))
* **streamline syntax highlighting logic for operators** ([e9aea16](https://github.com/solisoft/soli_lang/commit/e9aea16bcf7a9b788f4733b119a7d3f6f013c6e2))
* **simplify CI workflow by removing build and language test jobs** ([39b4c20](https://github.com/solisoft/soli_lang/commit/39b4c20c57c9d636bf81f373ff575dc98d01c268))
* **update release workflow to trigger on version tags** ([6605214](https://github.com/solisoft/soli_lang/commit/660521410aa14d90a49d3178998269f33978e930))
* **simplify code formatting and structure in various files** ([2052d69](https://github.com/solisoft/soli_lang/commit/2052d69dd6ea3ff96eadd2176cfded24f9675273))
* **restructure CI workflow to include Clippy and Rustfmt checks** ([f06f866](https://github.com/solisoft/soli_lang/commit/f06f8669d03001890d450adf78caf1245db5c68d))

### Performance
* **VM string/array/hash method dispatch and for-in loop fix** ([755bfde](https://github.com/solisoft/soli_lang/commit/755bfde8946eed0bb3856e423da68d568229c0dd))

### Documentation
* **add CLAUDE.md documentation files** ([0285dcf](https://github.com/solisoft/soli_lang/commit/0285dcf7ae3ca82297d3bc83c206a64101cd0bd9))
* **add editor integration documentation for LSP support** ([8bbb2f9](https://github.com/solisoft/soli_lang/commit/8bbb2f9dfc3ff1df547fe922806129a3c14a7474))
* **mark define_method as implemented in metaprogramming docs** ([f34e569](https://github.com/solisoft/soli_lang/commit/f34e569b7c5c13fdf3c916aff25f4273ba1a702e))
* **update metaprogramming feature status** ([1e5c880](https://github.com/solisoft/soli_lang/commit/1e5c88082f11b251f32cd064bea48e9f4d7db780))
* **add agents section and verification checklist to AGENT.md; update method names to 'includes?' in various files** ([bf38c16](https://github.com/solisoft/soli_lang/commit/bf38c16540a20b9c7c368bdb3711f3d81a045469))

### Other
* **add class name debug output** ([902da4c](https://github.com/solisoft/soli_lang/commit/902da4cf41f1830abe920786bccfc77f923cd385))
* **add handler call tracing for debugging** ([a6cbacd](https://github.com/solisoft/soli_lang/commit/a6cbacd3159c08220b9aa1b41b0c33c83a23ab1f))
* **bump version to 0.53.3** ([2edd96d](https://github.com/solisoft/soli_lang/commit/2edd96d9f4a0a26b8aa90eb0bef53b9dae813344))
* **Release solilang version 0.53.2** ([d1633e8](https://github.com/solisoft/soli_lang/commit/d1633e8216bd2dd2f331ee8741e3482082d4a9c1))
* **Release solilang version 0.53.0** ([859575a](https://github.com/solisoft/soli_lang/commit/859575a504baaf625322cf36c4890149684a9956))
* **Improve error logging with UUID request IDs for log correlation** ([32ad841](https://github.com/solisoft/soli_lang/commit/32ad841992ad876d74419a9c355d0237be410990))
* **Release solilang version 0.52.1** ([bde1bcd](https://github.com/solisoft/soli_lang/commit/bde1bcdd6b5d407de4ffa1e20d6a6327112624f0))
* **update Cargo.lock version to 0.52.0** ([7100635](https://github.com/solisoft/soli_lang/commit/710063574c11386cb25237558de4ec9c6b0a6cae))
* **bump version to 0.52.0** ([25538a5](https://github.com/solisoft/soli_lang/commit/25538a54cdd7e936c8238e1030471422ad7a34c3))
* **bump version to 0.51.0** ([045660e](https://github.com/solisoft/soli_lang/commit/045660e06d513e8b8f71df9439eb5d5830df9626))
* **bump version to 0.50.0** ([4580ac0](https://github.com/solisoft/soli_lang/commit/4580ac0287dd06c17c527c146903693618acd04c))
* **bump version to 0.49.0** ([339c36d](https://github.com/solisoft/soli_lang/commit/339c36d6f41f87e6111753c2c7d39a8a828c3456))
* **bump version to 0.48.0** ([2693061](https://github.com/solisoft/soli_lang/commit/2693061b3d65e5af6db3412f23afece3736baa8a))
* **Security and performance fixes** ([318be88](https://github.com/solisoft/soli_lang/commit/318be886d2f1d6437c8647e5cbb9c128787a9255))
* **Make trailing slash optional for routes** ([20d1322](https://github.com/solisoft/soli_lang/commit/20d13225e5d389de6798834e35fb5ca9e4680b55))
* **bump solilang version to 0.44.0 in Cargo.lock** ([f408f9b](https://github.com/solisoft/soli_lang/commit/f408f9b1f09d0186248dff02d1a5128bac6d6db9))
* **bump solilang version to 0.43.0 in Cargo.lock** ([fd2d8bb](https://github.com/solisoft/soli_lang/commit/fd2d8bb8da0777a36d4a2b5cb2d158fcc25532cf))
* **bump solilang version to 0.41.0 in Cargo.toml** ([d442de5](https://github.com/solisoft/soli_lang/commit/d442de5d0f63d46f733ea625150b2ba8e0ec5f47))
* **bump solilang version to 0.40.0 and add self-update feature** ([be8db62](https://github.com/solisoft/soli_lang/commit/be8db625043fab6319ed902339b4e079b9a4967d))
* **bump solilang version to 0.39.0 and refactor REPL source preparation** ([7779a36](https://github.com/solisoft/soli_lang/commit/7779a367f79016a619a63215c752a05ae6cabf8a))
* **remove state machines documentation and update related files** ([b7ed07b](https://github.com/solisoft/soli_lang/commit/b7ed07ba4c945cfdeca03bf46c31de9ce718f61a))
* **bump version to 0.38.0 and enhance model relationship functionality** ([f935e42](https://github.com/solisoft/soli_lang/commit/f935e42d8f087f2e582f36c031009532d915ed91))
* **bump version to 0.37.2 and enhance QueryBuilder functionality** ([3a4e920](https://github.com/solisoft/soli_lang/commit/3a4e92077e86670762114aeffcc21f21f7819b6a))
* **bump version to 0.37.1 and add compound assignment and increment/decrement operators** ([64a34f3](https://github.com/solisoft/soli_lang/commit/64a34f3a94528aad29ab8357b003642d6fd68429))
* **update solilang to version 0.37.0 and refactor REPL output handling** ([a8ce736](https://github.com/solisoft/soli_lang/commit/a8ce736a221bbafe23ac6c3beff34d5e87c5f3dc))
* **bump version to 0.37.0 and enhance REPL output handling** ([5a47c70](https://github.com/solisoft/soli_lang/commit/5a47c704266d2323c31d1353dc8bbdabd10eae7e))
* **update Cargo.toml and enhance REPL paste handling** ([0330eb0](https://github.com/solisoft/soli_lang/commit/0330eb0f9a720d776bb98f4ccad150d091af56f6))
* **update solilang version and enhance array/hash methods** ([ef5804d](https://github.com/solisoft/soli_lang/commit/ef5804da16c46989b47ad6be04db19b88b187505))
* **update dependencies and enhance REPL functionality** ([50d2b06](https://github.com/solisoft/soli_lang/commit/50d2b06b2a82674789f63ebdc37c5d3ce7c53b49))
* **bump version to 0.35.0 and enhance test server functionality** ([0d4a09d](https://github.com/solisoft/soli_lang/commit/0d4a09de3cafc802f53fa6b8db4fdecb25984b9c))
* ... and 40 more commits

### Tests
* **add comprehensive test coverage for missing language features** ([280c88f](https://github.com/solisoft/soli_lang/commit/280c88f2fc2fdcc58e53f75d868c2887c0de10bd))

### CI
* **remove cargo publish from CI** ([e7b9aa2](https://github.com/solisoft/soli_lang/commit/e7b9aa24f500d413d25b6c704588c6cd853fce62))
* **remove osx x86 build (darwin amd64)** ([860d0e9](https://github.com/solisoft/soli_lang/commit/860d0e9d40818482c463a12f44016f6b5569e308))

### Styling
* **fix formatting in handler cache** ([36fbf99](https://github.com/solisoft/soli_lang/commit/36fbf9973ee97c3b7a0e5d3e276ace27eaf18cf8))
* **fix formatting in server.rs** ([4c629cd](https://github.com/solisoft/soli_lang/commit/4c629cddff6044dac798de1df25d8def438adf2a))

### Reverts
* **remove SIGTERM handler from soli serve** ([ca194fd](https://github.com/solisoft/soli_lang/commit/ca194fd173d7410c17679f41f02743afbce711d0))

## [0.11.0](https://github.com/solisoft/soli_lang/compare/0.10.0...0.11.0) (2026-02-06)

### Features
* **add wildcard action expansion and enhance routing documentation** ([a803e8c](https://github.com/solisoft/soli_lang/commit/a803e8ce9ce74f5bc7f6c82dff2559d82baf8f1b))

### Other
* **Merge pull request #12 from solisoft/release-please--branches--main--components--solilang** ([c692928](https://github.com/solisoft/soli_lang/commit/c692928bc96b01af4964c8aca52fff3ed1cc7304))
* **release 0.11.0** ([9c4b101](https://github.com/solisoft/soli_lang/commit/9c4b10146306f6415283300399a6e2bec850f55f))

## [0.12.0](https://github.com/solisoft/soli_lang/compare/0.11.0...0.12.0) (2026-02-06)

### Features
* **implement global versioning and caching for security headers** ([20998b1](https://github.com/solisoft/soli_lang/commit/20998b18ebe0cb2a2bd24ba9ecc135f60d9c2d96))

### Other
* **Merge pull request #13 from solisoft/release-please--branches--main--components--solilang** ([ef6c42b](https://github.com/solisoft/soli_lang/commit/ef6c42bf140d15e241bd0a18e6e0f58fa65d5b36))
* **release 0.12.0** ([f1332ff](https://github.com/solisoft/soli_lang/commit/f1332ffcc5b655187c2797277d873f9b6e5eecaa))

## [0.13.0](https://github.com/solisoft/soli_lang/compare/0.12.0...0.13.0) (2026-02-06)

### Features
* **redefine DSL helpers and remove type annotations from controller methods** ([d0eb8d9](https://github.com/solisoft/soli_lang/commit/d0eb8d9acb4ca2ba8696695820c1163ea4f6c592))

### Bug Fixes
* **update ControlFlow to return values in expressions** ([7128dca](https://github.com/solisoft/soli_lang/commit/7128dca232d4464fa2415e2958a21b461accab6d))

### Other
* **Merge pull request #14 from solisoft/release-please--branches--main--components--solilang** ([2d1d382](https://github.com/solisoft/soli_lang/commit/2d1d382826c6404e5629d625cef6ed9e1c8feca2))
* **release 0.13.0** ([14faa6a](https://github.com/solisoft/soli_lang/commit/14faa6a4234feeacf3be8f9c8cac82c0e50237f3))

## [0.14.0](https://github.com/solisoft/soli_lang/compare/0.13.0...0.14.0) (2026-02-07)

### Features
* **add `sort_by` method for sorting arrays by key or function** ([799151e](https://github.com/solisoft/soli_lang/commit/799151e0dc6b79bba9d8f0672937d5ef7e6d0e58))

### Other
* **Merge pull request #15 from solisoft/release-please--branches--main--components--solilang** ([5dd6990](https://github.com/solisoft/soli_lang/commit/5dd6990d7564ea5702e8993ce70b62d1aeaaf47f))
* **release 0.14.0** ([6cce79b](https://github.com/solisoft/soli_lang/commit/6cce79bc6393280ed93bb86c1eff8c0abc752046))

## [0.15.0](https://github.com/solisoft/soli_lang/compare/0.14.0...0.15.0) (2026-02-07)

### Features
* **enhance non-blocking request sending and improve worker distribution** ([19fd116](https://github.com/solisoft/soli_lang/commit/19fd116ff83f211971060e299d69945e28fba58c))

### Other
* **Merge pull request #16 from solisoft/release-please--branches--main--components--solilang** ([1cf4882](https://github.com/solisoft/soli_lang/commit/1cf48820b895ba217c2129f77182d259e3ec161a))
* **release 0.15.0** ([390a848](https://github.com/solisoft/soli_lang/commit/390a848e8785af8c10d830f285bfe49e153b325e))

## [0.16.0](https://github.com/solisoft/soli_lang/compare/0.15.0...0.16.0) (2026-02-08)

### Features
* **integrate MessagePack serialization and deserialization** ([0febc61](https://github.com/solisoft/soli_lang/commit/0febc61138c50c7a096268b6fdd25954b13827ba))

### Refactoring
* **restructure CI workflow to include Clippy and Rustfmt checks** ([f06f866](https://github.com/solisoft/soli_lang/commit/f06f8669d03001890d450adf78caf1245db5c68d))

### Other
* **Merge pull request #17 from solisoft/release-please--branches--main--components--solilang** ([3f0a290](https://github.com/solisoft/soli_lang/commit/3f0a290fe55d29d7120281757b56794b31b6f54e))
* **release 0.16.0** ([4c9771a](https://github.com/solisoft/soli_lang/commit/4c9771a514f443d8856b3705be93a29fd380a752))

## [0.17.0](https://github.com/solisoft/soli_lang/compare/0.16.0...0.17.0) (2026-02-08)

### Features
* **enhance CI workflow with auto-merge functionality for release PRs** ([6e8296d](https://github.com/solisoft/soli_lang/commit/6e8296dbd385c7a9a02197cb425080a9d812d3e2))

### Bug Fixes
* **improve auto-merge logic for release PRs** ([c2ab222](https://github.com/solisoft/soli_lang/commit/c2ab222b5e1e7219458c2fd75cc0d7dfacadc054))

### Other
* **release 0.17.0 (#18)** ([cee08ac](https://github.com/solisoft/soli_lang/commit/cee08acbd643006fabb45132c546bdb2ac1eec62))

## [0.18.0](https://github.com/solisoft/soli_lang/compare/0.17.0...0.18.0) (2026-02-08)

### Features
* **add Rust test step to CI workflow** ([28895fb](https://github.com/solisoft/soli_lang/commit/28895fb1a69bd1e6a4b24dcde0dc0b07a05b85ae))
* **add build-binaries job to CI workflow for multi-platform releases** ([ec8894e](https://github.com/solisoft/soli_lang/commit/ec8894ed0339eacd6853ce26ebf9eef14c293a07))

### Other
* **release 0.18.0 (#19)** ([fcc2fb1](https://github.com/solisoft/soli_lang/commit/fcc2fb10fa37eb3154025bbfff23ba53da275e72))

## [0.19.0](https://github.com/solisoft/soli_lang/compare/0.18.0...0.19.0) (2026-02-08)

### Features
* **add Docker build and push step to CI workflow** ([63d9f93](https://github.com/solisoft/soli_lang/commit/63d9f932010d643177f8dd486ce93adcf115f9f0))
* **enhance CI workflow for multi-platform builds** ([3068075](https://github.com/solisoft/soli_lang/commit/30680751f8a4d2cc3ba5ff8e5f59bab3ac862f75))

### Refactoring
* **simplify code formatting and structure in various files** ([2052d69](https://github.com/solisoft/soli_lang/commit/2052d69dd6ea3ff96eadd2176cfded24f9675273))

### Other
* **release 0.19.0 (#20)** ([eed36a8](https://github.com/solisoft/soli_lang/commit/eed36a825385df96bc3c015fd720422ae3822fc8))

## [0.20.0](https://github.com/solisoft/soli_lang/compare/0.19.0...0.20.0) (2026-02-09)

### Features
* **update permissions in CI workflow** ([9a3effc](https://github.com/solisoft/soli_lang/commit/9a3effc5aba9f862068b855b9c97821225520a07))
* **update CI workflow for improved packaging and Docker integration** ([a7b008b](https://github.com/solisoft/soli_lang/commit/a7b008b859349867b14aee995d5b47a01f31822f))
* **add support for optional parentheses in function declarations** ([567f301](https://github.com/solisoft/soli_lang/commit/567f3011337230c7ed3564f18ed849c3c5c3c2f5))

### Bug Fixes
* **allow auto-merge step to continue on error** ([afa4f76](https://github.com/solisoft/soli_lang/commit/afa4f76b952a8ccf927df81f50fd838b0e82050f))

### Other
* **release 0.20.0 (#21)** ([8a85dc5](https://github.com/solisoft/soli_lang/commit/8a85dc504e4e17ccaf990c112f10dff00e991cbc))

## [0.21.0](https://github.com/solisoft/soli_lang/compare/0.20.0...0.21.0) (2026-02-10)

### Features
* **enhance route reloading by adding controller reloading in worker threads** ([b3db676](https://github.com/solisoft/soli_lang/commit/b3db676f50e72750bfbf28147989a586c4296a86))
* **add Hash.from_entries method for creating hashes from key-value pairs** ([7e7539a](https://github.com/solisoft/soli_lang/commit/7e7539a81f8f28dd8abf595d3fcd9fa79e234fe7))

### Other
* **Merge pull request #22 from solisoft/release-please--branches--main--components--solilang** ([83664d0](https://github.com/solisoft/soli_lang/commit/83664d081f0e76ad859d35583ac16ff0fd93d010))
* **release 0.21.0** ([64b534e](https://github.com/solisoft/soli_lang/commit/64b534e8245cea81603446a78aff1fccba031026))

## [0.21.1](https://github.com/solisoft/soli_lang/compare/0.21.0...0.21.1) (2026-02-12)

### Refactoring
* **update release workflow to trigger on version tags** ([6605214](https://github.com/solisoft/soli_lang/commit/660521410aa14d90a49d3178998269f33978e930))

### Other
* **bump version to 0.21.1 in Cargo.toml** ([48e6d93](https://github.com/solisoft/soli_lang/commit/48e6d93a706603448ce8d2818a036a3c991d174b))
* **Update installation instructions and recent activity logs** ([87fe47e](https://github.com/solisoft/soli_lang/commit/87fe47ec01202fa9459980b3d1c096f0b99338fd))

## [0.22.0](https://github.com/solisoft/soli_lang/compare/0.21.1...0.22.0) (2026-02-12)

### Features
* **introduce Decimal type support for exact arithmetic** ([c3b401c](https://github.com/solisoft/soli_lang/commit/c3b401cd65546579567b01e9d795f1ea5e285f37))

## [0.22.1](https://github.com/solisoft/soli_lang/compare/0.22.0...0.22.1) (2026-02-12)

### Other
* **bump version to 0.22.1 and update CI workflow** ([cbf9546](https://github.com/solisoft/soli_lang/commit/cbf954682ba4666a05567734aa3b058b1bc58390))

## [0.24.0](https://github.com/solisoft/soli_lang/compare/0.22.1...0.24.0) (2026-02-16)

### Features
* **introduce System class for command execution and backtick syntax** ([1d81dbd](https://github.com/solisoft/soli_lang/commit/1d81dbd57e6ab14cf5f745ed37c6c84fc27b75f8))
* **update view and layout file extensions to .slv** ([cacf03d](https://github.com/solisoft/soli_lang/commit/cacf03d234d83037209f54cc4b35b6c6efe14ea7))

### Refactoring
* **simplify CI workflow by removing build and language test jobs** ([39b4c20](https://github.com/solisoft/soli_lang/commit/39b4c20c57c9d636bf81f373ff575dc98d01c268))

### Other
* **bump version to 0.24.0 and implement const fields** ([a130554](https://github.com/solisoft/soli_lang/commit/a1305542e89e7767fd8b527e5a94e09c91f0f220))
* **remove outdated conventions and example files for Soli MVC** ([12c8cbf](https://github.com/solisoft/soli_lang/commit/12c8cbf6d33cb86fc957251681e699ec652806d9))
* **bump version to 0.23.0 and refactor inline cache** ([dbb80c5](https://github.com/solisoft/soli_lang/commit/dbb80c551f1f12db696a57875c0efc42bd7e81ca))

## [0.26.0](https://github.com/solisoft/soli_lang/compare/0.24.0...0.26.0) (2026-02-19)

### Features
* **enhance testing coverage for various built-in functions and classes** ([fb31e0c](https://github.com/solisoft/soli_lang/commit/fb31e0c558da26e78c6923aa2424933eaf165546))
* **introduce safe navigation operator and return type enforcement** ([13266e9](https://github.com/solisoft/soli_lang/commit/13266e9ca572f6ec48d22faa439d6bdac2f66438))
* **add `dig` method for nested key retrieval and enhance `clear` functionality** ([af25225](https://github.com/solisoft/soli_lang/commit/af25225adc117e8b91ffcfb5dcb786cfef2d704c))
* **enhance documentation for HTTP server functions and update controller signatures** ([c4a02d1](https://github.com/solisoft/soli_lang/commit/c4a02d12408fee62d95724229b970383517ecb8d))

## [0.27.1](https://github.com/solisoft/soli_lang/compare/0.26.0...0.27.1) (2026-02-19)

### Features
* **update Solilang version to 0.27.1 and enhance VSCode extension** ([209d8bf](https://github.com/solisoft/soli_lang/commit/209d8bf22bd2202e3bf45b41be0a1f346b79cb3f))
* **introduce linting functionality and enhance error handling syntax** ([64254e7](https://github.com/solisoft/soli_lang/commit/64254e757d6d92e8e1aa0839bb2a92f0bf443d4c))

### Bug Fixes
* **update class inheritance syntax from `extends` to `<` in Solilang examples** ([1dd64c1](https://github.com/solisoft/soli_lang/commit/1dd64c10d38641104761345e3402ced41d66a482))

### Refactoring
* **update comment syntax from `//` to `#` for consistency** ([e65c363](https://github.com/solisoft/soli_lang/commit/e65c3633c463dd14ae59eb440eda5ee884a8b7b0))
* **streamline syntax highlighting logic for operators** ([e9aea16](https://github.com/solisoft/soli_lang/commit/e9aea16bcf7a9b788f4733b119a7d3f6f013c6e2))

## [0.27.2](https://github.com/solisoft/soli_lang/compare/0.27.1...0.27.2) (2026-02-19)

### Refactoring
* **simplify native method arity handling in member functions** ([60f5f9e](https://github.com/solisoft/soli_lang/commit/60f5f9ed8eacddfeca9b5899873c6c551ec52908))

## [0.28.0](https://github.com/solisoft/soli_lang/compare/0.27.2...0.28.0) (2026-02-20)

### Features
* **update Solilang to version 0.28.0 and enhance templates** ([c903576](https://github.com/solisoft/soli_lang/commit/c90357680305196533ed29d76c424a9af6b2269d))

### Refactoring
* **remove .gitignore and update template references** ([b1b40ae](https://github.com/solisoft/soli_lang/commit/b1b40aebc0168f04c053a6114f95753a43071af3))

## [0.29.0](https://github.com/solisoft/soli_lang/compare/0.28.0...0.29.0) (2026-02-20)

### Features
* **update Solilang to version 0.29.0 and enhance file structure** ([e16694a](https://github.com/solisoft/soli_lang/commit/e16694a25bcdb6575555f16d46ec1151d93b11dc))

### Refactoring
* **streamline conditional checks and improve formatting** ([818b4c2](https://github.com/solisoft/soli_lang/commit/818b4c2c4a0b6c0d67ee2cf3d0d0a66fd0e2ea05))

## [0.29.1](https://github.com/solisoft/soli_lang/compare/0.29.0...0.29.1) (2026-02-20)

### Features
* **implement reload cooldown for live reload functionality** ([433f535](https://github.com/solisoft/soli_lang/commit/433f53584767318369f8950499de6a97ca140962))

### Bug Fixes
* **replace instances of std::f64::consts::PI with hardcoded float values for consistency** ([0b8d141](https://github.com/solisoft/soli_lang/commit/0b8d1418bdf3f20e985f93a2b30c985b33318b62))

### Other
* **update Solilang version to 0.29.1 and improve benchmark error handling** ([5ec48f0](https://github.com/solisoft/soli_lang/commit/5ec48f00380ba55910a29b6000ec314bf7d2b7e7))

## [0.30.0](https://github.com/solisoft/soli_lang/compare/0.29.1...0.30.0) (2026-02-20)

### Features
* **add support for 'new' as a method name and enhance related tests** ([0b982a9](https://github.com/solisoft/soli_lang/commit/0b982a902f7b2b115a066088f8384304ad2a9234))

## [0.30.1](https://github.com/solisoft/soli_lang/compare/0.30.0...0.30.1) (2026-02-20)

### Features
* **add support for 'match' as a method name and enhance related tests** ([7f67ad5](https://github.com/solisoft/soli_lang/commit/7f67ad5f6210f9096c01bdaa74d7443073e162c2))

### Other
* **update Solilang version to 0.30.1 in Cargo.lock** ([a935ac1](https://github.com/solisoft/soli_lang/commit/a935ac1434503501d466c5427fa7c3d4c163d5cf))

## [0.30.2](https://github.com/solisoft/soli_lang/compare/0.30.1...0.30.2) (2026-02-20)

### Features
* **enforce same-line requirement for postfix if/unless statements** ([ed410ac](https://github.com/solisoft/soli_lang/commit/ed410acd32c8997f0cbdedfe009512df3d47261e))

### Other
* **chore(.gitignore): add CLAUDE.md and .dmux/ to ignore list; chore(Cargo.lock): update Solilang version to 0.30.2; test(parser): format test cases for improved readability** ([c4f5c61](https://github.com/solisoft/soli_lang/commit/c4f5c61246121c80f7165bd494f530f33530d4e8))
* **bump Solilang version to 0.30.2 in Cargo.toml** ([330a50b](https://github.com/solisoft/soli_lang/commit/330a50b93856c93d1ccd0f495516263b68009af6))

## [0.30.3](https://github.com/solisoft/soli_lang/compare/0.30.2...0.30.3) (2026-02-20)

### Features
* **enhance middleware function extraction to support both `//` and `#` comment styles** ([e49d9ed](https://github.com/solisoft/soli_lang/commit/e49d9ed8b56f55940bf9ed302a4a25768796c006))

### Other
* **update Solilang version to 0.30.3 in Cargo.lock; refactor comment handling in extract_middleware_functions** ([acdae0a](https://github.com/solisoft/soli_lang/commit/acdae0aca534ce2f191ab4586231e91e5cd885bc))
* **bump Solilang version to 0.30.3 in Cargo.toml** ([166fa3a](https://github.com/solisoft/soli_lang/commit/166fa3a21e32f388f7018a5e8f569484047431a9))

## [0.31.0](https://github.com/solisoft/soli_lang/compare/0.30.3...0.31.0) (2026-02-21)

### Other
* **bump Solilang version to 0.31.0; update default server port to 5011 and adjust related documentation** ([42bbc6b](https://github.com/solisoft/soli_lang/commit/42bbc6bf4269afee9bc21a59911bb71fffbe3d2c))
* **update Solilang version to 0.30.4 in Cargo.lock and Cargo.toml; refactor resources function to allow default null block parameter** ([5eca535](https://github.com/solisoft/soli_lang/commit/5eca535c3ac0f57a98137a61bb9e8d363bb5ffdb))

## [0.31.1](https://github.com/solisoft/soli_lang/compare/0.31.0...0.31.1) (2026-02-21)

### Features
* **add `len` method to String class; update token parsing for `and` and `or` keywords; fix server port configuration in tests** ([82bd522](https://github.com/solisoft/soli_lang/commit/82bd5228f3a42c6859b5b1d8b95ba6d8d9be4b0d))

### Other
* **bump Solilang version to 0.31.1; add `len` method as alias for `length` in String class and update related tests** ([4d7661f](https://github.com/solisoft/soli_lang/commit/4d7661f029128927024fa55a78de29d9f8de3492))

## [0.31.2](https://github.com/solisoft/soli_lang/compare/0.31.1...0.31.2) (2026-02-21)

### Features
* **load models into REPL session on first use; add models_loaded flag to ReplSession** ([a5dde94](https://github.com/solisoft/soli_lang/commit/a5dde94f049583d1303659043517bd631c923d81))

### Bug Fixes
* **update comment syntax from `#` to `//` in JavaScript files for consistency; refactor conditional statements to use `else if` instead of `elsif`** ([a7009d4](https://github.com/solisoft/soli_lang/commit/a7009d461efbe8dbe2a3c161c50858e3439c5512))

### Other
* **bump Solilang version to 0.31.2; refactor HTTP request handling to use JSON instead of MessagePack and enhance query string parsing with URL decoding** ([7a8836d](https://github.com/solisoft/soli_lang/commit/7a8836dba1c170dc31cce8ab0a7ffadfaa02d5ad))

## [0.33.0](https://github.com/solisoft/soli_lang/compare/0.31.2...0.33.0) (2026-02-28)

### Features
* **route output expressions through core parser; auto-call no-arg methods** ([34634e9](https://github.com/solisoft/soli_lang/commit/34634e9acc3ee6e4715ccaef9e6f66107dbf5163))
* **route <% %> code blocks through the core language parser for full language support** ([38a1d4a](https://github.com/solisoft/soli_lang/commit/38a1d4aeefb3d26a6cab09e57d4fcbf94d2fd906))
* **optimize template rendering by introducing a shared interpreter for layout and view rendering, reducing allocations and improving performance; enhance environment with data hash for efficient variable lookups** ([a31ee5c](https://github.com/solisoft/soli_lang/commit/a31ee5c68833488e0f0faa8c80b5343b1e917b70))
* **enhance for loop syntax to support optional index variable; update related parsing, execution, and linting logic** ([c1a18c5](https://github.com/solisoft/soli_lang/commit/c1a18c56b67ce2df65c48013b69b65d7960d3258))

### Bug Fixes
* **auto-invoke zero-arg NativeFunction on member access** ([46f23f3](https://github.com/solisoft/soli_lang/commit/46f23f34cf824bd9bb51873c6a99de20369cc3b4))

### Refactoring
* **enhance auto-invoke logic for zero-arg functions** ([349193e](https://github.com/solisoft/soli_lang/commit/349193ea8b163cdbb7ce1ba40dc5fcb2aff337e4))
* **improve multiline detection and brace balance handling** ([31d7615](https://github.com/solisoft/soli_lang/commit/31d7615f8d666b79d69c1202b76bcf65b8f53b8c))
* **route all template expressions through core language parser** ([bf94bf1](https://github.com/solisoft/soli_lang/commit/bf94bf1a57522977c8fb69309f98121dbd581e3e))
* **implement caching for resolved handlers in production mode to improve performance; optimize single-argument function calls to reduce heap allocations** ([5847a92](https://github.com/solisoft/soli_lang/commit/5847a928a6c45bc30cdbb6b5550e7cf6c0aa6763))
* **optimize header extraction and wildcard action handling to improve performance and reduce unnecessary processing** ([69bfad5](https://github.com/solisoft/soli_lang/commit/69bfad549c68e96891eec5d90e7ff2fe905a594c))
* **change query and headers parameters to owned types in build_request_hash functions to reduce cloning and improve performance** ([70f77f0](https://github.com/solisoft/soli_lang/commit/70f77f09e3b019c348280c4d46f4d371ea697350))
* **optimize exact match storage by using a nested HashMap for method:path lookups; enhance route finding efficiency and reduce allocations in request handling** ([84b1122](https://github.com/solisoft/soli_lang/commit/84b1122938519b580150ae97392beb786069da76))
* **simplify builtins registration by removing unnecessary line breaks for cleaner code** ([34c3c6b](https://github.com/solisoft/soli_lang/commit/34c3c6b6d77fb1baa57f480c540f658c93147941))

### Other
* **bump Solilang version to 0.33.0; integrate mimalloc for improved memory allocation and update HTTP client handling for better performance** ([d52da39](https://github.com/solisoft/soli_lang/commit/d52da39886fb78946694629ddeef9d3981af94c5))
* **bump Solilang version to 0.32.0; add support for Markdown views with .md and .html.md extensions, enhancing the rendering pipeline to process template tags before converting Markdown to HTML** ([b59616f](https://github.com/solisoft/soli_lang/commit/b59616f24c77188f40715c29db928bc4c55d6ec4))

## [0.35.0](https://github.com/solisoft/soli_lang/compare/0.33.0...0.35.0) (2026-03-03)

### Features
* **update package management and add new commands** ([85d0db4](https://github.com/solisoft/soli_lang/commit/85d0db43f733f84bae5386bfad36a7245c9ee25b))
* **introduce common REPL utilities for multiline handling** ([29bd3af](https://github.com/solisoft/soli_lang/commit/29bd3af5174adf6197c17f39636e1e5fb895ffba))

### Refactoring
* **streamline multiline statements and improve readability** ([5a3d4d7](https://github.com/solisoft/soli_lang/commit/5a3d4d7cd18732b25118e3a64862522f70eb91a5))

### Other
* **bump version to 0.35.0 and enhance test server functionality** ([0d4a09d](https://github.com/solisoft/soli_lang/commit/0d4a09de3cafc802f53fa6b8db4fdecb25984b9c))
* **bump version to 0.35.0 and enhance builtins registration** ([4a67e6e](https://github.com/solisoft/soli_lang/commit/4a67e6e2492512c54fd7cfb62ffc973d89771f12))
* **update dependencies and refactor code for consistency** ([ca39ba4](https://github.com/solisoft/soli_lang/commit/ca39ba4c96c721907a5fd2ed3958dcd910b8ef2f))

## [0.36.0](https://github.com/solisoft/soli_lang/compare/0.35.0...0.36.0) (2026-03-04)

### Features
* **add JSON performance benchmarks and enhance hash operations** ([8b408bf](https://github.com/solisoft/soli_lang/commit/8b408bf01ecf7c9872c6c7763e2d9e93e97a90a0))
* **enhance completion functionality in REPL TUI** ([91c4280](https://github.com/solisoft/soli_lang/commit/91c4280dbce38b40dbd0753b7968e4b8ffb6c240))

### Other
* **update Cargo.toml and enhance REPL paste handling** ([0330eb0](https://github.com/solisoft/soli_lang/commit/0330eb0f9a720d776bb98f4ccad150d091af56f6))
* **update solilang version and enhance array/hash methods** ([ef5804d](https://github.com/solisoft/soli_lang/commit/ef5804da16c46989b47ad6be04db19b88b187505))
* **update dependencies and enhance REPL functionality** ([50d2b06](https://github.com/solisoft/soli_lang/commit/50d2b06b2a82674789f63ebdc37c5d3ce7c53b49))

## [0.37.0](https://github.com/solisoft/soli_lang/compare/0.36.0...0.37.0) (2026-03-04)

### Other
* **update solilang to version 0.37.0 and refactor REPL output handling** ([a8ce736](https://github.com/solisoft/soli_lang/commit/a8ce736a221bbafe23ac6c3beff34d5e87c5f3dc))
* **bump version to 0.37.0 and enhance REPL output handling** ([5a47c70](https://github.com/solisoft/soli_lang/commit/5a47c704266d2323c31d1353dc8bbdabd10eae7e))

## [0.37.1](https://github.com/solisoft/soli_lang/compare/0.37.0...0.37.1) (2026-03-04)

### Other
* **bump version to 0.37.1 and add compound assignment and increment/decrement operators** ([64a34f3](https://github.com/solisoft/soli_lang/commit/64a34f3a94528aad29ab8357b003642d6fd68429))

## [0.38.0](https://github.com/solisoft/soli_lang/compare/0.37.1...0.38.0) (2026-03-05)

### Other
* **remove state machines documentation and update related files** ([b7ed07b](https://github.com/solisoft/soli_lang/commit/b7ed07ba4c945cfdeca03bf46c31de9ce718f61a))
* **bump version to 0.38.0 and enhance model relationship functionality** ([f935e42](https://github.com/solisoft/soli_lang/commit/f935e42d8f087f2e582f36c031009532d915ed91))
* **bump version to 0.37.2 and enhance QueryBuilder functionality** ([3a4e920](https://github.com/solisoft/soli_lang/commit/3a4e92077e86670762114aeffcc21f21f7819b6a))

## [0.38.1](https://github.com/solisoft/soli_lang/compare/0.38.0...0.38.1) (2026-03-05)

### Features
* **add 'size' method to collections and enhance string methods** ([c7c2377](https://github.com/solisoft/soli_lang/commit/c7c2377bf80ef0df461d4ad8148323e0c5c044ac))
* **enhance Model includes and select functionality** ([15b27c5](https://github.com/solisoft/soli_lang/commit/15b27c573e4eea0160cf74d81cdc5608544bcfd3))

### Performance
* **VM string/array/hash method dispatch and for-in loop fix** ([755bfde](https://github.com/solisoft/soli_lang/commit/755bfde8946eed0bb3856e423da68d568229c0dd))

## [0.39.0](https://github.com/solisoft/soli_lang/compare/0.38.1...0.39.0) (2026-03-07)

### Features
* **integrate regex caching and enhance regex handling** ([3d7cbf7](https://github.com/solisoft/soli_lang/commit/3d7cbf7d490e34de220cc309cae17d713ba2ec5e))

### Other
* **bump solilang version to 0.39.0 and refactor REPL source preparation** ([7779a36](https://github.com/solisoft/soli_lang/commit/7779a367f79016a619a63215c752a05ae6cabf8a))

## [0.40.0](https://github.com/solisoft/soli_lang/compare/0.39.0...0.40.0) (2026-03-10)

### Bug Fixes
* **update class instantiation syntax for nested classes** ([2af56c7](https://github.com/solisoft/soli_lang/commit/2af56c7a0fef3f898a9774276b82650789f08ffb))

### Other
* **bump solilang version to 0.40.0 and add self-update feature** ([be8db62](https://github.com/solisoft/soli_lang/commit/be8db625043fab6319ed902339b4e079b9a4967d))

## [0.41.0](https://github.com/solisoft/soli_lang/compare/0.40.0...0.41.0) (2026-03-10)

### Features
* **enhance model instance handling and query execution** ([034be44](https://github.com/solisoft/soli_lang/commit/034be4473e63dc80bc97c8c12e44302a91d7ed75))

### Refactoring
* **update native method arity for Model functions** ([6406b13](https://github.com/solisoft/soli_lang/commit/6406b13e4aeb22852b94bb13cd4d2c8027111d8c))

### Documentation
* **add agents section and verification checklist to AGENT.md; update method names to 'includes?' in various files** ([bf38c16](https://github.com/solisoft/soli_lang/commit/bf38c16540a20b9c7c368bdb3711f3d81a045469))

### Other
* **bump solilang version to 0.41.0 in Cargo.toml** ([d442de5](https://github.com/solisoft/soli_lang/commit/d442de5d0f63d46f733ea625150b2ba8e0ec5f47))

## [0.43.0](https://github.com/solisoft/soli_lang/compare/0.41.0...0.43.0) (2026-03-11)

### Features
* **introduce symbol type and related functionality** ([04dc3a6](https://github.com/solisoft/soli_lang/commit/04dc3a6fe22efe10ef2c15a3de52964bdf2fd09c))
* **add translation support for model fields** ([76f9991](https://github.com/solisoft/soli_lang/commit/76f9991c703f2c18a79dccf7ccaafc41270f27bd))
* **enhance model functionality with new query and transaction features** ([048b3e8](https://github.com/solisoft/soli_lang/commit/048b3e8e2fd1bdb9596d5ee135a46e7e526439bb))

### Refactoring
* **update validation function to support key exclusion** ([7df11b6](https://github.com/solisoft/soli_lang/commit/7df11b6740476e8934e38a3f9a873fb75eae2b49))

### Other
* **bump solilang version to 0.43.0 in Cargo.lock** ([fd2d8bb](https://github.com/solisoft/soli_lang/commit/fd2d8bb8da0777a36d4a2b5cb2d158fcc25532cf))

## [0.44.0](https://github.com/solisoft/soli_lang/compare/0.43.0...0.44.0) (2026-03-11)

### Features
* **implement SoliKV-backed caching functionality** ([7a13ca4](https://github.com/solisoft/soli_lang/commit/7a13ca4a2808f3da2730d592250f944ae5d1f811))

### Other
* **bump solilang version to 0.44.0 in Cargo.lock** ([f408f9b](https://github.com/solisoft/soli_lang/commit/f408f9b1f09d0186248dff02d1a5128bac6d6db9))

## [0.46.0](https://github.com/solisoft/soli_lang/compare/0.44.0...0.46.0) (2026-03-21)

### Features
* **enhance File class with new methods and update documentation** ([d1af684](https://github.com/solisoft/soli_lang/commit/d1af684032513e9dc3921f28330b860e5a63eab9))
* **implement TOTP functionality and enhance image support** ([4bd82ea](https://github.com/solisoft/soli_lang/commit/4bd82ea24bae4623c4bdb552351c705e398d3dd3))
* **enhance array and hash methods with optimized string representation** ([e69f2bb](https://github.com/solisoft/soli_lang/commit/e69f2bbc062ed46127278df33e126d1b845ad6e2))

### Bug Fixes
* **update test assertions to use constant for score and refactor class initialization** ([330c7ab](https://github.com/solisoft/soli_lang/commit/330c7ab8cf3e18aa9d614dc3f54cc912d45cf477))

### Refactoring
* **simplify S3 client retrieval and improve error handling in tests** ([da92d62](https://github.com/solisoft/soli_lang/commit/da92d62d8d5756aea15786603496247288790b8f))
* **improve test assertion formatting for clarity** ([80f79eb](https://github.com/solisoft/soli_lang/commit/80f79eb4ee29d2a5e2f8058ea70e180788600d45))

## [0.46.1](https://github.com/solisoft/soli_lang/compare/0.46.0...0.46.1) (2026-03-23)

### Other
* **Make trailing slash optional for routes** ([20d1322](https://github.com/solisoft/soli_lang/commit/20d13225e5d389de6798834e35fb5ca9e4680b55))

## [0.46.2](https://github.com/solisoft/soli_lang/compare/0.46.1...0.46.2) (2026-03-23)

### Other
* **Security and performance fixes** ([318be88](https://github.com/solisoft/soli_lang/commit/318be886d2f1d6437c8647e5cbb9c128787a9255))

## [0.48.0](https://github.com/solisoft/soli_lang/compare/0.46.2...0.48.0) (2026-03-23)

### Features
* **add alias_method and inherited hook for metaprogramming** ([0f922bb](https://github.com/solisoft/soli_lang/commit/0f922bb3224ee46c1d790624368b11f268875687))
* **add define_method for runtime method definition** ([5ed1591](https://github.com/solisoft/soli_lang/commit/5ed15910baddea6d82767250a32e21890b5d1e21))
* **add class_eval for metaprogramming** ([825dd49](https://github.com/solisoft/soli_lang/commit/825dd49c829a9b8abed075f9fc6d7efa0bdb746f))
* **add instance_eval for metaprogramming** ([321c6a0](https://github.com/solisoft/soli_lang/commit/321c6a00905e7c64f0bbdfdbbc9877bce0d36d11))
* **add instance_variable_set metaprogramming method** ([9159f11](https://github.com/solisoft/soli_lang/commit/9159f115b4772dd2745fcf4f89ad6a0669534cf6))
* **add metaprogramming support (respond_to?, send, method_missing, etc.)** ([bce1d30](https://github.com/solisoft/soli_lang/commit/bce1d3076b6414cf57f6af01600521625b8f67b9))

### Documentation
* **mark define_method as implemented in metaprogramming docs** ([f34e569](https://github.com/solisoft/soli_lang/commit/f34e569b7c5c13fdf3c916aff25f4273ba1a702e))
* **update metaprogramming feature status** ([1e5c880](https://github.com/solisoft/soli_lang/commit/1e5c88082f11b251f32cd064bea48e9f4d7db780))

### Other
* **bump version to 0.48.0** ([2693061](https://github.com/solisoft/soli_lang/commit/2693061b3d65e5af6db3412f23afece3736baa8a))

## [0.49.0](https://github.com/solisoft/soli_lang/compare/0.48.0...0.49.0) (2026-03-24)

### Features
* **add soli deploy command for blue-green deployments** ([ad5b988](https://github.com/solisoft/soli_lang/commit/ad5b98841475e07e080a0bcb3954ae35541fa999))

### Documentation
* **add editor integration documentation for LSP support** ([8bbb2f9](https://github.com/solisoft/soli_lang/commit/8bbb2f9dfc3ff1df547fe922806129a3c14a7474))

### Other
* **bump version to 0.49.0** ([339c36d](https://github.com/solisoft/soli_lang/commit/339c36d6f41f87e6111753c2c7d39a8a828c3456))

## [0.50.0](https://github.com/solisoft/soli_lang/compare/0.49.0...0.50.0) (2026-03-24)

### Features
* **add migration support to deploy command** ([9954d1f](https://github.com/solisoft/soli_lang/commit/9954d1f86c6ab09eda39aeba31e4e6311e292676))

### Other
* **bump version to 0.50.0** ([4580ac0](https://github.com/solisoft/soli_lang/commit/4580ac0287dd06c17c527c146903693618acd04c))

## [0.51.0](https://github.com/solisoft/soli_lang/compare/0.50.0...0.51.0) (2026-03-24)

### Other
* **bump version to 0.51.0** ([045660e](https://github.com/solisoft/soli_lang/commit/045660e06d513e8b8f71df9439eb5d5830df9626))

### CI
* **remove osx x86 build (darwin amd64)** ([860d0e9](https://github.com/solisoft/soli_lang/commit/860d0e9d40818482c463a12f44016f6b5569e308))

## [0.52.0](https://github.com/solisoft/soli_lang/compare/0.51.0...0.52.0) (2026-03-24)

### Features
* **add --version and -v flags** ([974c3c5](https://github.com/solisoft/soli_lang/commit/974c3c53082eeeeff04505132b29866bc7dee2a1))

### Other
* **bump version to 0.52.0** ([25538a5](https://github.com/solisoft/soli_lang/commit/25538a54cdd7e936c8238e1030471422ad7a34c3))

## [0.52.1](https://github.com/solisoft/soli_lang/compare/0.52.0...0.52.1) (2026-03-24)

### Bug Fixes
* **resolve cross-device link error in self-update command** ([4ef8f67](https://github.com/solisoft/soli_lang/commit/4ef8f6705fa2ae30ada147d92627a9b3c4fa3a05))

### Other
* **Release solilang version 0.52.1** ([bde1bcd](https://github.com/solisoft/soli_lang/commit/bde1bcdd6b5d407de4ffa1e20d6a6327112624f0))
* **update Cargo.lock version to 0.52.0** ([7100635](https://github.com/solisoft/soli_lang/commit/710063574c11386cb25237558de4ec9c6b0a6cae))

## [0.52.2](https://github.com/solisoft/soli_lang/compare/0.52.1...0.52.2) (2026-03-25)

### Bug Fixes
* **root path "/" returns 404 when public/ dir exists** ([b7ff72f](https://github.com/solisoft/soli_lang/commit/b7ff72f0c7d271a686a0e4e913d0060133a9cb2f))

### CI
* **remove cargo publish from CI** ([e7b9aa2](https://github.com/solisoft/soli_lang/commit/e7b9aa24f500d413d25b6c704588c6cd853fce62))

## [0.52.3](https://github.com/solisoft/soli_lang/compare/0.52.2...0.52.3) (2026-03-27)

### Features
* **add HTTP Range request support for video/audio streaming** ([736a34b](https://github.com/solisoft/soli_lang/commit/736a34b1bde417b7f51ba8b4e8363ff66af24be9))

## [0.53.0](https://github.com/solisoft/soli_lang/compare/0.52.3...0.53.0) (2026-03-30)

### Other
* **Release solilang version 0.53.0** ([859575a](https://github.com/solisoft/soli_lang/commit/859575a504baaf625322cf36c4890149684a9956))
* **Improve error logging with UUID request IDs for log correlation** ([32ad841](https://github.com/solisoft/soli_lang/commit/32ad841992ad876d74419a9c355d0237be410990))

## [0.53.1](https://github.com/solisoft/soli_lang/compare/0.53.0...0.53.1) (2026-03-31)

### Bug Fixes
* **sort controllers alphabetically before dependency sort, not after** ([2635918](https://github.com/solisoft/soli_lang/commit/26359185975a19fc6a1bfd06166699156b059d9e))
* **deterministic controller loading order and skip registration on error** ([cae299b](https://github.com/solisoft/soli_lang/commit/cae299bb67276bbdb4c4f0b7512609a84d5f147a))

## [0.53.2](https://github.com/solisoft/soli_lang/compare/0.53.1...0.53.2) (2026-03-31)

### Other
* **Release solilang version 0.53.2** ([d1633e8](https://github.com/solisoft/soli_lang/commit/d1633e8216bd2dd2f331ee8741e3482082d4a9c1))

## [0.53.3](https://github.com/solisoft/soli_lang/compare/0.53.2...0.53.3) (2026-03-31)

### Bug Fixes
* **add HEAD→GET fallback in route matching** ([f91ade9](https://github.com/solisoft/soli_lang/commit/f91ade96ec2858a61883c2dee3d71017ba7aca55))

### Other
* **bump version to 0.53.3** ([2edd96d](https://github.com/solisoft/soli_lang/commit/2edd96d9f4a0a26b8aa90eb0bef53b9dae813344))

## [0.53.4](https://github.com/solisoft/soli_lang/compare/0.53.3...0.53.4) (2026-03-31)

### Styling
* **fix formatting in server.rs** ([4c629cd](https://github.com/solisoft/soli_lang/commit/4c629cddff6044dac798de1df25d8def438adf2a))

## [0.53.5](https://github.com/solisoft/soli_lang/compare/0.53.4...0.53.5) (2026-03-31)

### Bug Fixes
* **namespace handler cache key by working directory to avoid cross-app collisions** ([89eb5ac](https://github.com/solisoft/soli_lang/commit/89eb5acfae78eec1e3976528130fb27c4c49747f))

## [0.53.6](https://github.com/solisoft/soli_lang/compare/0.53.5...0.53.6) (2026-03-31)

### Styling
* **fix formatting in handler cache** ([36fbf99](https://github.com/solisoft/soli_lang/commit/36fbf9973ee97c3b7a0e5d3e276ace27eaf18cf8))

## [0.53.7](https://github.com/solisoft/soli_lang/compare/0.53.6...0.53.7) (2026-03-31)

### Bug Fixes
* **namespace class method handler key by working directory too** ([a179f58](https://github.com/solisoft/soli_lang/commit/a179f58cb490750fdfa983117fe17a3e3c966566))

## [0.53.8](https://github.com/solisoft/soli_lang/compare/0.53.7...0.53.8) (2026-03-31)

### Bug Fixes
* **namespace NON_OOP_CONTROLLERS cache by working directory** ([80cbbd1](https://github.com/solisoft/soli_lang/commit/80cbbd11a6bc3c3d5943038717ce8621ab2ae639))

## [0.53.9](https://github.com/solisoft/soli_lang/compare/0.53.8...0.53.9) (2026-03-31)

### Other
* **add handler call tracing for debugging** ([a6cbacd](https://github.com/solisoft/soli_lang/commit/a6cbacd3159c08220b9aa1b41b0c33c83a23ab1f))

## [0.53.10](https://github.com/solisoft/soli_lang/compare/0.53.9...0.53.10) (2026-03-31)

### Bug Fixes
* **remove handler cache to avoid stale handler lookups** ([7a85cf5](https://github.com/solisoft/soli_lang/commit/7a85cf5b99a91a2fca551a9082a5f676b71d5662))

## [0.53.11](https://github.com/solisoft/soli_lang/compare/0.53.10...0.53.11) (2026-03-31)

### Bug Fixes
* **remove NON_OOP_CONTROLLERS cache to avoid incorrect controller classification** ([1c52d56](https://github.com/solisoft/soli_lang/commit/1c52d56a25496512cc5262d3cd37af78140c784b))

## [0.53.12](https://github.com/solisoft/soli_lang/compare/0.53.11...0.53.12) (2026-03-31)

### Other
* **add class name debug output** ([902da4c](https://github.com/solisoft/soli_lang/commit/902da4cf41f1830abe920786bccfc77f923cd385))

## [0.53.13](https://github.com/solisoft/soli_lang/compare/0.53.12...0.53.13) (2026-03-31)

### Bug Fixes
* **remove JIT closure caching in VM that caused handler collisions** ([37a3af7](https://github.com/solisoft/soli_lang/commit/37a3af739e4107a5565d9ca07eb754f157fc3b23))

## [0.53.14](https://github.com/solisoft/soli_lang/compare/0.53.13...0.53.14) (2026-04-01)

### Features
* **add graceful shutdown with SIGTERM handler for blue-green deployments** ([83ebb21](https://github.com/solisoft/soli_lang/commit/83ebb21d0a69c9628ce13fdfdb243b9a756f7592))

## [0.53.15](https://github.com/solisoft/soli_lang/compare/0.53.14...0.53.15) (2026-04-01)

### Bug Fixes
* **reduce graceful drain to 5s and stop returning 503 during shutdown** ([4987c0e](https://github.com/solisoft/soli_lang/commit/4987c0eb79be2f32f10c3369266a0a649977b899))

## [0.53.16](https://github.com/solisoft/soli_lang/compare/0.53.15...0.53.16) (2026-04-01)

### Bug Fixes
* **reduce SIGTERM drain to 1s to avoid blocking restarts** ([c4060be](https://github.com/solisoft/soli_lang/commit/c4060be3a1fb0158334a30aa02197942fa29874a))

## [0.53.17](https://github.com/solisoft/soli_lang/compare/0.53.16...0.53.17) (2026-04-01)

### Reverts
* **remove SIGTERM handler from soli serve** ([ca194fd](https://github.com/solisoft/soli_lang/commit/ca194fd173d7410c17679f41f02743afbce711d0))

## [0.54.0](https://github.com/solisoft/soli_lang/compare/0.53.17...0.54.0) (2026-04-02)

### Features
* **add CLAUDE.md to new apps, fix interface docs** ([0f1549b](https://github.com/solisoft/soli_lang/commit/0f1549b9089178f6437c9e3ba741c70a6d57d3a3))

## [0.55.0](https://github.com/solisoft/soli_lang/compare/0.54.0...0.55.0) (2026-04-02)

### Tests
* **add comprehensive test coverage for missing language features** ([280c88f](https://github.com/solisoft/soli_lang/commit/280c88f2fc2fdcc58e53f75d868c2887c0de10bd))

## [0.55.2](https://github.com/solisoft/soli_lang/compare/0.55.0...0.55.2) (2026-04-09)

### Features
* **add release script and CI version guard** ([a428535](https://github.com/solisoft/soli_lang/commit/a42853505e000664a10ffcb46705ccce30bf05e4))
* **add percent literal arrays %w[], %i[], %n[] with decimal support** ([35074e3](https://github.com/solisoft/soli_lang/commit/35074e37c8e033d6e4af570f9c6caa41f0f78ec0))

### Documentation
* **add CLAUDE.md documentation files** ([0285dcf](https://github.com/solisoft/soli_lang/commit/0285dcf7ae3ca82297d3bc83c206a64101cd0bd9))

### Other
* **bump version to v0.55.2** ([3431766](https://github.com/solisoft/soli_lang/commit/3431766c5d376350d88553f761c200b3b31ffe03))
* **bump version to v0.55.1** ([c9fe339](https://github.com/solisoft/soli_lang/commit/c9fe33965b102ffff01648fa4eab52f9a56d7c37))

## [0.55.3](https://github.com/solisoft/soli_lang/compare/0.55.2...0.55.3) (2026-04-10)

### Features
* **add locale support to DateTime.format() for I18n** ([24e4889](https://github.com/solisoft/soli_lang/commit/24e4889888763ce3b9e71642faa110f38dd910e0))

### Other
* **bump version to v0.55.3** ([5f173c2](https://github.com/solisoft/soli_lang/commit/5f173c28dc88ba903fdb8bcc5e1da953da811486))

## [0.55.4](https://github.com/solisoft/soli_lang/compare/0.55.3...0.55.4) (2026-04-13)

### Features
* **add mountable engines + dev error page script-escape fix** ([02d2605](https://github.com/solisoft/soli_lang/commit/02d2605aee86aa5bdcbead20bbb0e509bb073613))

### Other
* **bump version to v0.55.4** ([2322603](https://github.com/solisoft/soli_lang/commit/23226032046b5d8c8be09299476e37ae7fbccd6d))

## [0.55.5](https://github.com/solisoft/soli_lang/compare/0.55.4...0.55.5) (2026-04-13)

### Features
* **make string.split() separator optional, defaulting to " "** ([b52ab49](https://github.com/solisoft/soli_lang/commit/b52ab493db7dba3b92e09ab5991a8894b688bfd2))

### Other
* **bump version to v0.55.5** ([e3016b9](https://github.com/solisoft/soli_lang/commit/e3016b97e4f77185652ff8b3b3f3ef6cf44165e5))

## [0.55.6](https://github.com/solisoft/soli_lang/compare/0.55.5...0.55.6) (2026-04-14)

### Features
* **add model engine registry + JSON serialization fixes** ([b835370](https://github.com/solisoft/soli_lang/commit/b835370e44965b01f00592df9a122e84d8325cf5))

### Bug Fixes
* **use vec![] macro instead of Vec::with_capacity + push** ([f931049](https://github.com/solisoft/soli_lang/commit/f9310499a637fbe12cd7104e57eff87004c3919d))

### Other
* **bump version to v0.55.6** ([81a5839](https://github.com/solisoft/soli_lang/commit/81a58396c667f0e515a579f546cf2da6bc85132a))

## [0.56.0](https://github.com/solisoft/soli_lang/compare/0.55.6...0.56.0) (2026-04-16)

### Bug Fixes
* **re-trigger web font loading after live reload CSS updates** ([187126e](https://github.com/solisoft/soli_lang/commit/187126ea92c513d3bf9de8fc49dd1e8486efce62))

### Other
* **bump version to v0.56.0** ([6585057](https://github.com/solisoft/soli_lang/commit/6585057bea89166b65bf5e881616417991b884b1))

## [0.56.1](https://github.com/solisoft/soli_lang/compare/0.56.0...0.56.1) (2026-04-16)

### Other
* **bump version to v0.56.1** ([dad7ff3](https://github.com/solisoft/soli_lang/commit/dad7ff3b70eca26f5a5e6b71ad15ae0fab07081c))

## [0.57.0](https://github.com/solisoft/soli_lang/compare/0.56.1...0.57.0) (2026-04-17)

### Bug Fixes
* **preserve closure captures and super() calls** ([8ddcee5](https://github.com/solisoft/soli_lang/commit/8ddcee5db8755f898f39d090ce48e143cb419d21))
* **skip external stylesheets on live reload** ([130341d](https://github.com/solisoft/soli_lang/commit/130341daf49d3ef2bf850d9e4f63e6cdcea249c7))

### Performance
* **cheap-guard hash/string fast paths; bypass named-args for positional-only calls** ([6637141](https://github.com/solisoft/soli_lang/commit/66371414aa7e79559d69878a5af8a04b3d8d47f9))
* **collapse assign/is_const chain walks; O(1) server-listen check** ([ca21b12](https://github.com/solisoft/soli_lang/commit/ca21b12f3cc011bf64585c1cdf217a4f062df30d))
* **reduce interpreter allocations on hot paths** ([6f41fe0](https://github.com/solisoft/soli_lang/commit/6f41fe0e09d0d8bba0f78515cf8eba6a886ebee8))

### Other
* **bump version to v0.57.0** ([fd3ea47](https://github.com/solisoft/soli_lang/commit/fd3ea47fffdddbf6f1030ea23913a5c69fe32309))
* **fix clippy::collapsible_match lints for Rust 1.95** ([b25d490](https://github.com/solisoft/soli_lang/commit/b25d490215330250e931db82af4e5803ddb5a97b))

## [0.57.1](https://github.com/solisoft/soli_lang/compare/0.57.0...0.57.1) (2026-04-17)

### Bug Fixes
* **preserve UTF-8 in html_escape** ([48cb209](https://github.com/solisoft/soli_lang/commit/48cb209745d62d231cebc38754ff9bdab34390df))

### Other
* **bump version to v0.57.1** ([c7a4338](https://github.com/solisoft/soli_lang/commit/c7a43387b23dea5e4a1eb4e05152ecab1e3608f6))

## [0.58.0](https://github.com/solisoft/soli_lang/compare/0.57.1...0.58.0) (2026-04-18)

### Features
* **dispatch closure-taking array methods (map/filter/reduce/each)** ([1207100](https://github.com/solisoft/soli_lang/commit/12071007d9ae98af1aa0634de0c7bd10aaf8ca95))

### Performance
* **h[const_str] = v peephole + tighter array iter** ([22bbe24](https://github.com/solisoft/soli_lang/commit/22bbe24b0de4a9074f8659aed5a7a86de942d7d9))
* **precompute hash-literal keys at compile time** ([6b310ca](https://github.com/solisoft/soli_lang/commit/6b310cafca781775b099440307fc8ae5b0e719f7))
* **batched closure invocation in array methods** ([054b429](https://github.com/solisoft/soli_lang/commit/054b429e2460d5f5875084c245627f936a49d8d0))
* **drop upfront array clone in map/filter/reduce/each** ([878a2f2](https://github.com/solisoft/soli_lang/commit/878a2f2b13d6496819496449ca303bacf423fd2d))
* **close hash-literal, hash-compact, and bracket-access gaps** ([2d5c30d](https://github.com/solisoft/soli_lang/commit/2d5c30d28183a43761b0114d4db893f0208c1409))
* **speed up string interpolation formatting** ([89a24b7](https://github.com/solisoft/soli_lang/commit/89a24b781f1e1792d1d64c718997cfbf76cc5e9c))

### Other
* **bump version to v0.58.0** ([e21f582](https://github.com/solisoft/soli_lang/commit/e21f582d0de125af2dfb739ab56d9daf340b7e51))

### Tests
* **cover UTF-8 preservation in html_escape** ([0365ad3](https://github.com/solisoft/soli_lang/commit/0365ad3d9786e3d7c7950819f1e44b50befb4b1c))

### Styling
* **cargo fmt session commits** ([ca9cafa](https://github.com/solisoft/soli_lang/commit/ca9cafada3981b5bee89a3562dc4aa31b0ec5529))

## [0.58.1](https://github.com/solisoft/soli_lang/compare/0.58.0...0.58.1) (2026-04-18)

### Features
* **expose unified params as a global variable** ([0d4d2bf](https://github.com/solisoft/soli_lang/commit/0d4d2bfe729d07aabbe27ec53b1f1ef8913519bd))

### Bug Fixes
* **create cookie on lazy write and after regenerate** ([103c3f2](https://github.com/solisoft/soli_lang/commit/103c3f291979449634c71069fbbb13fc95ea71c1))

### Documentation
* **SDBQL raw-query patterns and safe binding guidance** ([b05ad03](https://github.com/solisoft/soli_lang/commit/b05ad03326bedac8e2ab84df091331195cee3f39))

### Other
* **bump version to v0.58.1** ([c124c0a](https://github.com/solisoft/soli_lang/commit/c124c0a38ad088f577ba7a4bb10af55bfd457438))

## [0.58.2](https://github.com/solisoft/soli_lang/compare/0.58.1...0.58.2) (2026-04-18)

### Documentation
* **index params global and req["all"] for search** ([30ee3d8](https://github.com/solisoft/soli_lang/commit/30ee3d8d9c95b8f487a1a263af7cc781c093ecc7))

### Other
* **bump version to v0.58.2** ([c37fe7a](https://github.com/solisoft/soli_lang/commit/c37fe7a78d9110ca8b97358638bb6126730c6159))

## [0.59.0](https://github.com/solisoft/soli_lang/compare/0.58.2...0.59.0) (2026-04-18)

### Features
* **add pluggable storage backends (disk, solidb, solikv)** ([3c821e0](https://github.com/solisoft/soli_lang/commit/3c821e00bc85afa1d04f78ed7ce5f7da58aae7a6))

### Other
* **bump version to v0.59.0** ([3b8c156](https://github.com/solisoft/soli_lang/commit/3b8c1563b92f529807ae5e304f471cbf0535c2ec))

## [0.60.0](https://github.com/solisoft/soli_lang/compare/0.59.0...0.60.0) (2026-04-18)

### Bug Fixes
* **bind `this` when dispatching class-based controller actions** ([492df8f](https://github.com/solisoft/soli_lang/commit/492df8f9b98f4fd0807a851c9963b6fd079adf1b))

### Other
* **bump version to v0.60.0** ([7cb7cc2](https://github.com/solisoft/soli_lang/commit/7cb7cc2290f97decd0d65eed33ed9bb2824ee5fc))
* **rebuild Tailwind CSS (solid bg utilities)** ([10ab8da](https://github.com/solisoft/soli_lang/commit/10ab8da7d0e1a624dbe9443d6d3825e9f2a3cd91))

### Styling
* **apply cargo fmt to controller dispatch test** ([769f6d4](https://github.com/solisoft/soli_lang/commit/769f6d45d5f54466ce0ca6a95e16ce4a7a31f9cb))

## [0.61.0](https://github.com/solisoft/soli_lang/compare/0.60.0...0.61.0) (2026-04-19)

### Features
* **add `partial(...)` alias for `render_partial(...)`** ([406c81d](https://github.com/solisoft/soli_lang/commit/406c81d67c1e5f707105f9f4b1ea9eb76d9b607f))
* **default global `params` to {} instead of null** ([a333b1a](https://github.com/solisoft/soli_lang/commit/a333b1ac24832dd0e800b2c471c1f0560cd4fbb8))

### Bug Fixes
* **stop masking Model.count errors as 0** ([9a8f5c6](https://github.com/solisoft/soli_lang/commit/9a8f5c645c15e2802903d856d1c4308a2058e493))
* **middleware dev error pages show the middleware's file** ([40d628b](https://github.com/solisoft/soli_lang/commit/40d628bc1d7c7ef031ecb6bc28a67b45825a46c5))

### Documentation
* **prefer partial(...) over render_partial(...) in examples** ([c7bf221](https://github.com/solisoft/soli_lang/commit/c7bf2210bb8e0edcb057ca712f8ed02cdab586ee))
* **document <%- %> and <%== %> ERB output tags** ([37903e0](https://github.com/solisoft/soli_lang/commit/37903e01c807480b130b3cf6f69f0ef8517c58aa))

### Other
* **bump version to v0.61.0** ([c1313d3](https://github.com/solisoft/soli_lang/commit/c1313d3af30614353d86d3debaf9cfdf2f8f6388))

### Tests
* **cover parse_count_result shape handling** ([5bfc7d9](https://github.com/solisoft/soli_lang/commit/5bfc7d9c7af97d9b8f892d981d7fb6da6781bff9))

## [0.62.0](https://github.com/solisoft/soli_lang/compare/0.61.0...0.62.0) (2026-04-19)

### Bug Fixes
* **use COLLECTION_COUNT/LENGTH for counts** ([67e9f5c](https://github.com/solisoft/soli_lang/commit/67e9f5caaac326a2ce796f69abc7d788878de19a))

### Other
* **bump version to v0.62.0** ([0136070](https://github.com/solisoft/soli_lang/commit/0136070972089f82bc127f4daacf132055abf95f))

## [0.63.0](https://github.com/solisoft/soli_lang/compare/0.62.0...0.63.0) (2026-04-20)

### Features
* **add style/redundant-model-import rule** ([dc40819](https://github.com/solisoft/soli_lang/commit/dc4081914fd43e60663535366877336d223e51f2))

### Documentation
* **drop free-function merge(h1, h2) form** ([7b40eda](https://github.com/solisoft/soli_lang/commit/7b40eda486c9e6ecc6e12c1b21ab9cf2ca3b60e4))

### Other
* **bump version to v0.63.0** ([6569369](https://github.com/solisoft/soli_lang/commit/65693697bb2bef4cc82472008eeb9524d01010f9))

## [0.63.1](https://github.com/solisoft/soli_lang/compare/0.63.0...0.63.1) (2026-04-20)

### Bug Fixes
* **pass through AQL function calls in where() filters** ([f65c5c4](https://github.com/solisoft/soli_lang/commit/f65c5c4ff79b6bb36a2047ff74e7429d7efdcb8c))
* **canonicalize folder so hot-reload classifies events correctly** ([3eeb467](https://github.com/solisoft/soli_lang/commit/3eeb467ed103625ae2c54a9973a63657e4711b26))

### Other
* **bump version to v0.63.1** ([3399087](https://github.com/solisoft/soli_lang/commit/3399087509c4f6a88c65f6a14f96c725820ad2fd))

## [0.63.2](https://github.com/solisoft/soli_lang/compare/0.63.1...0.63.2) (2026-04-20)

### Features
* **add dynamic find_by_* methods for ORM** ([5f11fba](https://github.com/solisoft/soli_lang/commit/5f11fbae0f646d1e094588995c751dce366a6ff8))

### Other
* **bump version to v0.63.2** ([35dacf1](https://github.com/solisoft/soli_lang/commit/35dacf19e8662efb6b8e1d8c32f0860f6eec9d98))

## [0.64.0](https://github.com/solisoft/soli_lang/compare/0.63.2...0.64.0) (2026-04-21)

### Features
* **expose req and params in templates** ([9354eb5](https://github.com/solisoft/soli_lang/commit/9354eb5b74004b8994bcc3ac1cb95c7dadfc055d))

### Bug Fixes
* **inject req/params into partials rendering** ([67805a8](https://github.com/solisoft/soli_lang/commit/67805a8acfbd223e9d882c0e4f7d98cef2bfafb2))

### Other
* **bump version to v0.64.0** ([ebe0839](https://github.com/solisoft/soli_lang/commit/ebe083973663b73d5cc665f4cb242c64304bd3c4))
* **Optimize GetAndIncrLocal/GetAndDecrLocal to use mem::replace** ([dae9cf4](https://github.com/solisoft/soli_lang/commit/dae9cf4d6ffdd94ffc9bba09d046512967326b55))
* **Avoid blocking async runtime by creating dedicated single-thread runtime** ([f815b12](https://github.com/solisoft/soli_lang/commit/f815b120d73ec275fb40752d7968bed7b563922f))
* **Replace regex cache full-clear with LRU eviction** ([a5d50f1](https://github.com/solisoft/soli_lang/commit/a5d50f1ef19f3a9c8729d112ce009d010af6b8f6))
* **Cache compiled bytecode in CompiledModule cache for run_vm()** ([ed58e3c](https://github.com/solisoft/soli_lang/commit/ed58e3ca398c46823bbc7f414d2668a17276733a))
* **Cache JIT-compiled bytecode in Function struct** ([f0b3e95](https://github.com/solisoft/soli_lang/commit/f0b3e95bc9a250490cf065c9df1adf17565bf535))
* **Fix ORM eager-loading: lazy conversion to model instances on field access** ([889845b](https://github.com/solisoft/soli_lang/commit/889845b72a992764710a2ad489666155b90f26e8))

## [0.64.1](https://github.com/solisoft/soli_lang/compare/0.64.0...0.64.1) (2026-04-21)

### Other
* **bump version to v0.64.1** ([0cde46f](https://github.com/solisoft/soli_lang/commit/0cde46f10f9b538cd29d6bff3c16c2f7a39b514a))

### Styling
* **apply rustfmt** ([49a3729](https://github.com/solisoft/soli_lang/commit/49a372949f365eaada6f3c62d6f9978b6cb6fac2))

## [0.64.3](https://github.com/solisoft/soli_lang/compare/0.64.1...0.64.3) (2026-04-21)

### Features
* **add mock query infrastructure for integration testing** ([7a07818](https://github.com/solisoft/soli_lang/commit/7a078186d0b8256befe7b321c4d31b37c2db72c2))

### Bug Fixes
* **derive relation class from _id field instead of owner class** ([f67887a](https://github.com/solisoft/soli_lang/commit/f67887a54a7000ff1f3e4379349968b34911c972))

### Documentation
* **fix changelog for versions 0.58-0.64** ([bd7c496](https://github.com/solisoft/soli_lang/commit/bd7c496f4773bd46549298e25bd95d4047e1af25))
* **update CHANGELOG for v0.64.1 and v0.63.1** ([e8547d9](https://github.com/solisoft/soli_lang/commit/e8547d976b32da1212501e086611eceb30c84704))

### Other
* **bump version to v0.64.3** ([35ef941](https://github.com/solisoft/soli_lang/commit/35ef94167e6cece61f220593e1d57bdc1e8501c3))

### Tests
* **add integration tests for model includes class derivation** ([10bba11](https://github.com/solisoft/soli_lang/commit/10bba115e90b6ca8a08ae10ff6506871376f99cb))
* **add tests for _id-based class derivation in includes** ([c2787c9](https://github.com/solisoft/soli_lang/commit/c2787c947e4131c0457aa6aef1e935dd73843362))

## [0.64.4](https://github.com/solisoft/soli_lang/compare/0.64.3...0.64.4) (2026-04-21)

### Bug Fixes
* **model static method binding to use variadic arity** ([416ef81](https://github.com/solisoft/soli_lang/commit/416ef81277240d3bfb777525bb53c89e7c73e2c0))

### Other
* **bump version to v0.64.4** ([e40065d](https://github.com/solisoft/soli_lang/commit/e40065d2739405b0b995df007d43356c70ba9877))

## [0.64.5](https://github.com/solisoft/soli_lang/compare/0.64.4...0.64.5) (2026-04-21)

### Features
* **implement is_a? on all instances** ([46ae6f2](https://github.com/solisoft/soli_lang/commit/46ae6f2a8c0ff901f77c8dc5216e16303fd5f27a))

### Other
* **bump version to v0.64.5** ([816d151](https://github.com/solisoft/soli_lang/commit/816d151424fa39ca6ea91921d7e344fdbac31d32))

## [0.65.0](https://github.com/solisoft/soli_lang/compare/0.64.5...0.65.0) (2026-04-21)

### Documentation
* **update is_a? documentation to clarify dual behavior** ([28172da](https://github.com/solisoft/soli_lang/commit/28172da9edecb47c29045784fb94963e613cd8f9))

### Other
* **bump version to v0.65.0** ([ab21a51](https://github.com/solisoft/soli_lang/commit/ab21a515534138d3198bdb8106a91271935cbf14))
* **release prep** ([7c0427b](https://github.com/solisoft/soli_lang/commit/7c0427bf18913744f4f1d94010aafe4c5b2be518))

## [0.65.1](https://github.com/solisoft/soli_lang/compare/0.65.0...0.65.1) (2026-04-21)

### Documentation
* **update skills to include .sl tests** ([7fa83f3](https://github.com/solisoft/soli_lang/commit/7fa83f3d2c2107c8ac0a5ef006085941cec9a98b))

### Other
* **bump version to v0.65.1** ([3457b25](https://github.com/solisoft/soli_lang/commit/3457b254fcca641a6753e32cf46f5c45957a7fb4))

## [0.66.0](https://github.com/solisoft/soli_lang/compare/0.65.1...0.66.0) (2026-04-21)

### Other
* **bump version to v0.66.0** ([57c7562](https://github.com/solisoft/soli_lang/commit/57c7562a445e56bf198713aa156d7c3b62a7e17d))
* **release v0.65.2** ([a4f84e1](https://github.com/solisoft/soli_lang/commit/a4f84e1bb832851c426b82654f98f407670de641))

## [0.67.0](https://github.com/solisoft/soli_lang/compare/0.66.0...0.67.0) (2026-04-21)

### Documentation
* **update release skill to include changelog updates** ([c620e99](https://github.com/solisoft/soli_lang/commit/c620e992ef32f9d389b20fa5ec0d663b8c143195))

### Other
* **bump version to v0.67.0** ([88d1402](https://github.com/solisoft/soli_lang/commit/88d1402419391c54bbc0db5a6efb60bf6ee8e95a))
* **release v0.67.0** ([bbca3b2](https://github.com/solisoft/soli_lang/commit/bbca3b228a458376e9aaad8b75657622ba0cf65a))
* **reorganize skills and update docs with spreadsheet functions** ([08ab36a](https://github.com/solisoft/soli_lang/commit/08ab36a638e0e7f192ba4a5581bad4c7db8260ec))

## [0.67.1](https://github.com/solisoft/soli_lang/compare/0.67.0...0.67.1) (2026-04-21)

### Bug Fixes
* **coverage percentage calculation was inflated due to double-counting hits in end_test()** ([12de611](https://github.com/solisoft/soli_lang/commit/12de611f06f6766adb11875a005dfda52d98c4e6))

### Other
* **bump version to v0.67.1** ([053bd40](https://github.com/solisoft/soli_lang/commit/053bd40b83105144fb5ccf3928be1243286653c0))

## [0.67.2](https://github.com/solisoft/soli_lang/compare/0.67.1...0.67.2) (2026-04-21)

### Features
* **add expect().to_*() chainable assertions, global coverage tracker for test server, exclude tests/ from coverage, relative paths** ([3b46d8c](https://github.com/solisoft/soli_lang/commit/3b46d8cf4a5a8df3cbc518ecffa8eb3a249ceddd))

### Other
* **bump version to v0.67.2** ([23c5326](https://github.com/solisoft/soli_lang/commit/23c5326c38ed84f56d898638f71dfa39d3751250))

## [0.68.0](https://github.com/solisoft/soli_lang/compare/0.67.2...0.68.0) (2026-04-21)

### Bug Fixes
* **resolve deadlock and path issues in coverage tracking** ([571480f](https://github.com/solisoft/soli_lang/commit/571480f48ad747163ea76560182701621a0eb7ca))

### Other
* **bump version to v0.68.0** ([0a44270](https://github.com/solisoft/soli_lang/commit/0a44270ef4a1d2cad3b9b140148f8d409fbe3883))

## [0.68.1](https://github.com/solisoft/soli_lang/compare/0.68.0...0.68.1) (2026-04-21)

### Other
* **bump version to v0.68.1** ([13721d3](https://github.com/solisoft/soli_lang/commit/13721d3327b1e47c84534da3f43fa40ecc74e6fa))

## [0.68.2](https://github.com/solisoft/soli_lang/compare/0.68.1...0.68.2) (2026-04-22)

### Other
* **bump version to v0.68.2** ([e7ce40f](https://github.com/solisoft/soli_lang/commit/e7ce40fd0749e147ffaa46855a9af6eba414350e))
* **Fix coverage tracking and test assertion counting for do...end blocks** ([1cfb827](https://github.com/solisoft/soli_lang/commit/1cfb8277fe806cec8e0cee9ac25d63715df54bae))

## [0.69.0](https://github.com/solisoft/soli_lang/compare/0.68.2...0.69.0) (2026-04-22)

### Other
* **bump version to v0.69.0** ([63382d5](https://github.com/solisoft/soli_lang/commit/63382d53cf8fa9a1f6a456cdbd71c13ff8dd1a9e))
* **release v0.69.0** ([68bcf8a](https://github.com/solisoft/soli_lang/commit/68bcf8a4a72ed6ac3641746edfe54fb5ed11d7ba))

## [0.70.0](https://github.com/solisoft/soli_lang/compare/0.69.0...0.70.0) (2026-04-22)

### Other
* **bump version to v0.70.0** ([57620f7](https://github.com/solisoft/soli_lang/commit/57620f71b937aac40c24cd30310a903f892bca47))
* **release v0.70.0** ([2c40600](https://github.com/solisoft/soli_lang/commit/2c4060084ddd63c49e7ef64f084865cd3aaf775e))

## [0.71.0](https://github.com/solisoft/soli_lang/compare/0.70.0...0.71.0) (2026-04-22)

### Other
* **bump version to v0.71.0** ([c6d537f](https://github.com/solisoft/soli_lang/commit/c6d537f0b5121ac0153536049e3b4176cf836053))
* **release v0.71.0** ([961b755](https://github.com/solisoft/soli_lang/commit/961b755db999c5cfbabc458367026b847051b6d4))

## [0.72.0](https://github.com/solisoft/soli_lang/compare/0.71.0...0.72.0) (2026-04-22)

### Other
* **bump version to v0.72.0** ([b360fca](https://github.com/solisoft/soli_lang/commit/b360fca7099664bce342181c40f939c3880e63fb))
* **add @sigil syntax, expect() assertions, and view locals injection** ([5e62d5f](https://github.com/solisoft/soli_lang/commit/5e62d5f80054cd7ce0a5f4257b958d9cdf84b58b))

## [0.72.1](https://github.com/solisoft/soli_lang/compare/0.72.0...0.72.1) (2026-04-22)

### Other
* **bump version to v0.72.1** ([0440fdc](https://github.com/solisoft/soli_lang/commit/0440fdca15059bd298c49b04774ca48d23c4e8ed))

## [0.72.2](https://github.com/solisoft/soli_lang/compare/0.72.1...0.72.2) (2026-04-22)

### Features
* **add current_path(), current_method(), and current_path?() view helpers** ([1c02f93](https://github.com/solisoft/soli_lang/commit/1c02f934a7a21a9eff34c426ff7538d0dffcf91c))

### Other
* **bump version to v0.72.2** ([f92450b](https://github.com/solisoft/soli_lang/commit/f92450bc48e79a9e179a81d500bf0af015663ecd))

## [0.73.0](https://github.com/solisoft/soli_lang/compare/0.72.2...0.73.0) (2026-04-22)

### Features
* **add call-assignment desugaring for filtered controller hooks** ([9523ce1](https://github.com/solisoft/soli_lang/commit/9523ce1ba4fae985b364ee97cfe0b8f2faa2f487))

### Other
* **bump version to v0.73.0** ([02fb52d](https://github.com/solisoft/soli_lang/commit/02fb52d6bc820cd7b20a516dc6d6222ed7f724a5))

## [0.74.0](https://github.com/solisoft/soli_lang/compare/0.73.0...0.74.0) (2026-04-22)

### Bug Fixes
* **bind this in hooks so @sigil writes reach the controller instance** ([3be976e](https://github.com/solisoft/soli_lang/commit/3be976e6573fb84c9ba5db28489de8327d0a33ad))

### Other
* **bump version to v0.74.0** ([9db407e](https://github.com/solisoft/soli_lang/commit/9db407e6dd4e36b89e049336d4703fe7244f0e25))
* **release v0.65.0** ([991873d](https://github.com/solisoft/soli_lang/commit/991873da797102001b6d3573017b274538cae786))

## [0.75.0](https://github.com/solisoft/soli_lang/compare/0.74.0...0.75.0) (2026-04-22)

### Features
* **add hover-preload script injected into HTML responses** ([3e4e9a9](https://github.com/solisoft/soli_lang/commit/3e4e9a90835d06c115ef73990f2c2cdf18f4c20e))

### Bug Fixes
* **surface handler errors instead of swallowing them; clear __result before each hook** ([95dcd7b](https://github.com/solisoft/soli_lang/commit/95dcd7b4c8a5fb36de0cfbe30ea34eec7cb52edd))

### Other
* **bump version to v0.75.0** ([4edea91](https://github.com/solisoft/soli_lang/commit/4edea91e0f42b786afc0ca72649cbb18938c3ede))
* **release v0.75.0** ([d104665](https://github.com/solisoft/soli_lang/commit/d104665ca0c8e039bf99d06892776d4e0f462ca2))

## [0.76.0](https://github.com/solisoft/soli_lang/compare/0.75.0...0.76.0) (2026-04-23)

### Bug Fixes
* **handle empty hook bodies; rescan metadata on hot-reload** ([0965817](https://github.com/solisoft/soli_lang/commit/0965817a89f6f470b62db8f4128d0d9594b54c02))

### Other
* **bump version to v0.76.0** ([ec5a6c5](https://github.com/solisoft/soli_lang/commit/ec5a6c5240cc61f0234aff0e3c8b4bd84c1c716b))
* **update CHANGELOG for v0.76.0** ([4f9f53d](https://github.com/solisoft/soli_lang/commit/4f9f53df60726e0d21352fb0bd90143db8afe1b1))

## [0.76.1](https://github.com/solisoft/soli_lang/compare/0.76.0...0.76.1) (2026-04-23)

### Bug Fixes
* **normalize options hash for HTTP.request method** ([5ee31e2](https://github.com/solisoft/soli_lang/commit/5ee31e220e3ea01f2e4f574174f18d549dab1f89))

### Other
* **bump version to v0.76.1** ([31857a6](https://github.com/solisoft/soli_lang/commit/31857a654d9b57162a4cf2e378c3c0d7ac8088b2))

## [0.76.2](https://github.com/solisoft/soli_lang/compare/0.76.1...0.76.2) (2026-04-23)

### Bug Fixes
* **use navigator.connection data-saving flag to skip low-value links** ([c1bd548](https://github.com/solisoft/soli_lang/commit/c1bd54823419be24bea082538268995bf47d323c))

### Other
* **bump version to v0.76.2** ([1a30c21](https://github.com/solisoft/soli_lang/commit/1a30c21b123b613a06f4563d9a0418e718abc276))
* **update CHANGELOG for v0.76.2** ([eafa5e6](https://github.com/solisoft/soli_lang/commit/eafa5e6a4aaa0faf95e2b9e3ae36cfd99eeb1682))

## [0.77.0](https://github.com/solisoft/soli_lang/compare/0.76.2...0.77.0) (2026-04-23)

### Features
* **add defined() builtin to check if a variable exists** ([89e3755](https://github.com/solisoft/soli_lang/commit/89e375504de9865f1156de6b6ddde178e706b121))

### Documentation
* **add defined() builtin documentation** ([eecb0a4](https://github.com/solisoft/soli_lang/commit/eecb0a4e166f4d16637c8c3602544c5117b25781))

### Other
* **bump version to v0.77.0** ([c17e3f1](https://github.com/solisoft/soli_lang/commit/c17e3f1de48da9b622b0bb5f473960afa5af4a7c))
* **update CHANGELOG for v0.77.0** ([bd4da23](https://github.com/solisoft/soli_lang/commit/bd4da23a48cb675760bd0a314572f083427525e3))

## [0.78.0](https://github.com/solisoft/soli_lang/compare/0.77.0...0.78.0) (2026-04-23)

### Features
* **add universal methods (nil?, blank?, present?) on function values** ([b2e66fe](https://github.com/solisoft/soli_lang/commit/b2e66fe0a59b3fe430a58fede1d457104a517404))

### Bug Fixes
* **add Function to VM property access and type checker for universal methods** ([b7028e2](https://github.com/solisoft/soli_lang/commit/b7028e2ed791215bb40288727396756da59905d0))

### Other
* **bump version to v0.78.0** ([3f07518](https://github.com/solisoft/soli_lang/commit/3f075182110caf209f1c0f6ce3b054ce396202ed))
* **update CHANGELOG for v0.78.0** ([423a306](https://github.com/solisoft/soli_lang/commit/423a3060df1d41cf5a0b15b920801a2d671f45b0))

### Styling
* **format vm_classes.rs** ([8793e1d](https://github.com/solisoft/soli_lang/commit/8793e1d73464021bb1cbd249f75986eab68c9c8a))

## [0.78.1](https://github.com/solisoft/soli_lang/compare/0.78.0...0.78.1) (2026-04-23)

### Refactoring
* **rename error() builtin to halt() to avoid collisions with local variables** ([d2cc358](https://github.com/solisoft/soli_lang/commit/d2cc3588e53e130dbce0a5615d409eddc2435fb2))

### Other
* **bump version to v0.78.1** ([435e17e](https://github.com/solisoft/soli_lang/commit/435e17e16ed16fa6225b5416f17e2081632e002b))
* **update CHANGELOG for v0.78.1** ([034e6e8](https://github.com/solisoft/soli_lang/commit/034e6e88d0f84409048fe1424ad6eee7f188659e))
* **remove debug println from controller registry** ([c60d83f](https://github.com/solisoft/soli_lang/commit/c60d83f4a1659591ea733ec9466fb35761681221))

## [0.79.0](https://github.com/solisoft/soli_lang/compare/0.78.1...0.79.0) (2026-04-23)

### Features
* **add comment handling to static block extraction, controller inheritance, after_action hooks, and defensive partial tests** ([699a32a](https://github.com/solisoft/soli_lang/commit/699a32a1fa266cea03292bf956db9525c26bdcdb))

### Other
* **bump version to v0.79.0** ([11f2175](https://github.com/solisoft/soli_lang/commit/11f2175103f74d64449e83be1dc105a57b02516e))
* **update CHANGELOG for unreleased changes** ([5430ee2](https://github.com/solisoft/soli_lang/commit/5430ee27ff03ff18efc2740bc2aa460757114e60))
