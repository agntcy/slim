# Changelog

## [1.0.0](https://github.com/agntcy/agp/compare/agp-testutils-v0.1.0...agp-testutils-v1.0.0) (2025-06-02)


### ⚠ BREAKING CHANGES

* **data-plane/service:** This change breaks the python binding interface.

### Features

* add intial testutil image build ([#289](https://github.com/agntcy/agp/issues/289)) ([b0c9d1b](https://github.com/agntcy/agp/commit/b0c9d1b07940b87d20834ff84853f56c713863b3))
* add optional acks for FNF messages ([#264](https://github.com/agntcy/agp/issues/264)) ([508fdf3](https://github.com/agntcy/agp/commit/508fdf3ce00650a8a8d237db7223e7928c6bf395))
* add pub/sub session layer ([#146](https://github.com/agntcy/agp/issues/146)) ([d8a4c80](https://github.com/agntcy/agp/commit/d8a4c80bc8e8168b6220c7fdb481e0944dd3cde5))
* **data-plane/service:** first draft of session layer ([#106](https://github.com/agntcy/agp/issues/106)) ([6ae63eb](https://github.com/agntcy/agp/commit/6ae63eb76a13be3c231d1c81527bb0b1fd901bac))
* **data-plane:** support for multiple servers ([#173](https://github.com/agntcy/agp/issues/173)) ([1347d49](https://github.com/agntcy/agp/commit/1347d49c51b2705e55eea8792d9097be419e5b01))
* handle disconnection events ([#67](https://github.com/agntcy/agp/issues/67)) ([33801aa](https://github.com/agntcy/agp/commit/33801aa2934b81b5a682973e8a9a38cddc3fa54c))
* improve message processing file ([#101](https://github.com/agntcy/agp/issues/101)) ([6a0591c](https://github.com/agntcy/agp/commit/6a0591ce92411c76a6514e51322f8bee3294d768))
* new message format ([#88](https://github.com/agntcy/agp/issues/88)) ([aefaaa0](https://github.com/agntcy/agp/commit/aefaaa09e89c0a2e36f4e3f67cdafc1bfaa169d6))
* notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/agp/issues/72)) ([5fdbaea](https://github.com/agntcy/agp/commit/5fdbaea40d335c29cf48906528d9c26f1994c520))
* request/reply session type ([#124](https://github.com/agntcy/agp/issues/124)) ([0b4c4a5](https://github.com/agntcy/agp/commit/0b4c4a5239f79efc85b86d47cd3c752bd380391f))
* **session layer:** send rtx error if the packet is not in the producer buffer ([#166](https://github.com/agntcy/agp/issues/166)) ([2cadb50](https://github.com/agntcy/agp/commit/2cadb501458c1a729ca8e2329da642f7a96575c0))
* streaming test app ([#144](https://github.com/agntcy/agp/issues/144)) ([59b6dea](https://github.com/agntcy/agp/commit/59b6dea8a634af41bb3c2246baa9d5fab5171e16))
* **testing:** add workload generator and testing apps ([#62](https://github.com/agntcy/agp/issues/62)) ([bef4964](https://github.com/agntcy/agp/commit/bef4964a077a2620da0d9cf91770a038c9be57bc))


### Bug Fixes

* keep only the from_strings method to create an AgentType ([#288](https://github.com/agntcy/agp/issues/288)) ([2d6bbd9](https://github.com/agntcy/agp/commit/2d6bbd9b044ea112262847006e186f2a7c71adc0))
* **testing-apps:** build using common rust taskfiles ([#292](https://github.com/agntcy/agp/issues/292)) ([ffa5eed](https://github.com/agntcy/agp/commit/ffa5eede56b49054412459e1fa2689f66627fdd1))
