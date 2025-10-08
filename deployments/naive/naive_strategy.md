# Naive Deployment Strategy

## Description

The naive deployment strategy provides the simplest approach to deploying SLIM in a Kubernetes cluster. This strategy is designed for development, testing, and proof-of-concept scenarios where ease of deployment takes precedence over production-grade features like high availability or complex configurations.

**Target Audience:**

- Developers getting started with SLIM.
- Testing and development environments.
- Quick demonstrations and prototypes.

**Use Cases:**

- Local development and testing.
- Initial SLIM evaluation.
- Simple single-instance deployments.
- Educational and learning purposes.

## Details

The naive deployment strategy deploys SLIM components with minimal configuration:

- Single instance deployment (no high availability).
- Basic configuration setup.
- Minimal external dependencies.

This approach prioritizes simplicity and quick startup time over production readiness. It's ideal for environments where you need to quickly spin up a SLIM instance to test functionality or demonstrate capabilities.

![SLIM Naive Deployment Diagram](img/slim_naive.svg)

## Usage

Follow these steps to deploy SLIM using the naive deployment strategy:


1. Set up the Kubernetes cluster.

    ```bash
    task cluster:up
    ```

1. Deploy SLIM using the naive strategy.

    ```bash
    task slim:deploy
    ```

1. Verify the deployment.

    ```bash
    kubectl get pods -n slim
    ```

1. View SLIM logs.

    ```bash
    task slim:show-logs
    ```

1. Start the receiver application pod.

    ```bash
    task test:receiver:deploy
    ```

1. Start the sender application pod.

    ```bash
    task test:sender:deploy
    ```

1. Clean up when done.

    ```bash
    task cluster:down
    ```

> **Note:** The naive strategy uses the `naive-values.yaml` file for Helm chart configuration. You can customize this file to adjust deployment parameters as needed.