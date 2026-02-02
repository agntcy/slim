# Changelog

## [0.7.1](https://github.com/agntcy/slim/compare/helm-slim-v0.7.0...helm-slim-v0.7.1) (2026-02-02)


### Features

* Migrate Rust MCP proxy to its own repo ([#1100](https://github.com/agntcy/slim/issues/1100)) ([c016c7d](https://github.com/agntcy/slim/commit/c016c7d956843446fb72b027828708c594fda2ae))
* **slim-helm:** upgrade SLIM to 1.0.0 ([#1166](https://github.com/agntcy/slim/issues/1166)) ([be2d8ab](https://github.com/agntcy/slim/commit/be2d8abc09d4ad3bef462febcade01020e1ac219))
* Support different trust domains in auto route setup ([#1001](https://github.com/agntcy/slim/issues/1001)) ([9a7e930](https://github.com/agntcy/slim/commit/9a7e930f167751f185592c33d11b14f942ee9bb1))

## [0.7.0](https://github.com/agntcy/slim/compare/helm-slim-v0.6.0...helm-slim-v0.7.0) (2025-11-18)


### Features

* **slim-helm:** upgrade to latest image ([#966](https://github.com/agntcy/slim/issues/966)) ([7fd71c8](https://github.com/agntcy/slim/commit/7fd71c892cc265f01f38310a196052556f70a035))
* support data-plane spire configs ([#940](https://github.com/agntcy/slim/issues/940)) ([ab80876](https://github.com/agntcy/slim/commit/ab8087635fcb677bdcc7a698f9c947ea8913418c))

## [0.6.0](https://github.com/agntcy/slim/compare/helm-slim-v0.2.0...helm-slim-v0.6.0) (2025-10-10)


### Features

* upgrade helm chart to latest SLIM image ([#823](https://github.com/agntcy/slim/issues/823)) ([4e1dd41](https://github.com/agntcy/slim/commit/4e1dd41fab4f991d2ad906ef6ca178895d1d9bb7))


### Bug Fixes

* add group id to node id ([#746](https://github.com/agntcy/slim/issues/746)) ([06c42b3](https://github.com/agntcy/slim/commit/06c42b3f3846da331554ac72ec6d77e61876d78d))

## [0.2.0](https://github.com/agntcy/slim/compare/helm-slim-v0.1.9...helm-slim-v0.2.0) (2025-09-18)


### ⚠ BREAKING CHANGES

* feat: SLIM node ID should be unique in a deployment ([#630](https://github.com/agntcy/slim/pull/630)) ([b63ffc1](https://github.com/agntcy/slim/commit/b63ffc16b97ddb7115f6639c9d0301df8d65c1b9))

### Features

* **charts/slim:** upgrade image to SLIM 0.5.0 ([#714](https://github.com/agntcy/slim/issues/714)) ([35529c4](https://github.com/agntcy/slim/commit/35529c43ec12915d9e98a0f8770d14c7d19cebd6))


### Bug Fixes

* **helm:** incorrect port reference in ingress ([#597](https://github.com/agntcy/slim/issues/597)) ([245aeea](https://github.com/agntcy/slim/commit/245aeea2843a71e20ad550ca2900fcbaed986b0a))
* **helm:** use template comments for copyright headers ([#595](https://github.com/agntcy/slim/issues/595)) ([c8b39de](https://github.com/agntcy/slim/commit/c8b39de1ec41261c57d54691a4be3b72556d659c))

## [0.1.9](https://github.com/agntcy/slim/compare/helm-slim-v0.1.8...helm-slim-v0.1.9) (2025-07-31)


### Features

* **chart:** add extra Volumes and Volumemounts to the slim deployment ([#320](https://github.com/agntcy/slim/issues/320)) ([88a57c7](https://github.com/agntcy/slim/commit/88a57c748b62e1e454eeb57c256a8e5b669317d4))
* **helm:** upgrade to SLIM 0.3.15 ([#299](https://github.com/agntcy/slim/issues/299)) ([381e2f9](https://github.com/agntcy/slim/commit/381e2f9376c86c06233112982c9a9867972c19f3))
* **slim-helm:** upgrade helm to slim image 0.4.0 ([#495](https://github.com/agntcy/slim/issues/495)) ([3c20e75](https://github.com/agntcy/slim/commit/3c20e75294dfff1774152ffa64ec243adf086907))


### Bug Fixes

* add slim.overrideConfig to helm values ([#490](https://github.com/agntcy/slim/issues/490)) ([4121aa6](https://github.com/agntcy/slim/commit/4121aa6eef54915c9af169c638d8c13b5a20202a))
* **chart:** extraVolumes and mount intentation ([#323](https://github.com/agntcy/slim/issues/323)) ([0eea40d](https://github.com/agntcy/slim/commit/0eea40dac4ffe165908de55b69f6d38b1bca75d3))
* **helm-chart:** use release-please to release new chart version ([#375](https://github.com/agntcy/slim/issues/375)) ([6603c71](https://github.com/agntcy/slim/commit/6603c7185849ef256ca9adf14bac0dcc444411f7))

## [0.1.8](https://github.com/agntcy/slim/compare/slim-helm-v0.1.7...slim-helm-v0.1.8) (2025-06-30)

### Features

* **helm-chart:** Add spire support to SLIM chart ([#372](https://github.com/agntcy/slim/pull/372)) ([6603c7](https://github.com/agntcy/slim/commit/6603c7185849ef256ca9adf14bac0dcc444411f7))

## [0.1.5](https://github.com/agntcy/slim/compare/slim-helm-v0.1.4...slim-helm-v0.1.5) (2025-06-03)

### Bug Fixes

* **helm-chart:** use release-please to release new chart version ([#375](https://github.com/agntcy/slim/issues/375)) ([6603c71](https://github.com/agntcy/slim/commit/6603c7185849ef256ca9adf14bac0dcc444411f7))

## [0.1.7](https://github.com/agntcy/slim/compare/slim-helm-v0.1.6...slim-helm-v0.1.7) (2025-06-06)


### Bug Fixes

* **chart:** extraVolumes and mount intentation ([#323](https://github.com/agntcy/slim/issues/323)) ([0eea40d](https://github.com/agntcy/slim/commit/0eea40dac4ffe165908de55b69f6d38b1bca75d3))

## [0.1.6](https://github.com/agntcy/slim/compare/slim-helm-v0.1.5...slim-helm-v0.1.6) (2025-06-06)


### Features

* **chart:** add extra Volumes and Volumemounts to the slim deployment ([#320](https://github.com/agntcy/slim/issues/320)) ([88a57c7](https://github.com/agntcy/slim/commit/88a57c748b62e1e454eeb57c256a8e5b669317d4))

## [0.1.5](https://github.com/agntcy/slim/compare/slim-helm-v0.1.4...slim-helm-v0.1.5) (2025-06-03)


### Features

* **helm:** upgrade to SLIM 0.3.15 ([#299](https://github.com/agntcy/slim/issues/299)) ([381e2f9](https://github.com/agntcy/slim/commit/381e2f9376c86c06233112982c9a9867972c19f3))

## [0.1.4](https://github.com/agntcy/slim/compare/slim-helm-v0.1.3...slim-helm-v0.1.4) (2025-05-14)


### Features

* **chart:** add MCP proxy support ([#253](https://github.com/agntcy/slim/issues/253)) ([b0f743d](https://github.com/agntcy/slim/commit/b0f743d9da02d6ef711d7ec4d8752ab47258ad44))

## [0.1.3](https://github.com/agntcy/slim/compare/slim-helm-v0.1.2...slim-helm-v0.1.3) (2025-04-10)


### Features

* **charts/slim:** update slim image version ([#162](https://github.com/agntcy/slim/issues/162)) ([e545a49](https://github.com/agntcy/slim/commit/e545a49e26fa0b860a658fa0e9fa4640c5b64694))

## [0.1.2](https://github.com/agntcy/slim/compare/slim-helm-v0.1.1...slim-helm-v0.1.2) (2025-03-24)


### Features

* **helm:** release new helm version with slim 0.3.10 ([#113](https://github.com/agntcy/slim/issues/113)) ([b0d0ad6](https://github.com/agntcy/slim/commit/b0d0ad6e6cb75be999b79e1d93d491c4ae59668b))

## [0.1.1](https://github.com/agntcy/slim/compare/slim-helm-v0.1.0...slim-helm-v0.1.1) (2025-02-14)


### Features

* add checks for helm charts ([#49](https://github.com/agntcy/slim/issues/49)) ([accb7db](https://github.com/agntcy/slim/commit/accb7db30da76dfa7c7eab8688edda6cfad9c768))
