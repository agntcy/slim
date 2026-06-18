# AGNTCY SLIM Architecture Visualizer (2D)

An interactive 2D SVG-based visualizer explaining the **Secure Low-Latency Interactive Messaging (SLIM)** architecture and protocols. It demonstrates a multi-node distributed routing network in a clean, lightweight pipeline:

- **Local & Cloud Data Planes (SLIM Nodes 1 & 2)**: Visualizes a multi-hop topology. Agent A connects locally to **SLIM Node 1 (Local)**, which bridges and peers with **SLIM Node 2 (Cloud)**. Destinations (Agents B, C, D) are connected directly to the cloud node.
- **Control Plane Orchestration**: Highlights the separation of control and data planes. Shows how the out-of-band **SLIM Controller** pushes table configurations and pings telemetry heartbeats to both SLIM Node 1 and SLIM Node 2.
- **MLS End-to-End Encryption**: Exchanging key commits/welcomes across multiple hops, establishing secure group states (Epoch 1). Demonstrates that intermediate nodes 1 & 2 forward packets based only on envelope headers, and cannot decrypt payload ciphertexts.
- **Multicast Capability**: Replicating and distributing a single message payload from a sender (Agent A) to multiple destinations (Agents B, C, D) through the multi-node mesh.
- **Agent2Agent (A2A) RPC integration**: Serialization and execution of protobuf request/response frames over SLIMRPC.

---

## How to Run the Visualizer

### Direct File Open

You can open the [index.html](file:///Users/mamarton/Documents/notes/slim-doc/index.html) file directly in any modern web browser:

- Double-click `index.html` in your file explorer.
- Or drag and drop it into a browser tab.

---

## Interactive Features

1. **Interactive Journeys**: Click the tabs in the sidebar to switch between the 4 main architectural scenarios.
2. **Step Controller**: Pause/resume the flow or click "Next Step" to advance through the steps manually.
3. **Protocol Log Terminal**: View real-time, authentic Rust `tracing` logs (color-coded by component tag) as transmission particles flow across connection lines.
4. **Dynamic Node Badges**: Check active states (like current encryption epoch and routing maps) rendered directly inside the nodes.
5. **Interactive Tooltips**: Hover your cursor over any node in the SVG window (Agent A, SLIM Nodes 1 & 2, Agents B, C, D, or Controller) to view details about its structural role.
