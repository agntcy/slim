{{/*
Copyright AGNTCY Contributors (https://github.com/agntcy)
SPDX-License-Identifier: Apache-2.0
*/}}

{{/*
Expand the name of the chart.
*/}}
{{- define "k8s-troubleshooting-agent.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "k8s-troubleshooting-agent.fullname" -}}
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
{{- define "k8s-troubleshooting-agent.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "k8s-troubleshooting-agent.labels" -}}
helm.sh/chart: {{ include "k8s-troubleshooting-agent.chart" . }}
{{ include "k8s-troubleshooting-agent.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "k8s-troubleshooting-agent.selectorLabels" -}}
app.kubernetes.io/name: {{ include "k8s-troubleshooting-agent.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
Create the name of the service account to use
*/}}
{{- define "k8s-troubleshooting-agent.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "k8s-troubleshooting-agent.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Name of the Secret that holds the SLIM shared secret.
If slim.secretRef.name is set, that existing Secret is used.
Otherwise, a Secret is generated from slim.secret.
*/}}
{{- define "k8s-troubleshooting-agent.slimSecretName" -}}
{{- if .Values.slim.secretRef.name }}
{{- .Values.slim.secretRef.name }}
{{- else }}
{{- include "k8s-troubleshooting-agent.fullname" . }}-slim-secret
{{- end }}
{{- end }}

{{/*
Key inside the SLIM Secret.
*/}}
{{- define "k8s-troubleshooting-agent.slimSecretKey" -}}
{{- .Values.slim.secretRef.key | default "slim-secret" }}
{{- end }}
