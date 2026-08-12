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
Resolve which listener an Ingress exposes.

`ingressNorth.servicePort` / `ingressSouth.servicePort` select a listener by port
name or number. Unset selects the first listener, which is what a
single-listener deployment wants and what this chart did before the setting
existed.

Selecting by name or number rather than by list position matters because only the
selected listener is reachable through the Ingress — the rest stay in-cluster
(with `service.type: ClusterIP`), so getting it backwards silently exposes the
wrong one.

Fails rather than rendering an Ingress pointing at a port the Service does not
publish, which would otherwise show up only as a 503.

Call with:
  (dict "listeners" .Values.service.north
        "want" .Values.ingressNorth.servicePort
        "prefix" "north"
        "key" "ingressNorth.servicePort")
*/}}
{{- define "control-plane.ingressPort" -}}
{{- $listeners := .listeners | default list -}}
{{- if not $listeners -}}
{{- fail (printf "%s: service.%s has no listeners configured" .key .prefix) -}}
{{- end -}}
{{- if not .want -}}
{{- (first $listeners).port -}}
{{- else -}}
{{- $want := toString .want -}}
{{- $match := "" -}}
{{- $names := list -}}
{{- range $i, $l := $listeners -}}
{{- $name := $l.name | default (printf "%s-%d" $.prefix $i) -}}
{{- $names = append $names (printf "%s (%v)" $name $l.port) -}}
{{- if or (eq $want $name) (eq $want (toString $l.port)) -}}
{{- $match = toString $l.port -}}
{{- end -}}
{{- end -}}
{{- if eq $match "" -}}
{{- fail (printf "%s: %q matches no configured listener; available: %s" .key $want (join ", " $names)) -}}
{{- end -}}
{{- $match -}}
{{- end -}}
{{- end -}}
