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

## Explore SLIM

<div class="landing-explore">

<div class="landing-explore__item">
  <button type="button" class="landing-explore__toggle" aria-expanded="false">
    <span class="landing-explore__title"><p><span class="twemoji lg middle"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 15.5A3.5 3.5 0 0 1 8.5 12 3.5 3.5 0 0 1 12 8.5a3.5 3.5 0 0 1 3.5 3.5 3.5 3.5 0 0 1-3.5 3.5m7.43-2.53c.04-.32.07-.64.07-.97s-.03-.66-.07-1l2.11-1.63c.19-.15.24-.42.12-.64l-2-3.46c-.12-.22-.39-.31-.61-.22l-2.49 1c-.52-.39-1.06-.73-1.69-.98l-.37-2.65A.506.506 0 0 0 14 2h-4c-.25 0-.46.18-.5.42l-.37 2.65c-.63.25-1.17.59-1.69.98l-2.49-1c-.22-.09-.49 0-.61.22l-2 3.46c-.13.22-.07.49.12.64L4.57 11c-.04.34-.07.67-.07 1s.03.65.07.97l-2.11 1.66c-.19.15-.25.42-.12.64l2 3.46c.12.22.39.3.61.22l2.49-1.01c.52.4 1.06.74 1.69.99l.37 2.65c.04.24.25.42.5.42h4c.25 0 .46-.18.5-.42l.37-2.65c.63-.26 1.17-.59 1.69-.99l2.49 1.01c.22.08.49 0 .61-.22l2-3.46c.12-.22.07-.49-.12-.64z"/></svg></span><span class="landing-explore__label"><strong>SLIM Controller</strong><span class="landing-explore__tagline">Orchestrate nodes across distributed networks</span></span></p></span>
    <span class="landing-explore__chevron" aria-hidden="true"></span>
  </button>
  <div class="landing-explore__body" hidden>
    <p>The SLIM Controller orchestrates data-plane nodes in a distributed messaging system. It configures routes, manages node registration, and coordinates communication through northbound and southbound interfaces.</p>
    <p><a href="slim/slim-controller/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> SLIM Controller documentation</a></p>
  </div>
</div>

<div class="landing-explore__item">
  <button type="button" class="landing-explore__toggle" aria-expanded="false">
    <span class="landing-explore__title"><p><span class="twemoji lg middle"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M18 11h-3.18C14.4 9.84 13.3 9 12 9s-2.4.84-2.82 2H6c-.33 0-2-.1-2-2V8c0-1.83 1.54-2 2-2h10.18C16.6 7.16 17.7 8 19 8a3 3 0 0 0 3-3 3 3 0 0 0-3-3c-1.3 0-2.4.84-2.82 2H6C4.39 4 2 5.06 2 8v1c0 2.94 2.39 4 4 4h3.18c.42 1.16 1.52 2 2.82 2s2.4-.84 2.82-2H18c.33 0 2 .1 2 2v1c0 1.83-1.54 2-2 2H7.82C7.4 16.84 6.3 16 5 16a3 3 0 0 0-3 3 3 3 0 0 0 3 3c1.3 0 2.4-.84 2.82-2H18c1.61 0 4-1.07 4-4v-1c0-2.93-2.39-4-4-4m1-7a1 1 0 0 1 1 1 1 1 0 0 1-1 1 1 1 0 0 1-1-1 1 1 0 0 1 1-1M5 20a1 1 0 0 1-1-1 1 1 0 0 1 1-1 1 1 0 0 1 1 1 1 1 0 0 1-1 1"/></svg></span><span class="landing-explore__label"><strong>SLIM Messaging Layer</strong><span class="landing-explore__tagline">Route messages between named endpoints</span></span></p></span>
    <span class="landing-explore__chevron" aria-hidden="true"></span>
  </button>
  <div class="landing-explore__body" hidden>
    <p>The SLIM Messaging Layer routes and delivers messages between applications using hierarchical client and channel names. It supports anycast and unicast delivery, sessions, and optional end-to-end encryption with MLS.</p>
    <p><a href="slim/slim-data-plane/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> Messaging Layer documentation</a></p>
  </div>
</div>

<div class="landing-explore__item">
  <button type="button" class="landing-explore__toggle" aria-expanded="false">
    <span class="landing-explore__title"><p><span class="twemoji lg middle"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 5.5A3.5 3.5 0 0 1 15.5 9a3.5 3.5 0 0 1-3.5 3.5A3.5 3.5 0 0 1 8.5 9 3.5 3.5 0 0 1 12 5.5M5 8c.56 0 1.08.15 1.53.42-.15 1.43.27 2.85 1.13 3.96C7.16 13.34 6.16 14 5 14a3 3 0 0 1-3-3 3 3 0 0 1 3-3m14 0a3 3 0 0 1 3 3 3 3 0 0 1-3 3c-1.16 0-2.16-.66-2.66-1.62a5.54 5.54 0 0 0 1.13-3.96c.45-.27.97-.42 1.53-.42M5.5 18.25c0-2.07 2.91-3.75 6.5-3.75s6.5 1.68 6.5 3.75V20h-13zM0 20v-1.5c0-1.39 1.89-2.56 4.45-2.9-.59.68-.95 1.62-.95 2.65V20zm24 0h-3.5v-1.75c0-1.03-.36-1.97-.95-2.65 2.56.34 4.45 1.51 4.45 2.9z"/></svg></span><span class="landing-explore__label"><strong>SLIM Groups</strong><span class="landing-explore__tagline">Create shared encrypted group sessions</span></span></p></span>
    <span class="landing-explore__chevron" aria-hidden="true"></span>
  </button>
  <div class="landing-explore__body" hidden>
    <p>SLIM groups let multiple clients communicate over a shared encrypted channel. Create and manage group sessions, invite participants, and run multi-party agent workflows with MLS-backed confidentiality.</p>
    <p><a href="slim/slim-group/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> SLIM Groups documentation</a></p>
  </div>
</div>

<div class="landing-explore__item">
  <button type="button" class="landing-explore__toggle" aria-expanded="false">
    <span class="landing-explore__title"><p><span class="twemoji lg middle"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M21.4 7.5c.8.8.8 2.1 0 2.8l-2.8 2.8-7.8-7.8 2.8-2.8c.8-.8 2.1-.8 2.8 0l1.8 1.8 3-3 1.4 1.4-3 3zm-5.8 5.8-1.4-1.4-2.8 2.8-2.1-2.1 2.8-2.8-1.4-1.4-2.8 2.8-1.5-1.4-2.8 2.8c-.8.8-.8 2.1 0 2.8l1.8 1.8-4 4 1.4 1.4 4-4 1.8 1.8c.8.8 2.1.8 2.8 0l2.8-2.8-1.4-1.4z"/></svg></span><span class="landing-explore__label"><strong>Integrations</strong><span class="landing-explore__tagline">Connect agents using common protocols</span></span></p></span>
    <span class="landing-explore__chevron" aria-hidden="true"></span>
  </button>
  <div class="landing-explore__body" hidden>
    <p>Run common agent and observability protocols over the same secure SLIM overlay—without bespoke networking per protocol.</p>
    <p><a href="slim/slim-rpc/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> SLIMRPC</a></p>
    <p><a href="slim/slim-a2a/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> SLIM A2A</a></p>
    <p><a href="slim/slim-mcp/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> MCP over SLIM</a></p>
    <p><a href="slim/slim-otel/"><span class="twemoji"><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M13.22 19.03a.75.75 0 0 1 0-1.06L18.19 13H3.75a.75.75 0 0 1 0-1.5h14.44l-4.97-4.97a.749.749 0 0 1 .326-1.275.75.75 0 0 1 .734.215l6.25 6.25a.75.75 0 0 1 0 1.06l-6.25 6.25a.75.75 0 0 1-1.06 0"/></svg></span> OpenTelemetry over SLIM</a></p>
  </div>
</div>

</div>

## Get started with SLIM

<div class="grid cards" markdown>

- :material-rocket-launch:{ .lg .middle } **Get Started**

    Run a SLIM node and send your first messages in minutes.

    [:octicons-arrow-right-24: Getting Started](slim/slim-howto.md)

- :material-file-document-outline:{ .lg .middle } **Dive into the Specification**

    Explore the SLIM Internet Draft and protocol definition.

    [:octicons-arrow-right-24: SLIM Specification](https://datatracker.ietf.org/doc/draft-mpsb-agntcy-slim/)

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
