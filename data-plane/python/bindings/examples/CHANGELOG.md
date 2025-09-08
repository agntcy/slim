# Changelog

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
