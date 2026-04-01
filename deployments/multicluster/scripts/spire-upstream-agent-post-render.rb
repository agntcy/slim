#!/usr/bin/env ruby
# frozen_string_literal: true
#
# Helm post-renderer for spiffe/spire-nested child installs: replace upstream agent
# k8s_psat with join_token and inject join-token bootstrap (Secret keys = node names).
# Usage: stdin/stdout YAML stream (Helm). Optional args from --post-renderer-args are ignored.

require 'json'
require 'yaml'

JOIN_INIT = {
  'name' => 'join-token-bootstrap',
  'image' => 'cgr.dev/chainguard/bash:latest@sha256:3a1830320b1d57167a7317fcd6efd8c72cd872440da8055fa25730d600c9c39a',
  'imagePullPolicy' => 'IfNotPresent',
  'command' => ['bash', '-c'],
  'args' => [<<~SH.strip],
    set -euo pipefail
    f="/join-secrets/${NODE_NAME}"
    test -f "$f"
    cp "$f" /bootstrap/join_token
    chmod 600 /bootstrap/join_token
  SH
  'env' => [
    { 'name' => 'NODE_NAME', 'valueFrom' => { 'fieldRef' => { 'fieldPath' => 'spec.nodeName' } } }
  ],
  'volumeMounts' => [
    { 'name' => 'join-token-secrets', 'mountPath' => '/join-secrets', 'readOnly' => true },
    { 'name' => 'join-bootstrap', 'mountPath' => '/bootstrap' }
  ],
  'securityContext' => { 'runAsUser' => 0, 'runAsGroup' => 0 }
}.freeze

VOL_SECRET = {
  'name' => 'join-token-secrets',
  'secret' => { 'secretName' => 'spire-upstream-agent-join-tokens' }
}.freeze

VOL_BOOTSTRAP = {
  'name' => 'join-bootstrap',
  'emptyDir' => { 'medium' => 'Memory', 'sizeLimit' => '1Mi' }
}.freeze

def patch_upstream_configmap(doc)
  return unless doc['data'] && doc['data']['agent.conf']

  j = JSON.parse(doc['data']['agent.conf'])
  j['plugins'] ||= {}
  j['plugins']['NodeAttestor'] = [{ 'join_token' => { 'plugin_data' => {} } }]
  doc['data']['agent.conf'] = JSON.pretty_generate(j) + "\n"
end

def patch_upstream_daemonset(doc)
  spec = doc.dig('spec', 'template', 'spec')
  return unless spec

  inits = spec['initContainers'] || []
  unless inits.any? { |c| c['name'] == 'join-token-bootstrap' }
    spec['initContainers'] = [JOIN_INIT.dup] + inits
  end

  vols = spec['volumes'] || []
  unless vols.any? { |v| v['name'] == 'join-token-secrets' }
    vols = vols + [VOL_SECRET.dup, VOL_BOOTSTRAP.dup]
  end
  spec['volumes'] = vols

  ctrs = spec['containers'] || []
  ctr = ctrs.find { |c| c['name'] == 'upstream-spire-agent' }
  return unless ctr

  args = ctr['args'] || []
  unless args.include?('-joinTokenFile')
    ctr['args'] = args + ['-joinTokenFile', '/run/spire/bootstrap/join_token']
  end

  mounts = ctr['volumeMounts'] || []
  unless mounts.any? { |m| m['name'] == 'join-bootstrap' }
    mounts << {
      'name' => 'join-bootstrap',
      'mountPath' => '/run/spire/bootstrap',
      'readOnly' => true
    }
    ctr['volumeMounts'] = mounts
  end
end

input = STDIN.read
docs = YAML.load_stream(input).compact

docs.each do |doc|
  next unless doc.is_a?(Hash)

  if doc['kind'] == 'ConfigMap' && doc.dig('metadata', 'name') == 'spire-agent-upstream'
    patch_upstream_configmap(doc)
  elsif doc['kind'] == 'DaemonSet' && doc.dig('metadata', 'name') == 'spire-agent-upstream'
    patch_upstream_daemonset(doc)
  end
end

# Psych omits document separator for single doc; Helm uses multi-doc stream.
puts Psych.dump_stream(*docs)
