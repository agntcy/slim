# Changelog

## [0.6.1](https://github.com/agntcy/slim/compare/slim-testutils-v0.6.0...slim-testutils-v0.6.1) (2025-10-17)


### Features

* move session code in a new crate ([#828](https://github.com/agntcy/slim/issues/828)) ([6d0cf90](https://github.com/agntcy/slim/commit/6d0cf90a67cdfd62039fee857cf103a52a0380b1))

## [0.6.0](https://github.com/agntcy/slim/compare/slim-testutils-v0.2.2...slim-testutils-v0.6.0) (2025-10-09)


### ⚠ BREAKING CHANGES

* session layer APIs updated.

### Features

* implement control plane group management ([#554](https://github.com/agntcy/slim/issues/554)) ([d0065a0](https://github.com/agntcy/slim/commit/d0065a0e1955dbc7e7fd2bfabd5fdca210459a0b))
* improve point to point session with sender/receiver buffer ([#735](https://github.com/agntcy/slim/issues/735)) ([e6f65bb](https://github.com/agntcy/slim/commit/e6f65bb9d6584994538027dd4db45429d74821ea))
* **multicast:** remove moderator parameter from configuration ([#739](https://github.com/agntcy/slim/issues/739)) ([464d523](https://github.com/agntcy/slim/commit/464d523205a6f972e633eddd842c007929bb7974))
* **python/bindings:** improve publish function ([#749](https://github.com/agntcy/slim/issues/749)) ([85fd2ca](https://github.com/agntcy/slim/commit/85fd2ca2e24794998203fd25b51964eabc10c04e))
* refactor session receive() API ([#731](https://github.com/agntcy/slim/issues/731)) ([787d111](https://github.com/agntcy/slim/commit/787d111d030de5768385b72ea7a794ced85d6652))
* remove bearer auth in favour of static jwt ([#774](https://github.com/agntcy/slim/issues/774)) ([e08cb51](https://github.com/agntcy/slim/commit/e08cb51ecd20ca6dc1e5ef3f3c0c197e5f926e97))
* **session:** introduce session metadata ([#744](https://github.com/agntcy/slim/issues/744)) ([14528ee](https://github.com/agntcy/slim/commit/14528eec79e31e0729b3f305a8da5bc38ab0ac51))

## [0.2.2](https://github.com/agntcy/slim/compare/slim-testutils-v0.2.1...slim-testutils-v0.2.2) (2025-09-18)


### Features

* add testing for fire and forget session ([#540](https://github.com/agntcy/slim/issues/540)) ([51adc94](https://github.com/agntcy/slim/commit/51adc94b408798aa63f5d2a00621a71d56d71575))
* replace pubsub with dataplane in the node-config ([#591](https://github.com/agntcy/slim/issues/591)) ([88be6ee](https://github.com/agntcy/slim/commit/88be6eebef9e34e26f482812ab91d76c3186ba44))


### Bug Fixes

* fix ff session ([#538](https://github.com/agntcy/slim/issues/538)) ([e0d1f65](https://github.com/agntcy/slim/commit/e0d1f65b23556e1e8f045523daa9b93ff4d8635d))

## [0.2.1](https://github.com/agntcy/slim/compare/slim-testutils-v0.2.0...slim-testutils-v0.2.1) (2025-07-31)


### Bug Fixes

* **testutils:** use common dockerfile to build testutils ([#499](https://github.com/agntcy/slim/issues/499)) ([4f9d127](https://github.com/agntcy/slim/commit/4f9d12780df33fc375673e584a7415a57c2c2b7f))

## [0.2.0](https://github.com/agntcy/slim/compare/slim-testutils-v0.1.0...slim-testutils-v0.2.0) (2025-07-31)


### ⚠ BREAKING CHANGES

* **data-plane/service:** This change breaks the python binding interface.

### Features

* add auth support in sessions ([#382](https://github.com/agntcy/slim/issues/382)) ([242e38a](https://github.com/agntcy/slim/commit/242e38a96c9e8b3d9e4a69de3d35740a53fcf252))
* add flag to disable mls in channel test app ([#408](https://github.com/agntcy/slim/issues/408)) ([d3e5fef](https://github.com/agntcy/slim/commit/d3e5fef5516db7cac3feab40bea2b984c140d7ab))
* add identity and mls options to python bindings ([#436](https://github.com/agntcy/slim/issues/436)) ([8c9efbe](https://github.com/agntcy/slim/commit/8c9efbefea0dd09c93e320770d96adb399c8da28))
* add intial testutil image build ([#289](https://github.com/agntcy/slim/issues/289)) ([b0c9d1b](https://github.com/agntcy/slim/commit/b0c9d1b07940b87d20834ff84853f56c713863b3))
* add mls message types in slim messages ([#386](https://github.com/agntcy/slim/issues/386)) ([1623d0d](https://github.com/agntcy/slim/commit/1623d0d5c8088d236215f25552bf554319b3157a))
* add optional acks for FNF messages ([#264](https://github.com/agntcy/slim/issues/264)) ([508fdf3](https://github.com/agntcy/slim/commit/508fdf3ce00650a8a8d237db7223e7928c6bf395))
* add pub/sub session layer ([#146](https://github.com/agntcy/slim/issues/146)) ([d8a4c80](https://github.com/agntcy/slim/commit/d8a4c80bc8e8168b6220c7fdb481e0944dd3cde5))
* channel creation in session layer ([#374](https://github.com/agntcy/slim/issues/374)) ([88d1610](https://github.com/agntcy/slim/commit/88d16107e655a731176cbe7a29bb544c9d301b7c)), closes [#362](https://github.com/agntcy/slim/issues/362)
* **data-plane/service:** first draft of session layer ([#106](https://github.com/agntcy/slim/issues/106)) ([6ae63eb](https://github.com/agntcy/slim/commit/6ae63eb76a13be3c231d1c81527bb0b1fd901bac))
* **data-plane:** support for multiple servers ([#173](https://github.com/agntcy/slim/issues/173)) ([1347d49](https://github.com/agntcy/slim/commit/1347d49c51b2705e55eea8792d9097be419e5b01))
* do no create session on discovery request ([#402](https://github.com/agntcy/slim/issues/402)) ([35e05ef](https://github.com/agntcy/slim/commit/35e05ef29607195a6089e1bb006a202c737d67a1)), closes [#396](https://github.com/agntcy/slim/issues/396)
* handle disconnection events ([#67](https://github.com/agntcy/slim/issues/67)) ([33801aa](https://github.com/agntcy/slim/commit/33801aa2934b81b5a682973e8a9a38cddc3fa54c))
* improve channel testing app ([#389](https://github.com/agntcy/slim/issues/389)) ([d604e47](https://github.com/agntcy/slim/commit/d604e4723812c8e639a08f33a412088d29aebd5a))
* improve message processing file ([#101](https://github.com/agntcy/slim/issues/101)) ([6a0591c](https://github.com/agntcy/slim/commit/6a0591ce92411c76a6514e51322f8bee3294d768))
* integrate MLS with auth ([#385](https://github.com/agntcy/slim/issues/385)) ([681372a](https://github.com/agntcy/slim/commit/681372a90fc2c079715fdfc72b0997219045ea1d))
* new message format ([#88](https://github.com/agntcy/slim/issues/88)) ([aefaaa0](https://github.com/agntcy/slim/commit/aefaaa09e89c0a2e36f4e3f67cdafc1bfaa169d6))
* notify local app if a message is not processed correctly ([#72](https://github.com/agntcy/slim/issues/72)) ([5fdbaea](https://github.com/agntcy/slim/commit/5fdbaea40d335c29cf48906528d9c26f1994c520))
* process concurrent modification to the group ([#451](https://github.com/agntcy/slim/issues/451)) ([0ed1587](https://github.com/agntcy/slim/commit/0ed1587e40872e05fb2a6296a7eddd3a850c56f9)), closes [#438](https://github.com/agntcy/slim/issues/438)
* request/reply session type ([#124](https://github.com/agntcy/slim/issues/124)) ([0b4c4a5](https://github.com/agntcy/slim/commit/0b4c4a5239f79efc85b86d47cd3c752bd380391f))
* **session layer:** send rtx error if the packet is not in the producer buffer ([#166](https://github.com/agntcy/slim/issues/166)) ([2cadb50](https://github.com/agntcy/slim/commit/2cadb501458c1a729ca8e2329da642f7a96575c0))
* streaming test app ([#144](https://github.com/agntcy/slim/issues/144)) ([59b6dea](https://github.com/agntcy/slim/commit/59b6dea8a634af41bb3c2246baa9d5fab5171e16))
* **testing:** add workload generator and testing apps ([#62](https://github.com/agntcy/slim/issues/62)) ([bef4964](https://github.com/agntcy/slim/commit/bef4964a077a2620da0d9cf91770a038c9be57bc))


### Bug Fixes

* add missing channel binary to testing Cargo.toml ([#423](https://github.com/agntcy/slim/issues/423)) ([8544272](https://github.com/agntcy/slim/commit/85442720b9baef499b1986b8a5759554bbdfa1ad))
* **auth:** make simple identity usable for groups ([#387](https://github.com/agntcy/slim/issues/387)) ([ba2001f](https://github.com/agntcy/slim/commit/ba2001fc3dccb7e977e6627aa4289124717436f5))
* **channel_endpoint:** extend mls for all sessions ([#411](https://github.com/agntcy/slim/issues/411)) ([7687930](https://github.com/agntcy/slim/commit/76879306d9919a796d37f4c58f83d0859028ca3d))
* keep only the from_strings method to create an AgentType ([#288](https://github.com/agntcy/slim/issues/288)) ([2d6bbd9](https://github.com/agntcy/slim/commit/2d6bbd9b044ea112262847006e186f2a7c71adc0))
* remove all state on session close ([#449](https://github.com/agntcy/slim/issues/449)) ([31eea80](https://github.com/agntcy/slim/commit/31eea80e71a981901b22850f721faa82faf7b9b4)), closes [#493](https://github.com/agntcy/slim/issues/493)
* **testing-apps:** build using common rust taskfiles ([#292](https://github.com/agntcy/slim/issues/292)) ([ffa5eed](https://github.com/agntcy/slim/commit/ffa5eede56b49054412459e1fa2689f66627fdd1))
