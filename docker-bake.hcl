# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0


# Documentation available at: https://docs.docker.com/build/bake/

# Docker build args
variable "IMAGE_REPO" {default = ""}
variable "IMAGE_TAG" {default = "v0.0.0-dev"}

function "get_tag" {
  params = [tags, name]
  result = [for tag in tags: "${IMAGE_REPO}/${name}:${tag}"]
}

group "default" {
  targets = [
    "gw",
  ]
}

group "data-plane" {
  targets = [
    "gw",
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

target "gw" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  target = "gateway-release"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.gw.name}")
}

target "gw-debug" {
  context = "."
  dockerfile = "./data-plane/Dockerfile"
  target = "gateway-debug"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.gw-debug.name}")
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
  context = "./data-plane/integrations/mcp/agp-mcp"
  dockerfile = "./examples/mcp-server-time/Dockerfile"
  target = "mcp-server-time"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.mcp-server-time.name}")
}

target "llamaindex-time-agent" {
  context = "./data-plane/integrations/mcp/agp-mcp"
  dockerfile = "./examples/llamaindex-time-agent/Dockerfile"
  target = "llamaindex-time-agent"
  inherits = [
    "_common",
    "docker-metadata-action",
  ]
  tags = get_tag(target.docker-metadata-action.tags, "${target.llamaindex-time-agent.name}")
}