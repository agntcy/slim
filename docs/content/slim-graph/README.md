# SLIM Architecture Visualizer

Interactive SVG diagram embedded on the SLIM documentation home page. It demonstrates
point-to-point A2A messaging, multicast A2A channels, and cloud-to-on-prem MCP tool
invocation across local and cloud SLIM data plane nodes.

## Run locally

From the repository root, start the docs site:

```bash
task docs:run
```

Open [http://127.0.0.1:8000/](http://127.0.0.1:8000/) and scroll to **How SLIM Routes Messages**.

To preview the visualizer standalone, open `docs/content/slim-graph/index.html` in a
browser or serve the `docs/content` directory with any static file server.

## Structure

- `index.html` — diagram layout and journey controls
- `js/config.js` — node metadata, journey labels, and scenario steps
- `js/app.js` — playback, stepper, and tooltip behavior
- `css/styles.css` — diagram styling
