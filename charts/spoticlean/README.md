# spoticlean

![Version: 0.1.0](https://img.shields.io/badge/Version-0.1.0-informational?style=flat-square) ![Type: application](https://img.shields.io/badge/Type-application-informational?style=flat-square) ![AppVersion: 0.1.0](https://img.shields.io/badge/AppVersion-0.1.0-informational?style=flat-square)

A web app that finds the duplicates among your Spotify Liked Songs and removes the ones you do not want

The chart deploys the spoticlean web app as one stateless Deployment, with a
Service and an optional Ingress. See the [root README](../../README.md#deployment)
for installing it and for the Spotify app it needs.

## Secrets

`SPOTIFY_CLIENT_ID` and `SESSION_KEY` come from Kubernetes Secrets. Point
`app.spotify.existingSecret` and `app.session.existingSecret` at Secrets you
manage (`name`, and `key` when the entry is named differently from the
default); the chart then generates nothing. Otherwise `app.spotify.clientId`
and `app.session.key` fill Secrets the chart generates; one of the two forms is
required for each. Never commit a real value in a values file.

## Public URL and ingress

`PUBLIC_URL` is `app.publicUrl` when set, else `https://<app.ingress.host>`
(`http://` when `app.ingress.tls.enabled` is false). The redirect URI to
register in the Spotify developer dashboard is that origin followed by
`/auth/callback`. The ingress class and the cert-manager issuer are set with
`app.ingress.className` and `app.ingress.annotations`.

## Values

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| app.affinity | object | `{}` | Affinity of the pod. |
| app.caCerts | object | `{"existing":{"key":"ca-certificates.crt","kind":"ConfigMap","name":null},"inline":null}` | Extra CA certificates to trust for outbound TLS connections to Spotify, mounted at the path `SSL_CERT_FILE` points to. The bundle replaces the system roots the application would otherwise load, so it must hold every root needed to reach `accounts.spotify.com` and `api.spotify.com`. |
| app.caCerts.existing | object | `{"key":"ca-certificates.crt","kind":"ConfigMap","name":null}` | Mount the PEM bundle from a resource you already manage, instead of `inline`. Takes precedence over `inline` when `name` is set. |
| app.caCerts.existing.key | string | `"ca-certificates.crt"` | Key in the resource holding the PEM bundle (also the mounted file name). |
| app.caCerts.existing.kind | string | `"ConfigMap"` | Kind of the resource, `ConfigMap` or `Secret`. |
| app.caCerts.existing.name | string | `nil` | Name of the resource. Set, the bundle is mounted from it instead of a generated Secret. |
| app.caCerts.inline | string | `nil` | Inline PEM bundle, stored in a Secret and mounted. Ignored when `existing.name` is set. |
| app.extraEnv | list | `[]` | Extra environment variables for the container, as Kubernetes `EnvVar` objects. |
| app.extraVolumeMounts | list | `[]` | Extra volume mounts for the container. |
| app.extraVolumes | list | `[]` | Extra volumes for the pod. |
| app.fullnameOverride | string | `nil` | Overrides the full name of the resources. |
| app.image.pullPolicy | string | `"IfNotPresent"` | Image pull policy. |
| app.image.repository | string | `"ghcr.io/leroyguillaume/spoticlean"` | Image repository. |
| app.image.tag | string | `nil` | Image tag. Defaults to the chart's `appVersion`. |
| app.imagePullSecrets | list | `nil` | Image pull secrets of this component. Unset, `global.imagePullSecrets` applies. |
| app.ingress.annotations | object | `{}` | Annotations on the Ingress, for example `cert-manager.io/cluster-issuer` or controller settings. |
| app.ingress.className | string | `nil` | Ingress class name. Unset, the cluster's default class applies. |
| app.ingress.enabled | bool | `false` | Create an Ingress. |
| app.ingress.host | string | `nil` | Host the app is served at. Also derives `publicUrl` when that is unset. |
| app.ingress.path | string | `"/"` | Path routed to the app. |
| app.ingress.pathType | string | `"Prefix"` | Path type of that path. |
| app.ingress.tls.enabled | bool | `true` | Serve the host over TLS. It also makes the derived `publicUrl` HTTPS, which makes the cookies `Secure`. |
| app.ingress.tls.secretName | string | `nil` | Name of the Secret holding the certificate. Defaults to `<fullname>-tls`, which cert-manager fills when the annotations ask for it. |
| app.log.filter | string | `nil` | `tracing` filter directive (`LOG_FILTER`), for example `spoticlean=debug`. Unset, the application default applies. |
| app.log.format | string | `nil` | Log format on stderr, `text` or `json` (`LOG_FORMAT`). Unset, the application default applies. |
| app.matching.durationTolerance | int | `nil` | Largest duration gap, in seconds, between two copies of a song (`DURATION_TOLERANCE`). Unset, the application default applies. |
| app.nameOverride | string | `nil` | Overrides the chart name in resource names and labels. |
| app.nodeSelector | object | `{}` | Node selector of the pod. |
| app.podAnnotations | object | `{}` | Annotations on the pod. |
| app.podLabels | object | `{}` | Labels on the pod. |
| app.podSecurityContext | object | `{"fsGroup":65532,"runAsGroup":65532,"runAsNonRoot":true,"runAsUser":65532,"seccompProfile":{"type":"RuntimeDefault"}}` | Pod-level security context. |
| app.probes.liveness.initialDelaySeconds | int | `5` | Seconds before the liveness probe starts. |
| app.probes.liveness.periodSeconds | int | `10` | Seconds between liveness probes. |
| app.probes.readiness.initialDelaySeconds | int | `2` | Seconds before the readiness probe starts. |
| app.probes.readiness.periodSeconds | int | `5` | Seconds between readiness probes. |
| app.publicUrl | string | `nil` | Origin the browser reaches the app at, for example `https://spoticlean.example`. Unset, it is derived from `ingress.host` and `ingress.tls.enabled`. The Spotify redirect URI is this value followed by `/auth/callback`. |
| app.replicaCount | int | `1` | Number of replicas. The server is stateless and sessions live in an encrypted cookie, so any number works as long as every replica shares the session key. |
| app.resources | object | `{"limits":{"ephemeral-storage":"256Mi","memory":"128Mi"},"requests":{"cpu":"50m","ephemeral-storage":"64Mi","memory":"64Mi"}}` | Resource requests and limits. No CPU limit: it only throttles. |
| app.revisionHistoryLimit | int | `0` | Number of old ReplicaSets the Deployment keeps for rollback. |
| app.scan.concurrency | int | `nil` | Pages of Liked Songs fetched at the same time, 1 to 16 (`SCAN_CONCURRENCY`). Unset, the application default applies. |
| app.securityContext | object | `{"allowPrivilegeEscalation":false,"capabilities":{"drop":["ALL"]},"readOnlyRootFilesystem":true}` | Container-level security context. |
| app.service.annotations | object | `{}` | Annotations on the Service. |
| app.service.port | int | `80` | Port the Service exposes. |
| app.service.type | string | `"ClusterIP"` | Service type. |
| app.serviceAccount.annotations | object | `{}` | Annotations on the ServiceAccount. |
| app.serviceAccount.create | bool | `true` | Create a ServiceAccount for the pod. |
| app.serviceAccount.name | string | `nil` | Name of the ServiceAccount. Defaults to the full name when created, `default` otherwise. |
| app.session.existingSecret.key | string | `"SESSION_KEY"` | Key in that Secret holding the session key. |
| app.session.existingSecret.name | string | `nil` | Name of a pre-existing Secret holding the session key. Set, it takes precedence over `key`. |
| app.session.key | string | `nil` | Base64 of at least 64 random bytes (`openssl rand -base64 64`), stored in a generated Secret. Ignored when `existingSecret.name` is set. Changing it logs everyone out. |
| app.spotify.clientId | string | `nil` | Client ID of the Spotify app, stored in a generated Secret. Ignored when `existingSecret.name` is set. Not a secret by itself, but kept with the credentials. |
| app.spotify.existingSecret.key | string | `"SPOTIFY_CLIENT_ID"` | Key in that Secret holding the client ID. |
| app.spotify.existingSecret.name | string | `nil` | Name of a pre-existing Secret holding the client ID. Set, it takes precedence over `clientId`. |
| app.terminationGracePeriodSeconds | int | `30` | Seconds the pod gets to finish in-flight requests after `SIGTERM`. |
| app.tolerations | list | `[]` | Tolerations of the pod. |
| global.imagePullSecrets | list | `[]` | Secrets used to pull the image from a private registry, as a list of `{name: <secret>}` objects. Overridable per component by setting the same key in its block. |
