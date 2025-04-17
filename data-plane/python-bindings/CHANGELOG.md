# Changelog

## [0.3.0](https://github.com/agntcy/agp/compare/agp-bindings-v0.2.4...agp-bindings-v0.3.0) (2025-04-17)


### Features

* **data-plane:** support for multiple servers ([#173](https://github.com/agntcy/agp/issues/173)) ([1347d49](https://github.com/agntcy/agp/commit/1347d49c51b2705e55eea8792d9097be419e5b01))
* **python-bindings:** add session deletion API ([#176](https://github.com/agntcy/agp/issues/176)) ([ce28084](https://github.com/agntcy/agp/commit/ce28084f150a89294f703c70a0cd3f4e6b3ab351))
* **python-bindings:** improve configuration handling and further refactoring ([#167](https://github.com/agntcy/agp/issues/167)) ([d1a0303](https://github.com/agntcy/agp/commit/d1a030322b3270a0bfe762534c5f326958cd7a8b))
* **session:** add default config for sessions created upon message reception ([#181](https://github.com/agntcy/agp/issues/181)) ([1827936](https://github.com/agntcy/agp/commit/18279363432a8869aabc2895784a6bdae74cf19f))


### Bug Fixes

* **agp-bindings:** bug fixes ([#174](https://github.com/agntcy/agp/issues/174)) ([7e8bad3](https://github.com/agntcy/agp/commit/7e8bad3a71d11a3bb194fd97f6e6057d9ee79f12))
* **python-bindings:** rename and improve TimeoutError and improve docstring ([#180](https://github.com/agntcy/agp/issues/180)) ([df71d2e](https://github.com/agntcy/agp/commit/df71d2eb53798041cb42c277af41d36eff7a838b))


### Dependencies

* **data-plane:** tonic 0.12.3 -&gt; 0.13 ([#170](https://github.com/agntcy/agp/issues/170)) ([95f28cc](https://github.com/agntcy/agp/commit/95f28ccc4ff8d7cef81cedfca59a1fc4d04f79d5))

## [0.2.4](https://github.com/agntcy/agp/compare/agp-bindings-v0.2.3...agp-bindings-v0.2.4) (2025-04-11)


### Features

* **session layer:** send rtx error if the packet is not in the producer buffer ([#166](https://github.com/agntcy/agp/issues/166)) ([2cadb50](https://github.com/agntcy/agp/commit/2cadb501458c1a729ca8e2329da642f7a96575c0))

## [0.2.3](https://github.com/agntcy/agp/compare/agp-bindings-v0.2.2...agp-bindings-v0.2.3) (2025-04-09)


### Bug Fixes

* **agp-bindings:** build pypi package on ubuntu 22.04 ([#160](https://github.com/agntcy/agp/issues/160)) ([a9768c1](https://github.com/agntcy/agp/commit/a9768c189d0afd5cf24efd5f2b3f610d780cf762))

## [0.2.2](https://github.com/agntcy/agp/compare/agp-bindings-v0.2.1...agp-bindings-v0.2.2) (2025-04-09)


### Bug Fixes

* **python-bindings:** update example name in readme ([#158](https://github.com/agntcy/agp/issues/158)) ([8ecad2b](https://github.com/agntcy/agp/commit/8ecad2b69f0ed8caa0103b74b3ce3523d6695576))

## [0.2.1](https://github.com/agntcy/agp/compare/agp-bindings-v0.2.0...agp-bindings-v0.2.1) (2025-04-09)


### Bug Fixes

* service name in python bindings ([#155](https://github.com/agntcy/agp/issues/155)) ([66a5247](https://github.com/agntcy/agp/commit/66a524757bae335a5cb2b888ba77af95e94dc132))

## [0.2.0](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.14...agp-bindings-v0.2.0) (2025-04-08)


### ⚠ BREAKING CHANGES

* **data-plane/service:** This change breaks the python binding interface.

### Features

* **data-plane/service:** first draft of session layer ([#106](https://github.com/agntcy/agp/issues/106)) ([6ae63eb](https://github.com/agntcy/agp/commit/6ae63eb76a13be3c231d1c81527bb0b1fd901bac))
* **python-bindings:** add examples ([#153](https://github.com/agntcy/agp/issues/153)) ([a97ac2f](https://github.com/agntcy/agp/commit/a97ac2fc11bfbcd2c38d8f26902b1447a05ad4ac))
* request/reply session type ([#124](https://github.com/agntcy/agp/issues/124)) ([0b4c4a5](https://github.com/agntcy/agp/commit/0b4c4a5239f79efc85b86d47cd3c752bd380391f))


### Bug Fixes

* **python-bindings:** fix python examples ([#120](https://github.com/agntcy/agp/issues/120)) ([efbe776](https://github.com/agntcy/agp/commit/efbe7768d37b2a8fa86eea8afb8228a5345cbf95))

## [0.1.14](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.13...agp-bindings-v0.1.14) (2025-03-19)


### Features

* improve message processing file ([#101](https://github.com/agntcy/agp/issues/101)) ([6a0591c](https://github.com/agntcy/agp/commit/6a0591ce92411c76a6514e51322f8bee3294d768))

## [0.1.13](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.12...agp-bindings-v0.1.13) (2025-03-19)


### Features

* **tables:** do not require Default/Clone traits for elements stored in pool ([#97](https://github.com/agntcy/agp/issues/97)) ([afd6765](https://github.com/agntcy/agp/commit/afd6765fc6d05bc0b8692db33356469bfe749426))


### Bug Fixes

* **python-bindings:** move windows build instructions in dedicated file ([#100](https://github.com/agntcy/agp/issues/100)) ([2fcc546](https://github.com/agntcy/agp/commit/2fcc546ac4e175ea6052a30758be7fc618e38114))

## [0.1.12](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.11...agp-bindings-v0.1.12) (2025-03-18)


### Features

* new message format ([#88](https://github.com/agntcy/agp/issues/88)) ([aefaaa0](https://github.com/agntcy/agp/commit/aefaaa09e89c0a2e36f4e3f67cdafc1bfaa169d6))

## [0.1.11](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.10...agp-bindings-v0.1.11) (2025-03-18)


### Features

* propagate context to enable distributed tracing ([#90](https://github.com/agntcy/agp/issues/90)) ([4266d91](https://github.com/agntcy/agp/commit/4266d91854fa235dc6b07b108aa6cfb09a55e433))

## [0.1.10](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.9...agp-bindings-v0.1.10) (2025-03-14)


### Bug Fixes

* **python-bindings:** wheels for python 3.13 in windows ([#84](https://github.com/agntcy/agp/issues/84)) ([4418866](https://github.com/agntcy/agp/commit/4418866f354397a1f7ee8fcbdbdb6ca4eb725e96))

## [0.1.9](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.8...agp-bindings-v0.1.9) (2025-03-12)


### Features

* notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/agp/issues/72)) ([5fdbaea](https://github.com/agntcy/agp/commit/5fdbaea40d335c29cf48906528d9c26f1994c520))

## [0.1.8](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.7...agp-bindings-v0.1.8) (2025-03-06)


### Features

* handle disconnection events ([#67](https://github.com/agntcy/agp/issues/67)) ([33801aa](https://github.com/agntcy/agp/commit/33801aa2934b81b5a682973e8a9a38cddc3fa54c))

## [0.1.7](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.6...agp-bindings-v0.1.7) (2025-02-20)


### Features

* **tables:** distinguish local and remote connections in the subscription table ([#55](https://github.com/agntcy/agp/issues/55)) ([143520f](https://github.com/agntcy/agp/commit/143520f89cee8b29eb8e575b04d887458099ac2e))

## [0.1.6](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.5...agp-bindings-v0.1.6) (2025-02-19)


### Bug Fixes

* **examples:** fix example tests ([#52](https://github.com/agntcy/agp/issues/52)) ([411a617](https://github.com/agntcy/agp/commit/411a61714fa6c015b5f29f671e027340a5624c11))

## [0.1.5](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.4...agp-bindings-v0.1.5) (2025-02-14)


### Features

* add metadata for pypi ([#48](https://github.com/agntcy/agp/issues/48)) ([26d0e60](https://github.com/agntcy/agp/commit/26d0e6055f4d2a81f5dc20f71668f004502ed6a1))


### Bug Fixes

* **python-bindings:** propagate build PROFILE up to  task target ([#45](https://github.com/agntcy/agp/issues/45)) ([ac4e3a0](https://github.com/agntcy/agp/commit/ac4e3a00ee9ac0c8e738b97657be9a7fc25b7b56))

## [0.1.4](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.3...agp-bindings-v0.1.4) (2025-02-14)


### Features

* implement opentelemetry tracing subscriber ([5a0ec9e](https://github.com/agntcy/agp/commit/5a0ec9e876a73d90724f0a83cb0925de1c8d0af4))

## [0.1.3](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.2...agp-bindings-v0.1.3) (2025-02-13)


### Features

* **python-wheels:** add aarch64 build ([#37](https://github.com/agntcy/agp/issues/37)) ([7631f4e](https://github.com/agntcy/agp/commit/7631f4ea1425b40fd8139270ea51785463fad22e))

## [0.1.2](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.1...agp-bindings-v0.1.2) (2025-02-12)


### Bug Fixes

* python bindings import name ([#24](https://github.com/agntcy/agp/issues/24)) ([5f5e7c6](https://github.com/agntcy/agp/commit/5f5e7c6a823a3e842d13d326436cbdc73c64bacf))

## [0.1.1](https://github.com/agntcy/agp/compare/agp-bindings-v0.1.0...agp-bindings-v0.1.1) (2025-02-11)


### Features

* improve readme for pypi release ([#19](https://github.com/agntcy/agp/issues/19)) ([23dfa5c](https://github.com/agntcy/agp/commit/23dfa5cbd20c96a35e62d40a0808c3268b177f8b))
* include readme in published pypi package ([#18](https://github.com/agntcy/agp/issues/18)) ([5a26dea](https://github.com/agntcy/agp/commit/5a26dea6ece36124ed88861bc32fe7eea4aea184))

## [0.1.0](https://github.com/agntcy/agp/compare/agp-bindings-v0.0.0...agp-bindings-v0.1.0) (2025-02-11)


### Features

* automate python packages release ([#16](https://github.com/agntcy/agp/issues/16)) ([f806256](https://github.com/agntcy/agp/commit/f8062564c8451767c5b38fedce38c520c8c216ac))
