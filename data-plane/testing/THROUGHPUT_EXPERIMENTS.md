# SLIM message throughput experiments

This document describes a repeatable protocol for measuring end-to-end message throughput through a single SLIM data-plane node **while telemetry is enabled**, and how to record **processed messages per second** using Prometheus and Grafana.

Procedures below use **[Task](https://taskfile.dev/)** wherever the repo already defines tasks; only Prometheus/Grafana UI steps and PromQL remain outside Task.

## Purpose and scope

- **Goal:** Characterize how workload size (number of publication definitions in the generated workload file) relates to steady-state **dataplane message processing rate** on the server, with OpenTelemetry metrics exported to the bundled observability stack.
- **In scope:** One SLIM server, one subscriber client, one publisher client, workload file–driven traffic, default telemetry compose stack.
- **Out of scope:** Multi-node clusters, many OS processes each running `publisher`, or automated CI benchmarking (unless you add that separately).

## Terminology

Throughout this doc, **“publications”** means the `workload_gen` parameter **`-p` / `--publications`**: the number of `PUB` lines written to the workload file. That matches the informal “more publishers” scaling axis when you want a larger publication catalog—not the number of separate `publisher` OS processes.

Running **N separate `publisher` binaries** (each with a different `--id`) is a different experiment and is not what `-p` controls.

## Prerequisites

- **Docker** and **Docker Compose** (used under the hood by telemetry tasks).
- **Task** installed.
- Rust workspace builds successfully when Task runs `data-plane:build` (triggered by `data-plane:run:server-with-otel`).

### Where to run which tasks

| Task namespace | Working directory | Taskfile |
|----------------|-------------------|----------|
| `data-plane:*` | Repository **root** *or* [`data-plane/`](../) | [`data-plane/Taskfile.yaml`](../Taskfile.yaml) is included from the root [`Taskfile.yaml`](../../Taskfile.yaml) with `dir: ./data-plane`, so `task data-plane:…` works from the repo root. You can also `cd data-plane` and run the same task names. |
| `benchmark:*` | [`data-plane/testing/`](.) | [`Taskfile.yml`](Taskfile.yml) (benchmark helpers; tasks set `dir: ".."` so artifacts land under `data-plane/`). |

Replace `<P>` below with `1000`, `10000`, `100000`, or `10000000` (see caveat for 10M).

## Fixed parameters (recommended)

Hold these constant across runs so differences reflect **`-p` only**:

| Parameter | Value in `benchmark:workload-gen` | Notes |
|-----------|-----------------------------------|--------|
| Subscriptions `-s` | `1000` (fixed in task) | Must be large enough to generate diverse routes. |
| Instances per type `-i` | `3` | See `workload_gen --help` to change in [`Taskfile.yml`](Taskfile.yml). |
| Apps `-a` | `3` | **≥ 2** so `--id 0` / `--id 1` on subscribe/publish clients exist in the workload. |
| Publisher `msg-size` / `frequency` | `1024` / `0` | Set in `benchmark:publish` in [`Taskfile.yml`](Taskfile.yml). |
| Server | Telemetry config | `data-plane:run:server-with-otel` → [`config/telemetry/server-config.yaml`](../config/telemetry/server-config.yaml). |

**Publication count matrix:** `1_000`, `10_000`, `100_000`, `10_000_000` → pass as **`P=`** on benchmark tasks.

**Practical note on 10M publications:** The workload file grows roughly with `P` (plus fixed `SUB` lines). Ten million `PUB` rows implies a **very large** `.dat` file, long generation time, and significant RAM/parse time in the clients. Validate disk space and runtime on a smaller `P` first; treat `10_000_000` as an optional stress tier.

**Custom workload shape:** If you need different `-s`, `-i`, `-a`, or output paths than [`benchmark:workload-gen`](Taskfile.yml), edit that task or add another Task target; the rest of this doc still applies.

## Experiment procedure

Each **experiment run** is the same pipeline; only **`P`** changes.

### 1. Generate the workload

From **`data-plane/testing/`**:

```bash
task benchmark:workload-gen P=<P>
```

This writes **`../workload_<P>.dat`** (i.e. `data-plane/workload_<P>.dat`). Default `P` is `2000000` if omitted.

### 2. Start the telemetry stack

From **repository root** or **`data-plane/`**:

```bash
task data-plane:telemetry:start:detached
```

For interactive logs (foreground), use:

```bash
task data-plane:telemetry:start
```

Both use [`config/telemetry/docker-compose.yml`](../config/telemetry/docker-compose.yml): Jaeger, OpenTelemetry Collector, Prometheus, Grafana.

**Ports (host):**

| Service | Port | Role |
|---------|------|------|
| OTLP gRPC | `4317` | SLIM sends traces/metrics here |
| Prometheus UI | `9090` | Query and graph metrics |
| Grafana | `3000` | Dashboards (default admin user/password in compose file) |
| Jaeger UI | `16686` | Traces (optional for throughput) |

### 3. Start SLIM with telemetry enabled

From **repository root** or **`data-plane/`**:

```bash
task data-plane:run:server-with-otel
```

This builds (via Task) and runs `slim` with **`tracing.opentelemetry.enabled: true`**. The default OTLP gRPC endpoint in code is `http://localhost:4317`, which matches the published collector port when the stack runs on the same host.

### 4. Start the subscriber, then the publisher

From **`data-plane/testing/`**:

```bash
task benchmark:subscribe P=<P>
task benchmark:publish P=<P>
```

Use the **same `P`** as in step 1 so both load **`workload_<P>.dat`**. To use another file path:

```bash
task benchmark:subscribe WORKLOAD=my_workload.dat
task benchmark:publish WORKLOAD=my_workload.dat
```

- **`benchmark:subscribe`** → `subscriber` with `--id 0` and the resolved `--workload`.
- **`benchmark:publish`** → `publisher` with `--id 1`, same workload, `--msg-size 1024`, `--frequency 0`.

**Order:** subscribe first, then publish.

### 5. Steady state and timing window

The publisher sleeps briefly after setup (see the `publisher` binary). For metrics, wait until traffic is flowing and Prometheus shows a non-flat **rate** on the processed-message counter, then choose a **fixed time window** (e.g. 2–5 minutes) and record wall-clock duration for the results table.

Stop clients and server when the run is complete. Leave the telemetry stack running if you want to retain Prometheus TSDB history. When finished, from **repository root** or **`data-plane/`**:

```bash
task data-plane:telemetry:stop
```

## Collecting results: Prometheus and Grafana

### Metric path

1. SLIM increments a tracing-backed counter for each handled message in the dataplane (`monotonic_counter.num_processed_messages` in [`message_processing.rs`](../core/datapath/src/message_processing.rs)).
2. The application exports OTLP **metrics** to the collector.
3. The collector’s **metrics** pipeline exposes a Prometheus scrape endpoint ([`otel-collector-config.yml`](../config/telemetry/otel-collector-config.yml), `namespace: slim`).
4. Prometheus scrapes the collector ([`prometheus.yml`](../config/telemetry/prometheus.yml) job `otel-collector`).

Exact **Prometheus series names** can vary with OpenTelemetry Collector and exporter versions. Do not rely on a hard-coded name without checking.

### Finding the counter in Prometheus

1. Open **Prometheus**: [http://localhost:9090](http://localhost:9090).
2. Go to **Graph** → use **Metrics explorer** (or Enter metric autocomplete).
3. Search for substrings such as `processed`, `num_processed`, or `slim_` until you identify the **counter** that tracks processed messages (monotonic, cumulative).

### PromQL: processed messages per second

For a counter `METRIC` with optional labels:

```promql
rate(METRIC[1m])
```

Use a range window that matches your scrape interval and noise tolerance (e.g. `30s`, `1m`, `5m`). The result is an **instantaneous rate** in messages per second (per series).

If multiple time series exist (e.g. per label), sum them:

```promql
sum(rate(METRIC[1m]))
```

**Integral / average over an interval:** You can also record the counter value at **start** and **end** of the run from Prometheus or Grafana and compute:

\[
\text{mean msg/s} \approx \frac{\text{counter}(t_2) - \text{counter}(t_1)}{t_2 - t_1}
\]

This is useful when `rate()` is noisy at low traffic.

### Grafana

1. Open **Grafana**: [http://localhost:3000](http://localhost:3000) (see compose for default admin credentials).
2. A Prometheus datasource is provisioned at [`config/telemetry/grafana/provisioning/datasources/prometheus.yml`](../config/telemetry/grafana/provisioning/datasources/prometheus.yml).
3. Create a panel with the same PromQL as above; use **Stat** or **Time series** to read **mean** and **max** over your test window.

Export or screenshot panels if you attach evidence to an internal report.

## Results

Fill this table after each run. **Mean processed msg/s** should reflect the **SLIM server** counter (dataplane processed messages), not client-side counts, unless you explicitly document a client metric.

| Run ID | Git commit (short) | Publications `P` | Subscriptions `-s` (task) | Msg size (B) | Test duration (s) | Mean processed msg/s | Peak processed msg/s | Notes |
|--------|-------------------|------------------|---------------------------|--------------|-------------------|----------------------|----------------------|-------|
| R001 | — | 1_000 | 1_000 | 1024 | — | — | — | TBD |
| R002 | — | 10_000 | 1_000 | 1024 | — | — | — | TBD |
| R003 | — | 100_000 | 1_000 | 1024 | — | — | — | TBD |
| R004 | — | 10_000_000 | 1_000 | 1024 | — | — | — | TBD (optional / stress) |

**Column definitions**

- **Mean processed msg/s:** Average of `sum(rate(...))` over the steady-state window, or \((C_{end}-C_{start})/\Delta t\) from the same counter.
- **Peak processed msg/s:** Maximum of the rate series over the window (or short rolling max if you use a recording rule).
- **Notes:** Hardware (CPU/RAM), OS, container image digests (see checklist), anomalies, or `num_message_process_errors` if you locate that counter.

## Reproducibility checklist

- Record **git commit** and **Rust toolchain** version (`rustc -V`).
- Record **host OS**, **CPU model**, and **RAM**.
- Telemetry images in compose may use floating tags. For strict comparisons, record image digests (e.g. `docker image inspect <image> --format '{{json .RepoDigests}}'`) and paste into **Notes**.
- Keep the same **Task parameters** and server config across runs; only change **`P`** (and matching `WORKLOAD` if you override).

## Reference: Task-only runbook

Set `<P>` once and run in order.

```bash
# 1) Workload (from data-plane/testing/)
cd data-plane/testing
task benchmark:workload-gen P=<P>

# 2) Telemetry (from repo root or data-plane/)
cd ../..
task data-plane:telemetry:start:detached

# 3) SLIM + OTEL (from repo root or data-plane/)
task data-plane:run:server-with-otel

# 4) Load (from data-plane/testing/ — new terminal while server runs)
cd data-plane/testing
task benchmark:subscribe P=<P>
task benchmark:publish P=<P>
```

Example with `P=10000`:

```bash
cd data-plane/testing && task benchmark:workload-gen P=10000
cd ../.. && task data-plane:telemetry:start:detached
task data-plane:run:server-with-otel
# another terminal:
cd data-plane/testing && task benchmark:subscribe P=10000 && task benchmark:publish P=10000
```
