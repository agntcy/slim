# Changelog

## [1.1.0](https://github.com/agntcy/slim/compare/control-plane-v1.0.0...control-plane-v1.1.0) (2026-02-27)


### Features

* slimctl: show all properties of outlined routes ([#1002](https://github.com/agntcy/slim/issues/1002)) ([33fd62f](https://github.com/agntcy/slim/commit/33fd62f94a4d5a30a7a886fc44ed60ddd054b18c))
* Support different trust domains in auto route setup ([#1001](https://github.com/agntcy/slim/issues/1001)) ([9a7e930](https://github.com/agntcy/slim/commit/9a7e930f167751f185592c33d11b14f942ee9bb1))
* support for configuring the locally started slim instance ([#1139](https://github.com/agntcy/slim/issues/1139)) ([34af1c7](https://github.com/agntcy/slim/commit/34af1c74f469ae92498205390b1dabe6a17314f6))
* support for managing a slim node instance local to the tool ([#1015](https://github.com/agntcy/slim/issues/1015)) ([c7bbd29](https://github.com/agntcy/slim/commit/c7bbd2922d79774c2be3a702910b55d9e49214bc))
* use Controller internal DB to find moderator node ([#992](https://github.com/agntcy/slim/issues/992)) ([0ca09a1](https://github.com/agntcy/slim/commit/0ca09a155f7420ba41f7022118f9680f77e2fdce))


### Bug Fixes

* flaky integration test ([#1109](https://github.com/agntcy/slim/issues/1109)) ([fd8fe52](https://github.com/agntcy/slim/commit/fd8fe52525c102f902ad5f7b855c33e624bb8445))
* return ack from control plane ([#1098](https://github.com/agntcy/slim/issues/1098)) ([7f873b5](https://github.com/agntcy/slim/commit/7f873b578b4cf4146b4b6aa6c494a21b5c0b0005))
* vulnerability GO-2026-4337 ([#1213](https://github.com/agntcy/slim/issues/1213)) ([be83dbe](https://github.com/agntcy/slim/commit/be83dbebced5486b4b524eea7c5cb684ee246caa))

## [1.0.0](https://github.com/agntcy/slim/compare/control-plane-v0.7.1...control-plane-v1.0.0) (2026-01-30)


### Features

* slimctl: show all properties of outlined routes ([#1002](https://github.com/agntcy/slim/issues/1002)) ([33fd62f](https://github.com/agntcy/slim/commit/33fd62f94a4d5a30a7a886fc44ed60ddd054b18c))
* Support different trust domains in auto route setup ([#1001](https://github.com/agntcy/slim/issues/1001)) ([9a7e930](https://github.com/agntcy/slim/commit/9a7e930f167751f185592c33d11b14f942ee9bb1))
* support for configuring the locally started slim instance ([#1139](https://github.com/agntcy/slim/issues/1139)) ([34af1c7](https://github.com/agntcy/slim/commit/34af1c74f469ae92498205390b1dabe6a17314f6))
* support for managing a slim node instance local to the tool ([#1015](https://github.com/agntcy/slim/issues/1015)) ([c7bbd29](https://github.com/agntcy/slim/commit/c7bbd2922d79774c2be3a702910b55d9e49214bc))
* use Controller internal DB to find moderator node ([#992](https://github.com/agntcy/slim/issues/992)) ([0ca09a1](https://github.com/agntcy/slim/commit/0ca09a155f7420ba41f7022118f9680f77e2fdce))


### Bug Fixes

* flaky integration test ([#1109](https://github.com/agntcy/slim/issues/1109)) ([fd8fe52](https://github.com/agntcy/slim/commit/fd8fe52525c102f902ad5f7b855c33e624bb8445))
* return ack from control plane ([#1098](https://github.com/agntcy/slim/issues/1098)) ([7f873b5](https://github.com/agntcy/slim/commit/7f873b578b4cf4146b4b6aa6c494a21b5c0b0005))

## [0.7.1](https://github.com/agntcy/slim/compare/control-plane-v0.7.0...control-plane-v0.7.1) (2025-11-19)


### Bug Fixes

* set groupName as trust domain ([#968](https://github.com/agntcy/slim/issues/968)) ([32137ef](https://github.com/agntcy/slim/commit/32137ef9412d99c4db8a471e360b783eb83eb75e))

## [0.6.1](https://github.com/agntcy/slim/compare/control-plane-v0.6.0...control-plane-v0.7.0) (2025-11-18)


### Features

* outline routes (list routes from the controller) ([#871](https://github.com/agntcy/slim/issues/871)) ([278ec7d](https://github.com/agntcy/slim/commit/278ec7da6f85140da3b43a723773423386ec07a5))
* support data-plane spire configs ([#940](https://github.com/agntcy/slim/issues/940)) ([ab80876](https://github.com/agntcy/slim/commit/ab8087635fcb677bdcc7a698f9c947ea8913418c))
* use slqlite for Controller persistence ([#916](https://github.com/agntcy/slim/issues/916)) ([706916a](https://github.com/agntcy/slim/commit/706916a850cea78f46db6590ba92e37bc0c83f3c))


### Bug Fixes

* add original MsgID to all response messages ([#891](https://github.com/agntcy/slim/issues/891)) ([f297d5b](https://github.com/agntcy/slim/commit/f297d5bf1062994eca94bd07ada915e11f1d32f9))
* Handle route connection faliures and node connection detail changes ([#833](https://github.com/agntcy/slim/issues/833)) ([8027c3b](https://github.com/agntcy/slim/commit/8027c3b0a11a7d3c2b57184d2313e18d5de6ba3b))
* store componentID as string in sqlight db ([#958](https://github.com/agntcy/slim/issues/958)) ([2c2d175](https://github.com/agntcy/slim/commit/2c2d1754799f8b2de5f2b922c5dc084e9920876e))
* upgraded to golang version 1.25.4 ([#923](https://github.com/agntcy/slim/issues/923)) ([82cabef](https://github.com/agntcy/slim/commit/82cabef4e744fa7954559d06aa97e81d3e4eef3a))

## [0.6.0](https://github.com/agntcy/slim/compare/control-plane-v0.1.1...control-plane-v0.6.0) (2025-10-09)


### Features

* handle updates from SLIM nodes ([#708](https://github.com/agntcy/slim/issues/708)) ([ccc5183](https://github.com/agntcy/slim/commit/ccc518386d0ece16237647511118e7d032e033c6))
* implement control plane group management ([#554](https://github.com/agntcy/slim/issues/554)) ([d0065a0](https://github.com/agntcy/slim/commit/d0065a0e1955dbc7e7fd2bfabd5fdca210459a0b))


### Bug Fixes

* add group id to node id ([#746](https://github.com/agntcy/slim/issues/746)) ([06c42b3](https://github.com/agntcy/slim/commit/06c42b3f3846da331554ac72ec6d77e61876d78d))

## [0.1.1](https://github.com/agntcy/slim/compare/control-plane-v0.1.0...control-plane-v0.1.1) (2025-09-18)


### Features

* add control-plane tests and combined coverage workflow ([#664](https://github.com/agntcy/slim/issues/664)) ([8836b30](https://github.com/agntcy/slim/commit/8836b30fc52e6050291e453df1927531a859e27f))
* add original messageID check for Ack ([#583](https://github.com/agntcy/slim/issues/583)) ([899c116](https://github.com/agntcy/slim/commit/899c11652c82c8a9512ca5928b12f8f45d2dcac3))
* notify controller with new subscriptions ([#611](https://github.com/agntcy/slim/issues/611)) ([6c64b28](https://github.com/agntcy/slim/commit/6c64b28ddbe6c64dbdbd202ac70a32fd9c8e9556))
* Update SB API in control-plane to support group crud operations ([#478](https://github.com/agntcy/slim/issues/478)) ([c7b9afb](https://github.com/agntcy/slim/commit/c7b9afb139bac536f22a99f2a4e8185ad95af788))


### Bug Fixes

* add waitgroup when starting grpc servers in Controler ([#675](https://github.com/agntcy/slim/issues/675)) ([3b6f972](https://github.com/agntcy/slim/commit/3b6f97297702678805515b0ce34eecaa1ec4e2c9))
* added host and port in the NodeEntry ([#560](https://github.com/agntcy/slim/issues/560)) ([dd53b01](https://github.com/agntcy/slim/commit/dd53b016891cb7e9d1cc066f000ef21b4ae14dfd))

## [0.1.0](https://github.com/agntcy/slim/compare/control-plane-v0.0.1...control-plane-v0.1.0) (2025-07-31)


### Features

* add api endpoints for group management ([#450](https://github.com/agntcy/slim/issues/450)) ([dd828c3](https://github.com/agntcy/slim/commit/dd828c3bef6004ae3455987a13dbf8ebefd05695))
* control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/issues/388)) ([8a17dba](https://github.com/agntcy/slim/commit/8a17dbad99fa679e07585ca4fbcefe9cb3fa8a29))
* group svc backend with inmem db ([#456](https://github.com/agntcy/slim/issues/456)) ([a50a156](https://github.com/agntcy/slim/commit/a50a15610508774ff811edf88d1c2b251f622410))


### Bug Fixes

* control plane config is not loaded ([#452](https://github.com/agntcy/slim/issues/452)) ([97eb609](https://github.com/agntcy/slim/commit/97eb609ad176342769214837e57af989d4075a50))

## 0.0.1 (2025-07-24)


### Features

* add api endpoints for group management ([#450](https://github.com/agntcy/slim/issues/450)) ([dd828c3](https://github.com/agntcy/slim/commit/dd828c3bef6004ae3455987a13dbf8ebefd05695))
* control plane service & slimctl cp commands ([#388](https://github.com/agntcy/slim/issues/388)) ([8a17dba](https://github.com/agntcy/slim/commit/8a17dbad99fa679e07585ca4fbcefe9cb3fa8a29))


### Bug Fixes

* control plane config is not loaded ([#452](https://github.com/agntcy/slim/issues/452)) ([97eb609](https://github.com/agntcy/slim/commit/97eb609ad176342769214837e57af989d4075a50))
