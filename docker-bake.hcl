// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0


# Documentation available at: https://docs.docker.com/build/bake/

# Docker build args
variable "IMAGE_REPO" {default = ""}
variable "IMAGE_TAG" {default = "v0.0.0-dev"}

function "get_tag" {
  params = [tags, name]
  result = coalescelist(tags, ["${IMAGE_REPO}/${name}:${IMAGE_TAG}"])
}

group "default" {
  targets = [
    "gateway-tks",
    "gateway",
  ]
}

group "golang" {
  targets = [
    "gateway-tks",
  ]
}

group "rust" {
  targets = [
    "gateway",
  ]
}

target "_common" {
  output = [
    "type=image",
  ]
  platforms = [
    "linux/arm64",
    "linux/amd64",
  ]
}

target "docker-metadata-action" {
  tags = []
}

target "gateway-tks" {
  context = "."
  dockerfile = "./control-plane/token-service/Dockerfile"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.gateway-tks.name}")
}

target "gateway" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.gateway.name}")
}
