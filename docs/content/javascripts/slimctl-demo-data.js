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
    { type: "pause", ms: 800 },
    {
      type: "python",
      text: 'slim_bindings.connect("http://127.0.0.1:46357")',
    },
    { type: "pause", ms: 1000 },
    {
      type: "python",
      text: 'agent_a = slim_bindings.create_app("agntcy/demo/agent-a")',
    },
    { type: "pause", ms: 900 },
    {
      type: "python",
      text: 'agent_b = slim_bindings.create_app("agntcy/demo/agent-b")',
    },
    { type: "pause", ms: 1000 },
    {
      type: "python",
      text:
        'session_a = agent_a.create_session(session_type=POINT_TO_POINT, "agntcy/demo/agent-b")',
    },
    { type: "pause", ms: 1100 },
    { type: "python", text: "session_b = agent_b.listen_for_session()" },
    { type: "pause", ms: 900 },
    {
      type: "python",
      text: 'session_a.publish("Hello from Agent A")',
    },
    { type: "pause", ms: 1000 },
    {
      type: "output",
      text: '>>> print(session_b.get_message().payload)\n"Hello from Agent A"',
    },
    { type: "pause", ms: 1100 },
    {
      type: "python",
      text: 'session_b.publish("Nice to meet you, I\'m Agent B")',
    },
    { type: "pause", ms: 1000 },
    {
      type: "output",
      text:
        '>>> print(session_a.get_message().payload)\n"Nice to meet you, I\'m Agent B"',
    },
    { type: "pause", ms: 4000 },
  ],

  demoTitles: {
    node: "user@slim:~",
    message: "python",
  },

  helpText:
    "Explore the CLI:\n" +
    "  slimctl --help\n" +
    "  slimctl slim --help\n" +
    "  slimctl slim start\n" +
    "  slimctl controller node list\n" +
    "  slimctl controller route list --node-id slim/a\n" +
    "  help | clear",

  slimctlHelp:
    "SLIM CLI (slimctl)\n\n" +
    "Usage:\n" +
    "  slimctl [OPTIONS] <COMMAND>\n\n" +
    "Available Commands:\n" +
    "  slim        Manage SLIM data plane instances\n" +
    "  controller  Manage SLIM nodes via the control plane\n" +
    "  node        Manage a SLIM node directly\n" +
    "  config      Manage slimctl configuration\n" +
    "  help        Help about any command",

  slimSlimHelp:
    "Manage SLIM data plane instances\n\n" +
    "Usage:\n" +
    "  slimctl slim [command]\n\n" +
    "Available Commands:\n" +
    "  start       Start a local SLIM data plane node\n" +
    "  stop        Stop a running SLIM data plane node\n" +
    "  status      Show data plane status",

  controllerNodeList:
    "2 node(s) found\n" +
    "Node ID: slim/b status: CONNECTED\n" +
    "  Connection details:\n" +
    "  - Endpoint: 127.0.0.1:46457\n" +
    "Node ID: slim/a status: CONNECTED\n" +
    "  Connection details:\n" +
    "  - Endpoint: 127.0.0.1:46357",

  controllerRouteList:
    "NAME                         NEXT HOP\n" +
    "org/default/alice/0          slim/b\n" +
    "org/default/bob/0            slim/a (local)",

  nodeConnectionList:
    "CONNECTION   PEER ENDPOINT              STATE\n" +
    "conn/1       127.0.0.1:46357 (local)    ACTIVE",

  nodeRouteList:
    "NAME                         NEXT HOP\n" +
    "agntcy/demo/chat/agent-a     slim/0 (local)\n" +
    "agntcy/demo/chat             slim/0 (multicast)",
};
