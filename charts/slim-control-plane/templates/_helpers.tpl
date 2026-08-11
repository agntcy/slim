{{/*
Copyright AGNTCY Contributors (https://github.com/agntcy)
SPDX-License-Identifier: Apache-2.0
*/}}

{{/*
Expand the name of the chart.
*/}}
{{- define "control-plane.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "control-plane.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "control-plane.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "control-plane.labels" -}}
helm.sh/chart: {{ include "control-plane.chart" . }}
{{ include "control-plane.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "control-plane.selectorLabels" -}}
app.kubernetes.io/name: {{ include "control-plane.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "control-plane.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "control-plane.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "control-plane.pvcName" -}}
{{- if .Values.persistence.existingClaim }}
{{- .Values.persistence.existingClaim }}
{{- else }}
{{- printf "%s-db" (include "control-plane.fullname" .) | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}

{{/*
Normalize one API bound's listeners into a YAML list of {name, port, tls}.

`service.north` / `service.south` are the single source of truth for a bound's
listeners: the Service ports, the container ports, and the control-plane config's
own `northbound` / `southbound` entries are all derived from them, so the number
of listeners is declared exactly once. Accepts either shape:

  service.north.port: 50051                    # single listener
  service.north.ports:                         # several listeners
    - name: north
      port: 50051
    - name: north-tls
      port: 50451
      tls:                                     # per-listener, optional
        insecure: false
        source:
          type: spire
          socket_path: "unix:///run/spire/agent-sockets/api.sock"

`ports` wins when both are set. Names default to "<prefix>-<index>" and are
truncated to the 15 characters Kubernetes allows for a port name. `tls` falls
back to the bound-level `service.<bound>.tls`, then to `insecure: true`.

Call with: (dict "bound" .Values.service.north "prefix" "north")
Consume with: `| fromYamlArray`
*/}}
{{- define "control-plane.listenerPorts" -}}
{{- $bound := .bound | default dict -}}
{{- $prefix := .prefix -}}
{{- $boundTls := $bound.tls | default (dict "insecure" true) -}}
{{- if $bound.ports -}}
{{- range $i, $p := $bound.ports }}
- name: {{ $p.name | default (printf "%s-%d" $prefix $i) | trunc 15 | trimSuffix "-" | quote }}
  port: {{ $p.port | required (printf "service.%s.ports[%d].port is required" $prefix $i) }}
  tls:
    {{- toYaml ($p.tls | default $boundTls) | nindent 4 }}
{{- end }}
{{- else }}
- name: {{ $prefix | quote }}
  port: {{ $bound.port | required (printf "service.%s.port (or service.%s.ports) is required" $prefix $prefix) }}
  tls:
    {{- toYaml $boundTls | nindent 4 }}
{{- end }}
{{- end -}}

{{/*
The primary port for a bound — the first listener. Used where a single port must
be named, such as an Ingress backend.
*/}}
{{- define "control-plane.primaryPort" -}}
{{- $ports := include "control-plane.listenerPorts" . | fromYamlArray -}}
{{- (first $ports).port -}}
{{- end -}}

{{/*
The control-plane `northbound` / `southbound` config for a bound, derived from
its listener ports so the endpoints cannot drift from what the Service exposes.

Emits a single mapping for one listener (the shape existing configs use) and a
sequence for several — both of which the control plane accepts.

`service.<bound>.bindAddress` overrides the bind host, default "0.0.0.0".
Setting `config.northbound` / `config.southbound` explicitly bypasses this
entirely; see configmap.yaml.
*/}}
{{- define "control-plane.boundConfig" -}}
{{- $bound := .bound | default dict -}}
{{- $prefix := .prefix -}}
{{- $host := $bound.bindAddress | default "0.0.0.0" -}}
{{- $boundTls := $bound.tls | default (dict "insecure" true) -}}
{{- /*
  Anything on the bound that is not port plumbing is a ServerConfig field
  (auth, keepalive, max_frame_size, …) and applies to every derived listener;
  the same keys on an individual port entry override it for that listener.
*/}}
{{- $boundExtra := omit $bound "port" "ports" "tls" "bindAddress" -}}
{{- $entries := $bound.ports | default (list (dict "port" ($bound.port | required (printf "service.%s.port (or service.%s.ports) is required" $prefix $prefix)))) -}}
{{- $servers := list -}}
{{- range $entries -}}
{{- $base := dict "endpoint" (printf "%s:%v" $host .port) "tls" (.tls | default $boundTls) -}}
{{- $servers = append $servers (merge $base (omit . "name" "port" "tls") $boundExtra) -}}
{{- end -}}
{{- if eq (len $servers) 1 -}}
{{- first $servers | toYaml -}}
{{- else -}}
{{- $servers | toYaml -}}
{{- end -}}
{{- end -}}
