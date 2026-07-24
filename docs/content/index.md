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
    <div class="slim-hero-tagline-rotate" data-slim-hero-tagline>
      <p class="slim-hero__tagline slim-hero-tagline-rotate__text">
        The secure transport layer for A2A, MCP, and SLIMRPC.
      </p>
    </div>
    <p class="slim-hero__lede">
      SLIM is built for interactive agent workloads: low-latency, end-to-end encrypted message
      delivery between agents that may be co-located or continents apart. Where protocols define
      conversation semantics, SLIM provides the data plane that makes those conversations work in
      production.
    </p>
    <div class="slim-hero__actions">
      <div class="slim-hero__actions-main">
        <a class="slim-hero__btn" href="#see-slim-in-action">
          See SLIM in Action
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 4l-1.41 1.41L16.17 11H4v2h12.17l-5.58 5.59L12 20l8-8z"/></svg>
        </a>
        <a class="slim-hero__btn" href="#community">
          Community
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

<div class="slim-page-body">

<section class="slim-why">
  <h2 class="slim-section-title">Why Use SLIM</h2>
  <div class="slim-features">
    <div class="slim-feature-card">
      <div class="slim-feature-card__art">
        <img
          src="assets/slim-why-icons/send.svg"
          alt=""
          width="22"
          height="22"
          loading="lazy"
        />
      </div>
      <p class="slim-feature-card__title">Transport for Agent Protocols</p>
      <p class="slim-feature-card__text">
        Carry A2A, MCP, OpenTelemetry, and custom agent protocols over a single secure
        overlay instead of bespoke per-protocol networking.
      </p>
    </div>
    <div class="slim-feature-card">
      <div class="slim-feature-card__art">
        <img
          src="assets/slim-why-icons/lock.svg"
          alt=""
          width="22"
          height="22"
          loading="lazy"
        />
      </div>
      <p class="slim-feature-card__title">End-to-End Encryption</p>
      <p class="slim-feature-card__text">
        Message Layer Security (MLS) keeps payloads encrypted from source to
        destination, even when traffic traverses shared routing infrastructure.
      </p>
    </div>
    <div class="slim-feature-card">
      <div class="slim-feature-card__art">
        <img
          src="assets/slim-why-icons/network.svg"
          alt=""
          width="22"
          height="22"
          loading="lazy"
        />
      </div>
      <p class="slim-feature-card__title">Network-Topology Independence</p>
      <p class="slim-feature-card__text">
        Agents connect outbound to SLIM nodes—reachable by name, without exposing every
        agent to the internet.
      </p>
    </div>
  </div>
</section>

<section class="slim-howto">
  <h2 class="slim-section-title">How SLIM Routes Messages</h2>
  <div class="slim-graph-wrap">
    <iframe
      class="slim-graph-frame"
      src="slim-graph/index.html"
      aria-label="Interactive SLIM message routing diagram"
      tabindex="-1"
      scrolling="no"
    ></iframe>
  </div>
</section>

<section class="slim-quickstart">
  <h2 class="slim-section-title" id="see-slim-in-action">See SLIM in Action</h2>

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
        <div class="slimctl-terminal-intro-group" id="slimctl-terminal-intros">
          <p class="slimctl-terminal-intro" data-intro-level="node" hidden>
            Start a local node using the slimctl CLI. See the
            <a href="slim/slim-howto/">Getting Started guide</a> for a full walkthrough,
            or the <a href="slim/deploy/index.html">Deployment docs</a> for Docker,
            Kubernetes, and multi-cluster options.
          </p>
          <p class="slimctl-terminal-intro" data-intro-level="message" hidden>
            Use the SLIM SDK to connect your app and exchange messages.
            See the <a href="slim/components/sdk/index.html">SLIM SDK overview</a>
            and the <a href="slim/components/sdk/tutorials/tutorial-connect.html">Tutorials</a>
            to get up and running in Python, Go, Java, Kotlin, .NET, or Node.js.
          </p>
        </div>
        <div class="slimctl-terminal-actions">
          <button type="button" class="slimctl-terminal-btn is-active" data-demo-level="node">Start Node</button>
          <button type="button" class="slimctl-terminal-btn" data-demo-level="message">Send a Message</button>
        </div>
      </div>
    </div>
  </section>
</section>

<section class="slim-community" id="community">
  <h2 class="slim-section-title">Community</h2>
  <p class="slim-community__lede">
    Connect with AGNTCY contributors, join working group meetings, and help shape secure agent messaging.
    For more information, see the <a href="community.md">community page</a>.
  </p>

  <div class="slim-community-social">
    <a
      class="slim-community-card"
      href="https://discord.gg/FbEnSHXD34"
      target="_blank"
      rel="noopener noreferrer"
    >
      <span class="slim-community-card__icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" role="img"><path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037 12.3 12.3 0 0 0-.608 1.25 18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028 14.09 14.09 0 0 0 1.226-1.994.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.955-2.419 2.157-2.419 1.21 0 2.176 1.096 2.157 2.42 0 1.333-.946 2.418-2.157 2.418z"/></svg>
      </span>
      <span class="slim-community-card__body">
        <span class="slim-community-card__title">Discord</span>
        <span class="slim-community-card__text">Chat with maintainers and contributors in the AGNTCY Discord server.</span>
      </span>
    </a>

    <a
      class="slim-community-card"
      href="https://zoom-lfx.platform.linuxfoundation.org/meetings/agntcy?view=week"
      target="_blank"
      rel="noopener noreferrer"
    >
      <span class="slim-community-card__icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" role="img"><path d="M19 4h-1V2h-2v2H8V2H6v2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V6a2 2 0 0 0-2-2zm0 16H5V10h14v10zM5 8V6h14v2H5zm2 4h10v2H7v-2zm0 4h7v2H7v-2z"/></svg>
      </span>
      <span class="slim-community-card__body">
        <span class="slim-community-card__title">Meetings</span>
        <span class="slim-community-card__text">Join working group and community meetings on the AGNTCY calendar.</span>
      </span>
    </a>

    <a
      class="slim-community-card"
      href="https://blogs.agntcy.org/"
      target="_blank"
      rel="noopener noreferrer"
    >
      <span class="slim-community-card__icon" aria-hidden="true">
        <svg viewBox="0 0 24 24" role="img"><path d="M19 3H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2zm-5 14H7v-2h7v2zm3-4H7v-2h10v2zm0-4H7V7h10v2z"/></svg>
      </span>
      <span class="slim-community-card__body">
        <span class="slim-community-card__title">Blog</span>
        <span class="slim-community-card__text">Read announcements, tutorials, and technical deep dives from the SLIM team.</span>
      </span>
    </a>
  </div>

  <div class="slim-community-contribute">
    <div class="slim-community-contribute__text">
      <h3 class="slim-community-contribute__title">Contribute</h3>
      <p>
        Help build SLIM by contributing code, reporting bugs, or suggesting enhancements. Pick up a
        good first issue, review open pull requests, or read the contributing guide to get started.
      </p>
      <div class="slim-community-contribute__actions">
        <a
          class="slim-community-contribute__btn"
          href="https://github.com/agntcy/slim"
          target="_blank"
          rel="noopener noreferrer"
        >
          Visit our GitHub
          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M12 .5C5.73.5.5 5.73.5 12c0 5.08 3.29 9.39 7.86 10.91.58.11.79-.25.79-.56 0-.28-.01-1.02-.02-2-3.2.7-3.88-1.54-3.88-1.54-.53-1.34-1.29-1.7-1.29-1.7-1.05-.72.08-.71.08-.71 1.16.08 1.77 1.19 1.77 1.19 1.03 1.77 2.7 1.26 3.36.96.1-.75.4-1.26.73-1.55-2.55-.29-5.23-1.28-5.23-5.69 0-1.26.45-2.29 1.19-3.1-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.18 1.18a11.1 11.1 0 0 1 5.8 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.84 1.19 3.1 0 4.42-2.69 5.39-5.25 5.68.41.36.78 1.06.78 2.14 0 1.55-.01 2.8-.01 3.18 0 .31.21.68.8.56A11.51 11.51 0 0 0 23.5 12C23.5 5.73 18.27.5 12 .5z"/></svg>
        </a>
        <a
          class="slim-community-contribute__btn slim-community-contribute__btn--highlight"
          href="https://github.com/search?q=repo%3Aagntcy%2Fslim+type%3Aissue+label%3A%22good-first-issue%22%2C%22good+first+issue%22&type=issues"
          target="_blank"
          rel="noopener noreferrer"
        >
          Good first issues
        </a>
      </div>
    </div>
    <div class="slim-community-contribute__metrics">
      <a
        class="slim-repobeats__link"
        href="https://github.com/agntcy/slim/pulse?period=monthly"
        target="_blank"
        rel="noopener noreferrer"
        data-slim-repobeats
        data-repo="agntcy/slim"
      >
        <div class="slim-repobeats" aria-busy="true" aria-label="GitHub repository metrics">
          <p class="slim-repobeats__loading">Loading repository metrics…</p>
        </div>
      </a>
    </div>
  </div>
</section>

</div>

<section class="slim-get-started" markdown="1">

## Explore SLIM {.slim-section-title}

<div class="grid cards" markdown="1">

- :material-book-open:{ .lg .middle } **Introduction**

    Understand core concepts, the messaging layer, and the controller.

    [:octicons-arrow-right-24: Overview](slim/slim-overview.md)

- :material-rocket-launch:{ .lg .middle } **Get Started**

    Run a SLIM node and send your first messages in minutes.

    [:octicons-arrow-right-24: Getting Started](slim/slim-howto.md)

- :material-layers-triple:{ .lg .middle } **Architecture**

    Sessions, groups, naming, routing, and end-to-end encryption.

    [:octicons-arrow-right-24: Architecture](slim/architecture/index.md)

- :material-code-braces:{ .lg .middle } **SDK & Tutorials**

    Connect, build apps, create sessions, and call SLIMRPC services.

    [:octicons-arrow-right-24: SLIM SDK](slim/components/sdk/index.md)

- :material-connection:{ .lg .middle } **Integrations**

    Run A2A, MCP, SLIMRPC, and OpenTelemetry over SLIM.

    [:octicons-arrow-right-24: Integrations](slim/integrations/index.md)

- :material-cloud-upload:{ .lg .middle } **Deployment**

    Run SLIM locally, in Docker, on Kubernetes, or across multiple clusters.

    [:octicons-arrow-right-24: Deployment](slim/deploy/index.md)

- :material-file-document-outline:{ .lg .middle } **Specification**

    Explore the SLIM Internet Draft and protocol definition.

    [:octicons-arrow-right-24: SLIM Specification](https://datatracker.ietf.org/doc/draft-mpsb-agntcy-slim/)

- :material-account-group:{ .lg .middle } **Community**

    Connect with maintainers and contributors across the AGNTCY project.

    [:octicons-arrow-right-24: SLIM Community](community.md#repositories)

</div>

</section>
