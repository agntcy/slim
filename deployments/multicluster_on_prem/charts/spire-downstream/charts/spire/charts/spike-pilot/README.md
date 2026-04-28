# spike-pilot

![Version: 0.1.0](https://img.shields.io/badge/Version-0.1.0-informational?style=flat-square) ![Type: application](https://img.shields.io/badge/Type-application-informational?style=flat-square) ![AppVersion: v0.4.1](https://img.shields.io/badge/AppVersion-v0.4.1-informational?style=flat-square)
[![Development Phase](https://github.com/spiffe/spiffe/blob/main/.img/maturity/dev.svg)](https://github.com/spiffe/spiffe/blob/main/MATURITY.md#development)

A Helm chart to deploy spike pilot

**Homepage:** <https://github.com/spiffe/helm-charts-hardened/tree/main/charts/spire>

## Version support

> [!Note]
> This Chart is still in development and still subject to change the API (`values.yaml`).
> Until we reach a `1.0.0` version of the chart we can't guarantee backwards compatibility although
> we do aim for as much stability as possible.

| Dependency | Supported Versions |
|:-----------|:-------------------|
| Helm       | `3.x`              |

## Source Code

* <https://github.com/spiffe/spike>

<!-- The parameters section is generated using helm-docs.sh and should not be edited by hand. -->

## Parameters

### Chart parameters

| Name                             | Description                                                                                 | Value                |
| -------------------------------- | ------------------------------------------------------------------------------------------- | -------------------- |
| `image.registry`                 | The OCI registry to pull the image from                                                     | `ghcr.io`            |
| `image.repository`               | The repository within the registry                                                          | `spiffe/spike-pilot` |
| `image.pullPolicy`               | The image pull policy                                                                       | `IfNotPresent`       |
| `image.tag`                      | Overrides the image tag whose default is the chart appVersion                               | `""`                 |
| `shell.image.registry`           | The OCI registry to pull the image from                                                     | `""`                 |
| `shell.image.repository`         | The repository within the registry                                                          | `busybox`            |
| `shell.image.pullPolicy`         | The image pull policy                                                                       | `IfNotPresent`       |
| `shell.image.tag`                | Overrides the image tag whose default is the chart appVersion                               | `1.37.0-uclibc`      |
| `tools.busybox.image.registry`   | The OCI registry to pull the image from                                                     | `""`                 |
| `tools.busybox.image.repository` | The repository within the registry                                                          | `busybox`            |
| `tools.busybox.image.pullPolicy` | The image pull policy                                                                       | `IfNotPresent`       |
| `tools.busybox.image.tag`        | Overrides the image tag whose default is the chart appVersion                               | `1.37.0-uclibc`      |
| `replicas`                       | The number of keepers to launch                                                             | `1`                  |
| `trustRoot.nexus`                | Override which trustRoot(s) Nexus is in (array)                                             | `[]`                 |
| `trustRoot.pilot`                | Override which trustRoot(s) this SPIKE Pilot instance is in (array)                         | `[]`                 |
| `logLevel`                       | The log level, valid values are "debug", "info", "warn", and "error"                        | `debug`              |
| `agentSocketName`                | The name of the spire-agent unix socket                                                     | `spire-agent.sock`   |
| `csiDriverName`                  | The csi driver to use                                                                       | `csi.spiffe.io`      |
| `imagePullSecrets`               | Pull secrets for images                                                                     | `[]`                 |
| `nameOverride`                   | Name override                                                                               | `""`                 |
| `namespaceOverride`              | Namespace override                                                                          | `""`                 |
| `fullnameOverride`               | Fullname override                                                                           | `""`                 |
| `serviceAccount.create`          | Specifies whether a service account should be created                                       | `true`               |
| `serviceAccount.annotations`     | Annotations to add to the service account                                                   | `{}`                 |
| `serviceAccount.name`            | The name of the service account to use. If not set and create is true, a name is generated. | `""`                 |
| `labels`                         | Labels for pods                                                                             | `{}`                 |
| `podSecurityContext`             | Pod security context                                                                        | `{}`                 |
| `securityContext`                | Security context                                                                            | `{}`                 |
| `nodeSelector`                   | (Optional) Select specific nodes to run on.                                                 | `{}`                 |
| `affinity`                       | Affinity rules                                                                              | `{}`                 |
| `tolerations`                    | List of tolerations                                                                         | `[]`                 |
| `topologySpreadConstraints`      | List of topology spread constraints for resilience                                          | `[]`                 |
