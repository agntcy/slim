# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0


# Documentation available at: https://docs.docker.com/build/bake/

# Docker build args
variable "IMAGE_REPO" { default = "" }
variable "IMAGE_TAG" { default = "latest" }

function "get_tag" {
  params = [tags, name]
  // Check if IMAGE_REPO ends with name to avoid repetition
  result = [for tag in coalescelist(tags, [IMAGE_TAG]):
    can(regex("${name}$", IMAGE_REPO)) ?
      "${IMAGE_REPO}:${tag}" :
      "${IMAGE_REPO}/${name}:${tag}"
  ]
}

group "default" {
  targets = [
    "slim",
  ]
}

group "data-plane" {
  targets = [
    "slim",
  ]
}

group "control-plane" {
  targets = [
    "control-plane",
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

target "slim" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  target = "slim-release"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.slim.name}")
}

target "slim-debug" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  target = "slim-debug"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.slim-debug.name}")
}

target "control-plane" {
  context = "."
  dockerfile = "./control-plane/Dockerfile"
  target     = "control-plane"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.control-plane.name}")
}
