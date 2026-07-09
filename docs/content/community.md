---
hide:

  - navigation
  - toc

---

# SLIM Community

Contributors to **Secure Low-Latency Interactive Messaging (SLIM)** work in the
open on the specification, the Rust data plane, the control plane, the language
bindings, and the protocol integrations. If you run SLIM nodes, build agents on
top of the bindings, or integrate SLIM transport into your stack, the links
below are where the work happens—repos, Discord, meetings, and recent posts from
the team.

SLIM is developed within
[AGNTCY](https://github.com/agntcy), alongside DIR, OASF, and other
infrastructure for open, vendor-neutral agent communication.

---

## Connect with us

Stay involved with the AGNTCY community and SLIM contributors:

- **[AGNTCY Discord server](https://discord.gg/nbeEENusy)** — chat with maintainers and contributors
- **[AGNTCY Meeting Calendar](https://zoom-lfx.platform.linuxfoundation.org/meetings/agntcy?view=week)** — working group and community meetings
- **[AGNTCY on GitHub](https://github.com/agntcy)** — organization home for all IoA projects
- **[AGNTCY Blogs](https://blogs.agntcy.org/)** — announcements, tutorials, and project updates

---

## Recent News & Blog Posts

Stay up to date with announcements, tutorials, and technical deep dives from the SLIM team and community:

- **[Now You're Thinking with Agents](https://blogs.agntcy.org/technical/agents/a2a/2026/06/08/thinking-with-agents.html)** — *June 8, 2026*
- **[SLIM MVP: Multicluster Customer Remediation with AI Agents](https://blogs.agntcy.org/technical/2026/04/21/mvp-ai-agents-multicluster.html)** — *April 21, 2026*
- **[SLIM for Observability and Remediation: Beyond Agentic AI](https://blogs.agntcy.org/technical/2026/04/02/transporting-opentelemetry-over-slim.html)** — *April 2, 2026*
- **[SlimRPC Multicast: One Call, Every Agent](https://blogs.agntcy.org/technical/slim/agents/2026/03/31/slimrpc-multicast.html)** — *March 31, 2026*
- **[Announcing slim-a2a-go 0.1.0: Native A2A over SLIM for Go](https://blogs.agntcy.org/agents/go/slim/a2a/announcements/2026/03/24/slim-a2a-go-release.html)** — *March 24, 2026*
- **[Write Once, Run Everywhere: Why Rust + UniFFI is the Future of Multi-Language Libraries](https://blogs.agntcy.org/technical/2026/03/13/rust-uniffi-multilanguage-strategy.html)** — *March 13, 2026*
- **[SLIM v1.0: Secure Low-Latency Interactive Messaging for Agentic AI](https://blogs.agntcy.org/technical/2026/02/03/slim-v1.html)** — *February 3, 2026*
- **[Integrating Rust and Go with UniFFI: A Complete Guide](https://blogs.agntcy.org/technical/2026/01/27/integrating-rust-and-go-with-uniffi.html)** — *January 27, 2026*
- **[Distributing C Artifacts for Go Modules: A Practical Approach](https://blogs.agntcy.org/technical/2026/01/20/distributing-c-artifacts-for-go-modules.html)** — *January 20, 2026*

---

## Repositories

<div class="grid cards" markdown>

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim](https://github.com/agntcy/slim)**

    Reference implementation — Rust data plane, control plane, and core.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-spec](https://github.com/agntcy/slim-spec)**

    SLIM protocol specification.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-bindings](https://github.com/agntcy/slim-bindings)**

    SLIM language bindings.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-bindings-go](https://github.com/agntcy/slim-bindings-go)**

    Go language bindings — `go get github.com/agntcy/slim-bindings-go`.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-mcp-python](https://github.com/agntcy/slim-mcp-python)**

    SLIM integrations for the MCP Python SDK.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-mcp-rust](https://github.com/agntcy/slim-mcp-rust)**

    SLIM integrations for MCP in Rust, plus the SLIM MCP Proxy.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-a2a-python](https://github.com/agntcy/slim-a2a-python)**

    SLIMRPC transport for the a2a-python SDK.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-a2a-go](https://github.com/agntcy/slim-a2a-go)**

    SLIMRPC transport for the a2a-go SDK.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-a2a-java](https://github.com/agntcy/slim-a2a-java)**

    SLIMRPC transport for the a2a-java SDK.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-a2a-dotnet](https://github.com/agntcy/slim-a2a-dotnet)**

    SLIMRPC transport for the a2a-dotnet SDK.

- :fontawesome-brands-github:{ .lg .middle } **[agntcy/slim-otel](https://github.com/agntcy/slim-otel)**

    SLIM components for OpenTelemetry.

</div>

---

## Contributing

We welcome contributions across the SLIM ecosystem and the broader AGNTCY project:

- Pick up a ["good first issue"](https://github.com/search?q=org%3Aagntcy+type%3Aissue+label%3A%22good-first-issue%22%2C%22good+first+issue%22&type=issues)
  across AGNTCY repositories
- Report bugs or suggest enhancements on [agntcy/slim Issues](https://github.com/agntcy/slim/issues)
- Review open pull requests on [agntcy/slim](https://github.com/agntcy/slim/pulls)

Read the AGNTCY [Contributing Guide](https://github.com/agntcy/governance/blob/main/CONTRIBUTING.md),
[Governance](https://github.com/agntcy/governance) repository, and
[Code of Conduct](https://github.com/agntcy/governance/blob/main/CODE_OF_CONDUCT.md).

---

## Related AGNTCY components

SLIM works alongside other AGNTCY building blocks:

- **[DIR](https://docs.agntcy.org/dir/overview/)** — Agent Directory Service for open agent discovery
- **[OASF](https://docs.agntcy.org/oasf/open-agentic-schema-framework/)** — Open Agentic Schema Framework for agent metadata
