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
    {
      type: "command",
      text: "slimctl node connection list --server=127.0.0.1:46358",
    },
    {
      type: "output",
      text:
        "CONNECTION   PEER ENDPOINT              STATE\n" +
        "conn/1       127.0.0.1:46357 (local)    ACTIVE\n" +
        "conn/2       10.0.1.12:46357 (peer)     ACTIVE",
    },
    { type: "pause", ms: 1400 },
    {
      type: "command",
      text:
        "slimctl node route add agntcy/demo/chat/agent-b via peer-node.json --server=127.0.0.1:46358",
    },
    { type: "output", text: "Route added: agntcy/demo/chat/agent-b → 10.0.1.12:46357" },
    { type: "pause", ms: 1400 },
    {
      type: "command",
      text: "slimctl node route list --server=127.0.0.1:46358",
    },
    {
      type: "output",
      text:
        "NAME                         NEXT HOP\n" +
        "agntcy/demo/chat/agent-a     slim/0 (local)\n" +
        "agntcy/demo/chat/agent-b     10.0.1.12:46357\n" +
        "agntcy/demo/chat             slim/0 (multicast)",
    },
    { type: "pause", ms: 4000 },
  ],

  messageDemoScript: [
    {
      type: "user",
      text: "Two agents share a channel on a local SLIM node.",
    },
    { type: "pause", ms: 1200 },
    { type: "command", text: "slimctl slim start" },
    {
      type: "output",
      text: "SLIM node slim/0 ready · dataplane :46357 · controller :46358",
    },
    { type: "pause", ms: 1200 },
    {
      type: "tool",
      text: "agent-a.subscribe(agntcy/demo/chat/agent-a)",
    },
    { type: "output", text: "Subscribed · MLS group epoch=1" },
    { type: "pause", ms: 1000 },
    {
      type: "tool",
      text: "agent-b.subscribe(agntcy/demo/chat/agent-b)",
    },
    { type: "output", text: "Subscribed · MLS group epoch=1" },
    { type: "pause", ms: 1200 },
    {
      type: "user",
      text: "Agent A publishes to the shared channel.",
    },
    { type: "pause", ms: 900 },
    {
      type: "tool",
      text: 'publish("Hello from Agent A") → agntcy/demo/chat',
    },
    {
      type: "output",
      text:
        "INFO slim_dataplane::datapath received publication\n" +
        "INFO slim_dataplane::datapath forwarding message to connection",
    },
    { type: "pause", ms: 1200 },
    {
      type: "tool",
      text: "agent-b.on_message(agntcy/demo/chat)",
    },
    {
      type: "output",
      text: "Hello from Agent A  · end-to-end encrypted (MLS)",
    },
    { type: "pause", ms: 4000 },
  ],

  demoTitles: {
    node: "user@slim:~",
    message: "user@slim:~",
  },

  helpText:
    "Try these commands:\n" +
    "  slimctl help\n" +
    "  slimctl slim start\n" +
    "  slimctl node connection list --server=127.0.0.1:46358\n" +
    "  slimctl node route list --server=127.0.0.1:46358\n" +
    "  help | clear",

  slimctlHelp:
    "SLIM CLI (slimctl)\n\n" +
    "Usage:\n" +
    "  slimctl [OPTIONS] <COMMAND>\n\n" +
    "Available Commands:\n" +
    "  slim        Manage SLIM data plane instances\n" +
    "  node        Manage a SLIM node directly\n" +
    "  config      Manage slimctl configuration\n" +
    "  help        Help about any command",

  nodeConnectionList:
    "CONNECTION   PEER ENDPOINT              STATE\n" +
    "conn/1       127.0.0.1:46357 (local)    ACTIVE",

  nodeRouteList:
    "NAME                         NEXT HOP\n" +
    "agntcy/demo/chat/agent-a     slim/0 (local)\n" +
    "agntcy/demo/chat             slim/0 (multicast)",
};
