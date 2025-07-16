{{/*
Expand the name of the chart.
*/}}
{{- define "slim-control-plane.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "slim-control-plane.fullname" -}}
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
Token service name
*/}}
{{- define "slim-control-plane.tokenServiceName" -}}
{{ include "slim-control-plane.fullname" . }}-token-service
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "slim-control-plane.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "slim-control-plane.labels" -}}
helm.sh/chart: {{ include "slim-control-plane.chart" . }}
{{ include "slim-control-plane.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "slim-control-plane.selectorLabels" -}}
app.kubernetes.io/name: {{ include "slim-control-plane.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Token service labels
*/}}
{{- define "slim-control-plane.tokenServiceLabels" -}}
helm.sh/chart: {{ include "slim-control-plane.chart" . }}
{{ include "slim-control-plane.tokenServiceSelectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels for token service
*/}}
{{- define "slim-control-plane.tokenServiceSelectorLabels" -}}
app.kubernetes.io/name: {{ include "slim-control-plane.tokenServiceName" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "slim-control-plane.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "slim-control-plane.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}