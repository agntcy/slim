---
hide:

  - navigation
  - toc

---

<div class="slim-landing">

<section class="slim-hero">
  <div class="slim-hero__inner">
    <h1 class="slim-hero__title">Secure Low-Latency Interactive Messaging</h1>
    <div class="slim-hero__partner">
      <span class="slim-hero__partner-text">part of</span>
      <a
        href="https://www.linuxfoundation.org/press/linux-foundation-welcomes-the-agntcy-project-to-standardize-open-multi-agent-system-infrastructure-and-break-down-ai-agent-silos"
        target="_blank"
        rel="noopener noreferrer"
      >
        <picture>
          <source
            media="(max-width: 59.9375em)"
            srcset="assets/lf-stacked-white.png"
          />
          <img
            src="assets/lf-horizontal-white.png"
            alt="Linux Foundation"
            class="slim-hero__partner-logo"
          />
        </picture>
      </a>
    </div>
    <p class="slim-hero__tagline">
      The secure transport layer for A2A, MCP, and SLIMRPC agent protocols.
    </p>
    <p class="slim-hero__lede">
      <strong>Secure Low-Latency Interactive Messaging (SLIM)</strong> is the secure transport layer
      for agent communication, an open source project under the
      <a href="https://www.linuxfoundation.org/press/linux-foundation-welcomes-the-agntcy-project-to-standardize-open-multi-agent-system-infrastructure-and-break-down-ai-agent-silos" target="_blank" rel="noopener noreferrer">Linux Foundation</a>
      building the Internet of Agents. While protocols like A2A define <em>what</em> agents say,
      SLIM defines <em>how</em> those messages are securely delivered across distributed
      networks—whether agents run in a data center, in a browser, on mobile devices, or
      across organizational boundaries.
    </p>
    <div class="slim-hero__actions">
      <div class="slim-hero__actions-main">
        <a class="slim-hero__btn" href="#quick-start">
          Quickstart
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z"/></svg>
        </a>
        <a class="slim-hero__btn" href="https://github.com/agntcy/slim" target="_blank" rel="noopener noreferrer">
          GitHub
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 .5C5.73.5.5 5.73.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.28-.01-1.02-.02-2-3.2.7-3.88-1.54-3.88-1.54-.53-1.34-1.29-1.7-1.29-1.7-1.05-.72.08-.71.08-.71 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.23-1.28-5.23-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.18 1.18a11.1 11.1 0 0 1 5.8 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.84 1.19 3.1 0 4.42-2.69 5.39-5.25 5.68.41.36.78 1.06.78 2.14 0 1.55-.01 2.8-.01 3.18 0 .31.21.68.8.56A11.51 11.51 0 0 0 23.5 12C23.5 5.73 18.27.5 12 .5z"/></svg>
        </a>
      </div>
    </div>
  </div>
</section>

</div>

<section class="slim-why">
  <h2 class="slim-section-title">Why use SLIM</h2>
  <div class="slim-why-cards">
    <article class="slim-why-card">
      <div class="slim-why-card__icon-wrap">
        <img
          src="assets/slim-why-icons/send.svg"
          alt=""
          width="24"
          height="24"
          class="slim-why-card__icon"
        />
      </div>
      <h3 class="slim-why-card__title">Transport for Agent Protocols</h3>
      <p class="slim-why-card__desc">
        Carry A2A, MCP, SLIMRPC, and custom agent protocols over a single secure
        overlay instead of bespoke per-protocol networking.
      </p>
    </article>
    <article class="slim-why-card">
      <div class="slim-why-card__icon-wrap">
        <img
          src="assets/slim-why-icons/lock.svg"
          alt=""
          width="24"
          height="24"
          class="slim-why-card__icon"
        />
      </div>
      <h3 class="slim-why-card__title">End-to-End Encryption</h3>
      <p class="slim-why-card__desc">
        Message Layer Security (MLS) keeps payloads encrypted from source to
        destination, even when traffic traverses shared routing infrastructure.
      </p>
    </article>
    <article class="slim-why-card">
      <div class="slim-why-card__icon-wrap">
        <img
          src="assets/slim-why-icons/network.svg"
          alt=""
          width="24"
          height="24"
          class="slim-why-card__icon"
        />
      </div>
      <h3 class="slim-why-card__title">Network-Topology Independence</h3>
      <p class="slim-why-card__desc">
        Agents connect outbound to SLIM nodes and are reachable by hierarchical name—no
        inbound ports, VPNs, or NAT traversal required.
      </p>
    </article>
  </div>
</section>

<div class="slim-landing">

<section class="slim-howto">
  <div class="slim-graph-wrap">
    <iframe
      class="slim-graph-frame"
      src="slim-graph/index.html"
      title="SLIM architecture visualizer"
      tabindex="-1"
    ></iframe>
  </div>
</section>

<section class="slim-quickstart">
  <h2 class="slim-section-title" id="quick-start">Quickstart</h2>

  <section class="slimctl-terminal-section">
    <div class="slimctl-terminal-layout">
      <div class="slimctl-terminal-main">
        <div class="slimctl-terminal" data-mode="demo">
          <div class="slimctl-terminal-bar">
            <span class="slimctl-terminal-title">user@slim:~</span>
            <div class="slimctl-terminal-controls" aria-hidden="true">
              <span class="slimctl-terminal-control">&#8211;</span>
              <span class="slimctl-terminal-control slimctl-terminal-control-close">&#10005;</span>
            </div>
          </div>
          <pre
            class="slimctl-terminal-output"
            id="slimctl-terminal-output"
            aria-label="Terminal output"
          ></pre>
          <form class="slimctl-terminal-input" hidden>
            <label
              class="slimctl-terminal-prompt"
              for="slimctl-terminal-command"
            >user@slim:~$</label>
            <input
              id="slimctl-terminal-command"
              class="slimctl-terminal-command"
              type="text"
              autocomplete="off"
              spellcheck="false"
              aria-label="Enter a slimctl command"
            />
          </form>
        </div>
      </div>
      <div class="slimctl-terminal-side">
        <div class="slimctl-terminal-intro-group" id="slimctl-terminal-intros" markdown="1">

Start a local node, inspect connections, and add routes with slimctl. See the [Quickstart guide](slim/slim-howto) for installation.
{: .slimctl-terminal-intro data-intro-level="node" hidden}

Agents subscribe to named channels, publish encrypted payloads, and SLIM routes them across nodes. Watch the diagram above for the full path.
{: .slimctl-terminal-intro data-intro-level="message"}

Use `slimctl --help` to explore commands. See the [SLIM Controller](slim/slim-controller) docs for the full CLI reference.
{: .slimctl-terminal-intro data-intro-level="try" hidden}

        </div>
        <div class="slimctl-terminal-actions">
          <button type="button" class="slimctl-terminal-btn is-active" data-demo-level="message">Send a message</button>
          <button type="button" class="slimctl-terminal-btn" data-demo-level="node">Start node</button>
          <button type="button" class="slimctl-terminal-btn" data-mode-switch="try">Try it yourself</button>
        </div>
      </div>
    </div>
  </section>
</section>

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
