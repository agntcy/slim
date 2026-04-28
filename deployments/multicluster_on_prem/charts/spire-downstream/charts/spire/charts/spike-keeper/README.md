# spike-keeper

![Version: 0.1.0](https://img.shields.io/badge/Version-0.1.0-informational?style=flat-square) ![Type: application](https://img.shields.io/badge/Type-application-informational?style=flat-square) ![AppVersion: v0.4.1](https://img.shields.io/badge/AppVersion-v0.4.1-informational?style=flat-square)
[![Development Phase](https://github.com/spiffe/spiffe/blob/main/.img/maturity/dev.svg)](https://github.com/spiffe/spiffe/blob/main/MATURITY.md#development)

A Helm chart to deploy spike keepers

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

| Name                               | Description                                                                                                                                                                                                                             | Value                 |
| ---------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------- |
| `image.registry`                   | The OCI registry to pull the image from                                                                                                                                                                                                 | `ghcr.io`             |
| `image.repository`                 | The repository within the registry                                                                                                                                                                                                      | `spiffe/spike-keeper` |
| `image.pullPolicy`                 | The image pull policy                                                                                                                                                                                                                   | `IfNotPresent`        |
| `image.tag`                        | Overrides the image tag whose default is the chart appVersion                                                                                                                                                                           | `""`                  |
| `replicas`                         | The number of keepers to launch                                                                                                                                                                                                         | `3`                   |
| `trustRoot.nexus`                  | Override which trustRoot(s) Nexus is in (array)                                                                                                                                                                                         | `[]`                  |
| `trustRoot.bootstrap`              | Override which trustRoot(s) Bootstrap is in (array)                                                                                                                                                                                     | `[]`                  |
| `trustRoot.self`                   | Override which trustRoot(s) this Keeper instance is in (array)                                                                                                                                                                          | `[]`                  |
| `logLevel`                         | The log level, valid values are "debug", "info", "warn", and "error"                                                                                                                                                                    | `debug`               |
| `agentSocketName`                  | The name of the spire-agent unix socket                                                                                                                                                                                                 | `spire-agent.sock`    |
| `csiDriverName`                    | The csi driver to use                                                                                                                                                                                                                   | `csi.spiffe.io`       |
| `imagePullSecrets`                 | Pull secrets for images                                                                                                                                                                                                                 | `[]`                  |
| `nameOverride`                     | Name override                                                                                                                                                                                                                           | `""`                  |
| `namespaceOverride`                | Namespace override                                                                                                                                                                                                                      | `""`                  |
| `fullnameOverride`                 | Fullname override                                                                                                                                                                                                                       | `""`                  |
| `serviceAccount.create`            | Specifies whether a service account should be created                                                                                                                                                                                   | `true`                |
| `serviceAccount.annotations`       | Annotations to add to the service account                                                                                                                                                                                               | `{}`                  |
| `serviceAccount.name`              | The name of the service account to use. If not set and create is true, a name is generated.                                                                                                                                             | `""`                  |
| `labels`                           | Labels for pods                                                                                                                                                                                                                         | `{}`                  |
| `podSecurityContext`               | Pod security context                                                                                                                                                                                                                    | `{}`                  |
| `securityContext`                  | Security context                                                                                                                                                                                                                        | `{}`                  |
| `service.type`                     | Service type                                                                                                                                                                                                                            | `ClusterIP`           |
| `service.port`                     | Service port                                                                                                                                                                                                                            | `443`                 |
| `service.annotations`              | Annotations for service resource                                                                                                                                                                                                        | `{}`                  |
| `nodeSelector`                     | (Optional) Select specific nodes to run on.                                                                                                                                                                                             | `{}`                  |
| `affinity`                         | Affinity rules                                                                                                                                                                                                                          | `{}`                  |
| `tolerations`                      | List of tolerations                                                                                                                                                                                                                     | `[]`                  |
| `topologySpreadConstraints`        | List of topology spread constraints for resilience                                                                                                                                                                                      | `[]`                  |
| `startupProbe.enabled`             | Enable startupProbe                                                                                                                                                                                                                     | `true`                |
| `startupProbe.initialDelaySeconds` | Initial delay seconds for startupProbe                                                                                                                                                                                                  | `5`                   |
| `startupProbe.periodSeconds`       | Period seconds for startupProbe                                                                                                                                                                                                         | `10`                  |
| `startupProbe.timeoutSeconds`      | Timeout seconds for startupProbe                                                                                                                                                                                                        | `5`                   |
| `startupProbe.failureThreshold`    | Failure threshold count for startupProbe                                                                                                                                                                                                | `6`                   |
| `startupProbe.successThreshold`    | Success threshold count for startupProbe                                                                                                                                                                                                | `1`                   |
| `ingress.enabled`                  | Flag to enable ingress                                                                                                                                                                                                                  | `false`               |
| `ingress.className`                | Ingress class name                                                                                                                                                                                                                      | `""`                  |
| `ingress.controllerType`           | Specify what type of ingress controller you're using to add the necessary annotations accordingly. If blank, auto-detection is attempted. If other, no annotations will be added. Must be one of [ingress-nginx, openshift, other, ""]. | `""`                  |
| `ingress.annotations`              | Annotations                                                                                                                                                                                                                             | `{}`                  |
| `ingress.host`                     | Host name for the ingress. If no '.' in host, trustDomain is automatically appended. The rest of the rules will be autogenerated. For more customizability, use hosts[] instead.                                                        | `keeper`              |
| `ingress.tlsSecret`                | Secret that has the certs. If blank will use default certs. Used with host var.                                                                                                                                                         | `""`                  |
| `ingress.hosts`                    | Host paths for ingress object. If empty, rules will be built based on the host var.                                                                                                                                                     | `[]`                  |
| `ingress.tls`                      | Secrets containing TLS certs to enable https on ingress. If empty, rules will be built based on the host and tlsSecret vars.                                                                                                            | `[]`                  |
