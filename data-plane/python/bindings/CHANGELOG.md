# Changelog

## [0.8.0](https://github.com/agntcy/slim/compare/slim-bindings-python-v0.7.1...slim-bindings-python-v0.8.0) (2026-01-30)


### ⚠ BREAKING CHANGES

* session layer APIs updated.

### Features

* **bindings:** configuration file support ([#1099](https://github.com/agntcy/slim/issues/1099)) ([d06d09b](https://github.com/agntcy/slim/commit/d06d09b4e75abf5b682cce626994f3ee32c7c44c))
* **bindings:** expose complete configuration for auth and creating clients, servers ([#1084](https://github.com/agntcy/slim/issues/1084)) ([ac6240f](https://github.com/agntcy/slim/commit/ac6240fb0b40fbf1dcf78c5be1512097c9a0024d))
* **bindings:** expose completion handle to foreign language ([#1090](https://github.com/agntcy/slim/issues/1090)) ([a70bbfa](https://github.com/agntcy/slim/commit/a70bbfa0df1a74d6e085fec5e0cd3e94d1d4c2d9))
* **bindings:** expose global and local service ([#1095](https://github.com/agntcy/slim/issues/1095)) ([cab992b](https://github.com/agntcy/slim/commit/cab992bf48791f83d16e6e0a5cf60d095df03c62))
* **bindings:** expose identity configuration ([#1092](https://github.com/agntcy/slim/issues/1092)) ([8e417ce](https://github.com/agntcy/slim/commit/8e417ce177e347f7945a80e1c720c68c0250460b))
* **bindings:** move to new python bindings ([#1116](https://github.com/agntcy/slim/issues/1116)) ([64c8f24](https://github.com/agntcy/slim/commit/64c8f245991a29bbe3a6d98505adfba8123ce710))
* enable spire as token provider for clients ([#945](https://github.com/agntcy/slim/issues/945)) ([6d8f65f](https://github.com/agntcy/slim/commit/6d8f65f329098e31b9f62bbaeaadb58b50f60b20))
* expand SharedSecret Auth from simple secret:id to HMAC tokens ([#858](https://github.com/agntcy/slim/issues/858)) ([1d0c6e8](https://github.com/agntcy/slim/commit/1d0c6e8f694ee15ac437564fe6400b4d7bd4dde1))
* Go bindings generation using uniffi ([#979](https://github.com/agntcy/slim/issues/979)) ([5b7d813](https://github.com/agntcy/slim/commit/5b7d813a95528788d87614502b170a63b6369812))
* Golang binding improvements ([#1032](https://github.com/agntcy/slim/issues/1032)) ([f55424d](https://github.com/agntcy/slim/commit/f55424d43901c1e3cffe17952b46444d6ee0e21d))
* implementation of Identity provider client credential flow ([#464](https://github.com/agntcy/slim/issues/464)) ([504b1dd](https://github.com/agntcy/slim/commit/504b1dd1516a9f0b00be89af658f2f4762e36e7e))
* improve point to point session with sender/receiver buffer ([#735](https://github.com/agntcy/slim/issues/735)) ([e6f65bb](https://github.com/agntcy/slim/commit/e6f65bb9d6584994538027dd4db45429d74821ea))
* Integrate SPIRE-based mTLS & identity, unify TLS sources, enhance gRPC config, and add flexible metadata support ([#892](https://github.com/agntcy/slim/issues/892)) ([a86bfd6](https://github.com/agntcy/slim/commit/a86bfd64a75d7f5aba2b440d58fa8f3d5bd3a8a0))
* make backoff retry configurable ([#991](https://github.com/agntcy/slim/issues/991)) ([9392edd](https://github.com/agntcy/slim/commit/9392edddefb9bd694ac0117fbceb12cf1090b7fa))
* move session code in a new crate ([#828](https://github.com/agntcy/slim/issues/828)) ([6d0cf90](https://github.com/agntcy/slim/commit/6d0cf90a67cdfd62039fee857cf103a52a0380b1))
* **multicast:** remove moderator parameter from configuration ([#739](https://github.com/agntcy/slim/issues/739)) ([464d523](https://github.com/agntcy/slim/commit/464d523205a6f972e633eddd842c007929bb7974))
* **pysession:** expose session type, src and dst names ([#737](https://github.com/agntcy/slim/issues/737)) ([1c16ccc](https://github.com/agntcy/slim/commit/1c16ccc74d4b0572a424369223320bf8a52269c2))
* **python-bindings:** Remove Py prefix from the python names ([#931](https://github.com/agntcy/slim/issues/931)) ([5ffb2cc](https://github.com/agntcy/slim/commit/5ffb2cca4bcb3c9c2f6985e8bff8e8e223266585))
* **python-bindings:** Remove remaining Py... names from python bindings ([#954](https://github.com/agntcy/slim/issues/954)) ([da76c70](https://github.com/agntcy/slim/commit/da76c70f766c165bf91f81e24c0aa0fc06bbb8f9))
* **python/bindings:** create a unique SLIM data-plane instance per process by default ([#819](https://github.com/agntcy/slim/issues/819)) ([728290c](https://github.com/agntcy/slim/commit/728290c809d2f378dd939905af565c3ed483ebcd))
* **python/bindings:** improve documentation ([#748](https://github.com/agntcy/slim/issues/748)) ([88c43d8](https://github.com/agntcy/slim/commit/88c43d8a39acc8457fa9ed8344dac7ea85821887))
* **python/bindings:** improve publish function ([#749](https://github.com/agntcy/slim/issues/749)) ([85fd2ca](https://github.com/agntcy/slim/commit/85fd2ca2e24794998203fd25b51964eabc10c04e))
* **python/bindings:** remove request-reply API ([#677](https://github.com/agntcy/slim/issues/677)) ([65cec9d](https://github.com/agntcy/slim/commit/65cec9d9fc4439a696aadae2edad940792a52fa1))
* **python/examples:** allow each participant to publish ([#778](https://github.com/agntcy/slim/issues/778)) ([0a28d9d](https://github.com/agntcy/slim/commit/0a28d9d0c02adb08065e56043491208a638e2661))
* refactor session receive() API ([#731](https://github.com/agntcy/slim/issues/731)) ([787d111](https://github.com/agntcy/slim/commit/787d111d030de5768385b72ea7a794ced85d6652))
* **session:** graceful session draining + reliable blocking API completion ([#924](https://github.com/agntcy/slim/issues/924)) ([5ae9e80](https://github.com/agntcy/slim/commit/5ae9e806ae72ec465dfa6fe3da2c562fa5d73e7c))
* **session:** introduce session metadata ([#744](https://github.com/agntcy/slim/issues/744)) ([14528ee](https://github.com/agntcy/slim/commit/14528eec79e31e0729b3f305a8da5bc38ab0ac51))
* Update task files to generate coverage for python bindings ([#849](https://github.com/agntcy/slim/issues/849)) ([e45ad47](https://github.com/agntcy/slim/commit/e45ad47357b1008938edee3a7decb78f84bee5a4))


### Bug Fixes

* **app.rs:** get app name from local property ([#859](https://github.com/agntcy/slim/issues/859)) ([5918912](https://github.com/agntcy/slim/commit/591891219ebea0605a22abdbb292c29aa486073c))
* **bindings:** improve identity error handling ([#1042](https://github.com/agntcy/slim/issues/1042)) ([44002b5](https://github.com/agntcy/slim/commit/44002b51c598f3780645b8f3fac48f5e34a373cb))
* **bindings:** Make sure type hinting is working ([#920](https://github.com/agntcy/slim/issues/920)) ([380030e](https://github.com/agntcy/slim/commit/380030e4f3b7b2d2c3ac2a07c4c269fc4a98920e))
* flaky integration test ([#981](https://github.com/agntcy/slim/issues/981)) ([45a7eaa](https://github.com/agntcy/slim/commit/45a7eaada2a211280cdbdfb5739597d958e88e6b))
* **python-bindings:** default crypto provider initialization for Reqwest crate ([#706](https://github.com/agntcy/slim/issues/706)) ([16a71ce](https://github.com/agntcy/slim/commit/16a71ced6164e4b6df7953f897b8f195fd56b097))
* **python-bindings:** remove destination_name property ([#751](https://github.com/agntcy/slim/issues/751)) ([ab651da](https://github.com/agntcy/slim/commit/ab651da1a1d830a857a6a370d9cc66e2f6d737d5))
* **python/bindings:** add missing PyMessageContext type export ([#841](https://github.com/agntcy/slim/issues/841)) ([6301ced](https://github.com/agntcy/slim/commit/6301ced85073c028898d60eb620dacb0cff6afbf))
* **service:** disconnect API ([#890](https://github.com/agntcy/slim/issues/890)) ([4308cc4](https://github.com/agntcy/slim/commit/4308cc4d1bcd4a97bb4d6461d9286cc3b2a21e00))
* **session:** correctly handle multiple subscriptions ([#838](https://github.com/agntcy/slim/issues/838)) ([52b49aa](https://github.com/agntcy/slim/commit/52b49aa389f87f8ffdec345aaf5313a2e90998a5))
* **session:** prevent session queue saturation ([#903](https://github.com/agntcy/slim/issues/903)) ([3ba44eb](https://github.com/agntcy/slim/commit/3ba44eb7d15f129efdb9499806c544d829f25409))
* **session:** route dataplane errors to correct session ([#1056](https://github.com/agntcy/slim/issues/1056)) ([0fb9042](https://github.com/agntcy/slim/commit/0fb90425a7a54c0dea743f97f6f2f998e02facad))


### Documentation

* **python/bindings:** add documentantion for sessions and example ([#750](https://github.com/agntcy/slim/issues/750)) ([04f1d0f](https://github.com/agntcy/slim/commit/04f1d0f583698e94394b86f73445532c328a7796))
* update readmes in python examples ([#962](https://github.com/agntcy/slim/issues/962)) ([1e8472d](https://github.com/agntcy/slim/commit/1e8472dd356a4d9eef3570bba20971e363da9216))
