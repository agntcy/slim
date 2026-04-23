{{/*
Expand the name of the chart.
*/}}
{{- define "spike-nexus.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "spike-nexus.fullname" -}}
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
Allow the release namespace to be overridden for multi-namespace deployments in combined charts
*/}}
{{- define "spike-nexus.namespace" -}}
  {{- if .Values.namespaceOverride -}}
    {{- .Values.namespaceOverride -}}
  {{- else if and (dig "spire" "recommendations" "enabled" false .Values.global) (dig "spire" "recommendations" "namespaceLayout" true .Values.global) }}
    {{- if ne (len (dig "spire" "namespaces" "server" "name" "" .Values.global)) 0 }}
      {{- .Values.global.spire.namespaces.server.name }}
    {{- else }}
      {{- printf "spire-server" }}
    {{- end }}
  {{- else -}}
    {{- .Release.Namespace -}}
  {{- end -}}
{{- end -}}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "spike-nexus.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "spike-nexus.labels" -}}
helm.sh/chart: {{ include "spike-nexus.chart" . }}
{{ include "spike-nexus.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "spike-nexus.selectorLabels" -}}
app.kubernetes.io/name: {{ include "spike-nexus.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "spike-nexus.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "spike-nexus.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{- define "spike-nexus.workload-api-socket-path" -}}
{{- printf "/spiffe-workload-api/%s" .Values.agentSocketName }}
{{- end }}
