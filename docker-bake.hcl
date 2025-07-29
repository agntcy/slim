# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0


# Documentation available at: https://docs.docker.com/build/bake/

# Docker build args
variable "IMAGE_REPO" { default = "" }
variable "IMAGE_TAG" { default = "v0.0.0-dev" }

function "get_tag" {
  params = [tags, name]
  // Check if IMAGE_REPO ends with name to avoid repetition
  result = [for tag in tags:
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

target "mcp-proxy" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  target = "mcp-proxy-release"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.mcp-proxy.name}")
}

target "mcp-proxy-debug" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  target = "mcp-proxy-debug"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.mcp-proxy-debug.name}")
}

target "mcp-server-time" {
  context = "./data-plane/integrations/mcp/slim-mcp"
  dockerfile = "./examples/mcp-server-time/Dockerfile"
  target = "mcp-server-time"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.mcp-server-time.name}")
}

target "llamaindex-time-agent" {
  context = "./data-plane/integrations/mcp/slim-mcp"
  dockerfile = "./examples/llamaindex-time-agent/Dockerfile"
  target = "llamaindex-time-agent"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.llamaindex-time-agent.name}")
}

target "testutils" {
  context = "./data-plane"
  dockerfile = "./testing/Dockerfile"
  target = "testutils"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.testutils.name}")
}

target "control-plane" {
  contexts = {
    src = "."
  }
  dockerfile = "./control-plane/Dockerfile"
  target     = "control-plane"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.control-plane.name}")
}

target "slim-bindings-examples" {
  contexts = {
    src = "."
  }
  dockerfile = "./data-plane/python-bindings/examples/Dockerfile"
  target     = "slim-bindings-examples"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.slim-bindings-examples.name}")
}
