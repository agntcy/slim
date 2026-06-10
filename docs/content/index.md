---
hide:

  - navigation
  - toc

---

<div class="landing-hero">
  <div class="centered-logo-text-group">
    <h1>Secure Low-Latency Interactive Messaging</h1>
  </div>
  <div class="lf-partner-badge">
    <span class="lf-partner-badge__text">part of</span>
    <a
      href="https://www.linuxfoundation.org/press/linux-foundation-welcomes-the-agntcy-project-to-standardize-open-multi-agent-system-infrastructure-and-break-down-ai-agent-silos"
      target="_blank"
      rel="noopener noreferrer"
    >
      <img
        src="assets/lf-horizontal-black.png"
        alt="Linux Foundation"
        class="logo-light lf-partner-badge__logo"
      />
      <img
        src="assets/lf-horizontal-white.png"
        alt="Linux Foundation"
        class="logo-dark lf-partner-badge__logo"
      />
    </a>
  </div>
</div>

## What is SLIM?

**Secure Low-Latency Interactive Messaging (SLIM)** is the secure transport layer
for agent communication, an open source project under the
[Linux Foundation](https://www.linuxfoundation.org/press/linux-foundation-welcomes-the-agntcy-project-to-standardize-open-multi-agent-system-infrastructure-and-break-down-ai-agent-silos)
building the Internet of Agents. While protocols like A2A define *what* agents say,
SLIM defines *how* those messages are securely delivered across distributed
networks—whether agents run in a data center, in a browser, on mobile devices, or
across organizational boundaries.

SLIM combines gRPC's performance over HTTP/2, native channel and group messaging,
end-to-end encryption with the Message Layer Security (MLS) protocol, native RPC
support (SLIMRPC), and a clean separation of control and data planes for
scalability—serving as the transport for A2A, MCP, and custom agent protocols.

## Why use SLIM

<div class="grid cards" markdown>

- :material-transit-connection-variant:{ .lg .middle } **Transport for Agent Protocols**

    Carry A2A, MCP, SLIMRPC, and custom agent protocols over a single secure
    overlay instead of bespoke per-protocol networking.

- :material-shield-lock:{ .lg .middle } **End-to-End Encryption**

    Message Layer Security (MLS) keeps payloads encrypted from source to
    destination, even when traffic traverses shared routing infrastructure.

- :material-lan-connect:{ .lg .middle } **Network-Topology Independence**

    Agents connect outbound to SLIM nodes and are reachable by hierarchical name—no
    inbound ports, VPNs, or NAT traversal required.

</div>

## Get started with SLIM

<div class="grid cards" markdown>

- :material-rocket-launch:{ .lg .middle } **Get Started**

    Run a SLIM node and send your first messages in minutes.

    [:octicons-arrow-right-24: Getting Started](slim/slim-howto.md)

- :material-book-open:{ .lg .middle } **Read the Introduction**

    Understand core concepts, the messaging layer, and the controller.

    [:octicons-arrow-right-24: Overview](slim/slim-overview.md)

- :material-file-document-outline:{ .lg .middle } **Dive into the Specification**

    Explore the SLIM Internet Draft and protocol definition.

    [:octicons-arrow-right-24: SLIM Specification](https://datatracker.ietf.org/doc/draft-mpsb-agntcy-slim/)

- :material-connection:{ .lg .middle } **Integrations**

    Run A2A, MCP, SLIMRPC, and OpenTelemetry over SLIM.

    [:octicons-arrow-right-24: Integrations](slim/slim-integrations.md)

- :material-cog:{ .lg .middle } **Configure**

    Tune the data plane for your environment.

    [:octicons-arrow-right-24: Configuration Reference](slim/slim-data-plane-config.md)

- :material-text-box-outline:{ .lg .middle } **Reference**

    Controller API and configuration details.

    [:octicons-arrow-right-24: Controller Reference](slim/slim-controller-reference.md)

- :fontawesome-brands-github:{ .lg .middle } **Source Code**

    Reference implementation and related repositories.

    [:octicons-arrow-right-24: github.com/agntcy/slim](https://github.com/agntcy/slim)

- :material-account-group:{ .lg .middle } **Join the Community**

    Connect with maintainers and contributors across the AGNTCY project.

    [:octicons-arrow-right-24: SLIM Community](community.md)

- :material-newspaper-variant-outline:{ .lg .middle } **Linux Foundation Press Release**

    Read how the Linux Foundation welcomed the AGNTCY project to standardize open
    multi-agent system infrastructure and break down AI agent silos.

    [:octicons-arrow-right-24: LF press release](https://www.linuxfoundation.org/press/linux-foundation-welcomes-the-agntcy-project-to-standardize-open-multi-agent-system-infrastructure-and-break-down-ai-agent-silos)

</div>
