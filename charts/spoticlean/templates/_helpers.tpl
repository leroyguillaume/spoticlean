{{- define "spoticlean.name" -}}
{{- default .Chart.Name .Values.app.nameOverride | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "spoticlean.fullname" -}}
{{- if .Values.app.fullnameOverride -}}
{{- .Values.app.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- $name := default .Chart.Name .Values.app.nameOverride -}}
{{- if contains $name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "spoticlean.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "spoticlean.labels" -}}
helm.sh/chart: {{ include "spoticlean.chart" . }}
{{ include "spoticlean.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "spoticlean.selectorLabels" -}}
app.kubernetes.io/name: {{ include "spoticlean.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "spoticlean.serviceAccountName" -}}
{{- if .Values.app.serviceAccount.create -}}
{{- default (include "spoticlean.fullname" .) .Values.app.serviceAccount.name -}}
{{- else -}}
{{- default "default" .Values.app.serviceAccount.name -}}
{{- end -}}
{{- end -}}

{{- define "spoticlean.image" -}}
{{- printf "%s:%s" .Values.app.image.repository (default .Chart.AppVersion .Values.app.image.tag) -}}
{{- end -}}

{{- define "spoticlean.imagePullSecrets" -}}
{{- default .Values.global.imagePullSecrets .Values.app.imagePullSecrets | toYaml -}}
{{- end -}}

{{/* The origin the browser reaches the app at: explicit, else from the ingress. */}}
{{- define "spoticlean.publicUrl" -}}
{{- if .Values.app.publicUrl -}}
{{- .Values.app.publicUrl -}}
{{- else if and .Values.app.ingress.enabled .Values.app.ingress.host -}}
{{- printf "%s://%s" (ternary "https" "http" .Values.app.ingress.tls.enabled) .Values.app.ingress.host -}}
{{- else -}}
{{- fail "app.publicUrl is required unless app.ingress.enabled is true and app.ingress.host is set" -}}
{{- end -}}
{{- end -}}

{{- define "spoticlean.spotifySecretName" -}}
{{- default (printf "%s-spotify" (include "spoticlean.fullname" .)) .Values.app.spotify.existingSecret.name -}}
{{- end -}}

{{- define "spoticlean.spotifySecretKey" -}}
{{- if .Values.app.spotify.existingSecret.name -}}
{{- .Values.app.spotify.existingSecret.key -}}
{{- else -}}
SPOTIFY_CLIENT_ID
{{- end -}}
{{- end -}}

{{- define "spoticlean.sessionSecretName" -}}
{{- default (printf "%s-session" (include "spoticlean.fullname" .)) .Values.app.session.existingSecret.name -}}
{{- end -}}

{{- define "spoticlean.sessionSecretKey" -}}
{{- if .Values.app.session.existingSecret.name -}}
{{- .Values.app.session.existingSecret.key -}}
{{- else -}}
SESSION_KEY
{{- end -}}
{{- end -}}

{{- define "spoticlean.caCertsEnabled" -}}
{{- if or .Values.app.caCerts.inline .Values.app.caCerts.existing.name -}}true{{- end -}}
{{- end -}}

{{- define "spoticlean.caCertsValidate" -}}
{{- if not (has .Values.app.caCerts.existing.kind (list "ConfigMap" "Secret")) -}}
{{- fail "app.caCerts.existing.kind must be ConfigMap or Secret" -}}
{{- end -}}
{{- end -}}

{{- define "spoticlean.caCertsFromConfigMap" -}}
{{- if and .Values.app.caCerts.existing.name (eq .Values.app.caCerts.existing.kind "ConfigMap") -}}true{{- end -}}
{{- end -}}

{{- define "spoticlean.caCertsGenerateSecret" -}}
{{- if and .Values.app.caCerts.inline (not .Values.app.caCerts.existing.name) -}}true{{- end -}}
{{- end -}}

{{- define "spoticlean.caCertsSecretName" -}}
{{- default (printf "%s-ca-certs" (include "spoticlean.fullname" .)) .Values.app.caCerts.existing.name -}}
{{- end -}}

{{- define "spoticlean.caCertsKey" -}}
{{- if .Values.app.caCerts.existing.name -}}
{{- .Values.app.caCerts.existing.key -}}
{{- else -}}
ca-certificates.crt
{{- end -}}
{{- end -}}

{{- define "spoticlean.caCertsPath" -}}
/etc/ssl/custom/{{ include "spoticlean.caCertsKey" . }}
{{- end -}}
