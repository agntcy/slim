# Changelog

## [0.6.0](https://github.com/agntcy/slim/compare/slim-bindings-examples-v0.1.1...slim-bindings-examples-v0.6.0) (2025-10-09)


### ⚠ BREAKING CHANGES

* session layer APIs updated.

### Features

* **multicast:** remove moderator parameter from configuration ([#739](https://github.com/agntcy/slim/issues/739)) ([464d523](https://github.com/agntcy/slim/commit/464d523205a6f972e633eddd842c007929bb7974))
* **pysession:** expose session type, src and dst names ([#737](https://github.com/agntcy/slim/issues/737)) ([1c16ccc](https://github.com/agntcy/slim/commit/1c16ccc74d4b0572a424369223320bf8a52269c2))
* **python/bindings:** improve documentation ([#748](https://github.com/agntcy/slim/issues/748)) ([88c43d8](https://github.com/agntcy/slim/commit/88c43d8a39acc8457fa9ed8344dac7ea85821887))
* **python/bindings:** improve publish function ([#749](https://github.com/agntcy/slim/issues/749)) ([85fd2ca](https://github.com/agntcy/slim/commit/85fd2ca2e24794998203fd25b51964eabc10c04e))
* **python/bindings:** remove request-reply API ([#677](https://github.com/agntcy/slim/issues/677)) ([65cec9d](https://github.com/agntcy/slim/commit/65cec9d9fc4439a696aadae2edad940792a52fa1))
* **python/bindings:** update multicast example readme ([#788](https://github.com/agntcy/slim/issues/788)) ([46ce07e](https://github.com/agntcy/slim/commit/46ce07e3467cf10cee143e30d7e5c73920c1f4b9))
* **python/examples:** allow each participant to publish ([#778](https://github.com/agntcy/slim/issues/778)) ([0a28d9d](https://github.com/agntcy/slim/commit/0a28d9d0c02adb08065e56043491208a638e2661))
* refactor session receive() API ([#731](https://github.com/agntcy/slim/issues/731)) ([787d111](https://github.com/agntcy/slim/commit/787d111d030de5768385b72ea7a794ced85d6652))


### Bug Fixes

* create a new JWKS file containg all keys from all trust domains ([#776](https://github.com/agntcy/slim/issues/776)) ([ae900d4](https://github.com/agntcy/slim/commit/ae900d4c1e7a5f178e642c87e08ed8ab2d20c719))
* **python/examples:** correcly close multicast example ([#786](https://github.com/agntcy/slim/issues/786)) ([fa7de9c](https://github.com/agntcy/slim/commit/fa7de9cfcc872749b6cbf37568fd5a38b91191e2))


### Documentation

* fix readmes for python bindings examples ([#764](https://github.com/agntcy/slim/issues/764)) ([4d29cd8](https://github.com/agntcy/slim/commit/4d29cd8b5622a84ec4e06ecb796acc07906c93bc))
* **python/bindings:** add documentantion for sessions and example ([#750](https://github.com/agntcy/slim/issues/750)) ([04f1d0f](https://github.com/agntcy/slim/commit/04f1d0f583698e94394b86f73445532c328a7796))

## [0.1.1](https://github.com/agntcy/slim/compare/slim-bindings-examples-v0.1.0...slim-bindings-examples-v0.1.1) (2025-09-18)


### Features

* **python/bindings/examples:** upgrade dep to slim 0.5.0 ([#717](https://github.com/agntcy/slim/issues/717)) ([1fa4ff3](https://github.com/agntcy/slim/commit/1fa4ff31571caff4ccfd7da4c6c68d4c1999da2c))


### Bug Fixes

* **python-bindings:** default crypto provider initialization for Reqwest crate ([#706](https://github.com/agntcy/slim/issues/706)) ([16a71ce](https://github.com/agntcy/slim/commit/16a71ced6164e4b6df7953f897b8f195fd56b097))

## 0.1.0 (2025-08-01)


### ⚠ BREAKING CHANGES

* **data-plane/service:** This change breaks the python binding interface.

### Features

* **data-plane/service:** first draft of session layer ([#106](https://github.com/agntcy/slim/issues/106)) ([6ae63eb](https://github.com/agntcy/slim/commit/6ae63eb76a13be3c231d1c81527bb0b1fd901bac))
* get source and destination name form python ([#485](https://github.com/agntcy/slim/issues/485)) ([fd4ac79](https://github.com/agntcy/slim/commit/fd4ac796f38ee8785a0108b4936028a2068f8b64)), closes [#487](https://github.com/agntcy/slim/issues/487)
* improve configuration handling for tracing ([#186](https://github.com/agntcy/slim/issues/186)) ([ff959ee](https://github.com/agntcy/slim/commit/ff959ee95670ce8bbfc48bc18ccb534270178a2e))
* improve tracing in agp ([#237](https://github.com/agntcy/slim/issues/237)) ([ed1401c](https://github.com/agntcy/slim/commit/ed1401cf91aefa0e3f66c5461e6b331c96f26811))
* notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/slim/issues/72)) ([5fdbaea](https://github.com/agntcy/slim/commit/5fdbaea40d335c29cf48906528d9c26f1994c520))
* propagate context to enable distributed tracing ([#90](https://github.com/agntcy/slim/issues/90)) ([4266d91](https://github.com/agntcy/slim/commit/4266d91854fa235dc6b07b108aa6cfb09a55e433))
* **python-bindings:** add examples ([#153](https://github.com/agntcy/slim/issues/153)) ([a97ac2f](https://github.com/agntcy/slim/commit/a97ac2fc11bfbcd2c38d8f26902b1447a05ad4ac))
* **python-bindings:** improve configuration handling and further refactoring ([#167](https://github.com/agntcy/slim/issues/167)) ([d1a0303](https://github.com/agntcy/slim/commit/d1a030322b3270a0bfe762534c5f326958cd7a8b))
* **python-bindings:** update examples and make them packageable ([#468](https://github.com/agntcy/slim/issues/468)) ([287dcbc](https://github.com/agntcy/slim/commit/287dcbc8932e0978662e2148e08bee95fab1ce3b))
* **session:** add default config for sessions created upon message reception ([#181](https://github.com/agntcy/slim/issues/181)) ([1827936](https://github.com/agntcy/slim/commit/18279363432a8869aabc2895784a6bdae74cf19f))


### Bug Fixes

* **python-bindings:** fix python examples ([#120](https://github.com/agntcy/slim/issues/120)) ([efbe776](https://github.com/agntcy/slim/commit/efbe7768d37b2a8fa86eea8afb8228a5345cbf95))
* **python-byndings:** fix examples and taskfile ([#340](https://github.com/agntcy/slim/issues/340)) ([785f6a9](https://github.com/agntcy/slim/commit/785f6a99f319784000c7c61a0b1dbf6d7fb5d97c)), closes [#339](https://github.com/agntcy/slim/issues/339)
