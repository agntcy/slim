# Changelog

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
