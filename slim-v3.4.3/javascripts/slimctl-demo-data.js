/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Scripted demo lines and canned CLI responses for the home-page terminal. */
window.SlimctlDemoData = {
  nodeDemoScript: [
    { type: "command", text: "slimctl slim start" },
    {
      type: "output",
      text:
        "INFO slim-data-plane: dataplane listening on 0.0.0.0:46357\n" +
        "INFO slim-controller: controller API on 0.0.0.0:46358",
    },
    { type: "pause", ms: 1400 },
    { type: "command", text: "slimctl controller node list" },
    {
      type: "output",
      text:
        "2 node(s) found\n" +
        "Node ID: slim/b status: CONNECTED\n" +
        "  Connection details:\n" +
        "  - Endpoint: 127.0.0.1:46457\n" +
        "Node ID: slim/a status: CONNECTED\n" +
        "  Connection details:\n" +
        "  - Endpoint: 127.0.0.1:46357",
    },
    { type: "pause", ms: 1400 },
    {
      type: "command",
      text:
        "slimctl controller route add org/default/alice/0 via slim/b --node-id slim/a",
    },
    { type: "output", text: "Route added: org/default/alice/0 → slim/b" },
    { type: "pause", ms: 1400 },
    {
      type: "command",
      text: "slimctl controller route del org/default/alice/0 via slim/b --node-id slim/a",
    },
    { type: "output", text: "Route deleted: org/default/alice/0" },
    { type: "pause", ms: 4000 },
  ],

  messageDemoScript: [
    { type: "python", text: "import slim_bindings" },
    { type: "pause", ms: 600 },
    { type: "comment", text: "# Connect to the SLIM data plane" },
    {
      type: "python",
      text: 'slim_bindings.connect("http://127.0.0.1:46357")',
    },
    { type: "pause", ms: 800 },
    { type: "comment", text: "# Create CLI/IDE Agent application (local)" },
    {
      type: "python",
      text: 'agent_a = slim_bindings.create_app("agntcy/edge/cli")',
    },
    { type: "pause", ms: 700 },
    { type: "comment", text: "# Create AI Agent B application (cloud)" },
    {
      type: "python",
      text: 'agent_b = slim_bindings.create_app("agntcy/cloud/agent-b")',
    },
    { type: "pause", ms: 800 },
    {
      type: "comment",
      text: "# Create a point-to-point session from CLI/IDE Agent to AI Agent B",
    },
    {
      type: "python",
      text:
        'session_a = agent_a.create_session(session_type=POINT_TO_POINT, "agntcy/cloud/agent-b")',
    },
    { type: "pause", ms: 900 },
    {
      type: "comment",
      text: "# Agent B listens for and accepts the new session",
    },
    { type: "python", text: "session_b = agent_b.listen_for_session()" },
    { type: "pause", ms: 700 },
    { type: "comment", text: "# Agent A sends a message" },
    {
      type: "python",
      text: 'session_a.publish("Hello from CLI/IDE Agent")',
    },
    { type: "pause", ms: 800 },
    { type: "comment", text: "# Agent B receives the message" },
    {
      type: "output",
      text: '>>> print(session_b.get_message().payload)\n"Hello from CLI/IDE Agent"',
    },
    { type: "pause", ms: 900 },
    { type: "comment", text: "# Agent B responds" },
    {
      type: "python",
      text: 'session_b.publish("Nice to meet you, I\'m AI Agent B")',
    },
    { type: "pause", ms: 800 },
    { type: "comment", text: "# Agent A receives the message" },
    {
      type: "output",
      text:
        '>>> print(session_a.get_message().payload)\n"Nice to meet you, I\'m AI Agent B"',
    },
    { type: "pause", ms: 4000 },
  ],

  demoTitles: {
    node: "user@slim:~",
    message: "python",
  },
};
