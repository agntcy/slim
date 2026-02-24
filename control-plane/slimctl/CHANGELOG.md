# Changelog

## [1.0.1](https://github.com/agntcy/slim/compare/slimctl-v1.0.0...slimctl-v1.0.1) (2026-02-24)


### Bug Fixes

* close grpc connections ([#1159](https://github.com/agntcy/slim/issues/1159)) ([81f7e29](https://github.com/agntcy/slim/commit/81f7e2983f4c20729366c2d7c0893a740bd46d43))
* **slimctl:** improve startup error reporting ([#1248](https://github.com/agntcy/slim/issues/1248)) ([0174a41](https://github.com/agntcy/slim/commit/0174a419fc4646aee7c11fe84430fb61deaffa06))
* **slimctl:** remove misleading prints ([#1170](https://github.com/agntcy/slim/issues/1170)) ([8ad3e96](https://github.com/agntcy/slim/commit/8ad3e96ef4f3058eb72f3fe02d1fc7e59cb7522f))
* vulnerability GO-2026-4337 ([#1213](https://github.com/agntcy/slim/issues/1213)) ([be83dbe](https://github.com/agntcy/slim/commit/be83dbebced5486b4b524eea7c5cb684ee246caa))

## [1.0.0](https://github.com/agntcy/slim/compare/slimctl-v0.7.0...slimctl-v1.0.0) (2026-01-30)


### Features

* slimctl: show all properties of outlined routes ([#1002](https://github.com/agntcy/slim/issues/1002)) ([33fd62f](https://github.com/agntcy/slim/commit/33fd62f94a4d5a30a7a886fc44ed60ddd054b18c))
* Support different trust domains in auto route setup ([#1001](https://github.com/agntcy/slim/issues/1001)) ([9a7e930](https://github.com/agntcy/slim/commit/9a7e930f167751f185592c33d11b14f942ee9bb1))
* support for configuring the locally started slim instance ([#1139](https://github.com/agntcy/slim/issues/1139)) ([34af1c7](https://github.com/agntcy/slim/commit/34af1c74f469ae92498205390b1dabe6a17314f6))
* support for managing a slim node instance local to the tool ([#1015](https://github.com/agntcy/slim/issues/1015)) ([c7bbd29](https://github.com/agntcy/slim/commit/c7bbd2922d79774c2be3a702910b55d9e49214bc))


### Bug Fixes

* return ack from control plane ([#1098](https://github.com/agntcy/slim/issues/1098)) ([7f873b5](https://github.com/agntcy/slim/commit/7f873b578b4cf4146b4b6aa6c494a21b5c0b0005))

## [0.7.0](https://github.com/agntcy/slim/compare/slimctl-v0.6.0...slimctl-v0.7.0) (2025-11-18)


### Features

* add config subcommand to slimctl ([#818](https://github.com/agntcy/slim/issues/818)) ([562b550](https://github.com/agntcy/slim/commit/562b5504608cb37f6d6f6e9dede155cf92a83426))
* outline routes (list routes from the controller) ([#871](https://github.com/agntcy/slim/issues/871)) ([278ec7d](https://github.com/agntcy/slim/commit/278ec7da6f85140da3b43a723773423386ec07a5))
* support for homebrew  - slimctl releases ([#921](https://github.com/agntcy/slim/issues/921)) ([e6688b8](https://github.com/agntcy/slim/commit/e6688b825a8c1ad3d1fe5d13073d20efe931707a))


### Bug Fixes

* Handle route connection faliures and node connection detail changes ([#833](https://github.com/agntcy/slim/issues/833)) ([8027c3b](https://github.com/agntcy/slim/commit/8027c3b0a11a7d3c2b57184d2313e18d5de6ba3b))
* upgraded to golang version 1.25.4 ([#923](https://github.com/agntcy/slim/issues/923)) ([82cabef](https://github.com/agntcy/slim/commit/82cabef4e744fa7954559d06aa97e81d3e4eef3a))

## [0.6.0](https://github.com/agntcy/slim/compare/slimctl-v0.2.2...slimctl-v0.6.0) (2025-10-09)


### Features

* add basic auth to slimctl ([#763](https://github.com/agntcy/slim/issues/763)) ([b064d0d](https://github.com/agntcy/slim/commit/b064d0d1fce57c219f18e210770765f348c45fdd))
* handle updates from SLIM nodes ([#708](https://github.com/agntcy/slim/issues/708)) ([ccc5183](https://github.com/agntcy/slim/commit/ccc518386d0ece16237647511118e7d032e033c6))
* implement control plane group management ([#554](https://github.com/agntcy/slim/issues/554)) ([d0065a0](https://github.com/agntcy/slim/commit/d0065a0e1955dbc7e7fd2bfabd5fdca210459a0b))

## [0.2.2](https://github.com/agntcy/slim/compare/slimctl-v0.2.1...slimctl-v0.2.2) (2025-09-18)

### Fixes

* fixed the slimctl version crash ([#585](https://github.com/agntcy/slim/issues/585)) ([e4ba8e6](https://github.com/agntcy/slim/commit/e4ba8e6c44265bf8705d847069ed75fa87329564))

### Features

* add channel and participant commands ([#534](https://github.com/agntcy/slim/issues/534)) ([ecbca90](https://github.com/agntcy/slim/commit/ecbca9074548a51f26e889a2e1adecb1a67a2029))

## [0.2.1](https://github.com/agntcy/slim/compare/slimctl-v0.2.0...slimctl-v0.2.1) (2025-07-31)


### Features

* add auth support in sessions ([#382](https://github.com/agntcy/slim/issues/382)) ([242e38a](https://github.com/agntcy/slim/commit/242e38a96c9e8b3d9e4a69de3d35740a53fcf252))
* add client connections to control plane ([#429](https://github.com/agntcy/slim/issues/429)) ([26cfee6](https://github.com/agntcy/slim/commit/26cfee6565c7be933afd7edab36dca032753e132))
* add node register call to proto ([#406](https://github.com/agntcy/slim/issues/406)) ([bdce118](https://github.com/agntcy/slim/commit/bdce1181dd0d05d78eb4f577aa012c4033cad3b2))
* control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/issues/388)) ([8a17dba](https://github.com/agntcy/slim/commit/8a17dbad99fa679e07585ca4fbcefe9cb3fa8a29))
* **control-plane:** handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/issues/360)) ([9fe6230](https://github.com/agntcy/slim/commit/9fe623093614cf075d36e938734625003087e465))


### Bug Fixes

* slimctl: add scheme to endpoint param ([#459](https://github.com/agntcy/slim/issues/459)) ([6716ff0](https://github.com/agntcy/slim/commit/6716ff0c53f6b090170ff6cd64bd44ec9e4d387f))

## [0.2.0](https://github.com/agntcy/slim/compare/slimctl-v0.1.4...slimctl-v0.2.0) (2025-07-31)


### Features

* add auth support in sessions ([#382](https://github.com/agntcy/slim/issues/382)) ([242e38a](https://github.com/agntcy/slim/commit/242e38a96c9e8b3d9e4a69de3d35740a53fcf252))
* add client connections to control plane ([#429](https://github.com/agntcy/slim/issues/429)) ([26cfee6](https://github.com/agntcy/slim/commit/26cfee6565c7be933afd7edab36dca032753e132))
* add node register call to proto ([#406](https://github.com/agntcy/slim/issues/406)) ([bdce118](https://github.com/agntcy/slim/commit/bdce1181dd0d05d78eb4f577aa012c4033cad3b2))
* control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/issues/388)) ([8a17dba](https://github.com/agntcy/slim/commit/8a17dbad99fa679e07585ca4fbcefe9cb3fa8a29))
* **control-plane:** handle all configuration parameters when creating a new connection ([#360](https://github.com/agntcy/slim/issues/360)) ([9fe6230](https://github.com/agntcy/slim/commit/9fe623093614cf075d36e938734625003087e465))


### Bug Fixes

* slimctl: add scheme to endpoint param ([#459](https://github.com/agntcy/slim/issues/459)) ([6716ff0](https://github.com/agntcy/slim/commit/6716ff0c53f6b090170ff6cd64bd44ec9e4d387f))

## [0.1.4](https://github.com/agntcy/slim/compare/slimctl-v0.1.3...slimctl-v0.1.4) (2025-05-07)


### Bug Fixes

* fix description ([#223](https://github.com/agntcy/slim/issues/223)) ([ebaa732](https://github.com/agntcy/slim/commit/ebaa73294a12b3780fffe8168d81b4a0b17c627e))

## [0.1.3](https://github.com/agntcy/slim/compare/slimctl-v0.1.2...slimctl-v0.1.3) (2025-05-07)


### Bug Fixes

* fix description ([#220](https://github.com/agntcy/slim/issues/220)) ([b316693](https://github.com/agntcy/slim/commit/b316693fc4b71a976b831b98d99bf629a60fa21b))

## [0.1.2](https://github.com/agntcy/slim/compare/slimctl-v0.1.1...slimctl-v0.1.2) (2025-05-07)


### Bug Fixes

* fix description ([#217](https://github.com/agntcy/slim/issues/217)) ([f46e7b3](https://github.com/agntcy/slim/commit/f46e7b39964b6c014f8177e98f3ceb3e9d50dcd0))

## [0.1.1](https://github.com/agntcy/slim/compare/slimctl-v0.1.0...slimctl-v0.1.1) (2025-05-07)


### Bug Fixes

* fix description ([#214](https://github.com/agntcy/slim/issues/214)) ([3f0854d](https://github.com/agntcy/slim/commit/3f0854de5b8adc9404bb75b7a6213a1d8d577ab2))
