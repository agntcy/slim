// Node details for mouse over tooltips
const NODE_METADATA = {
  'node_Agent_A': {
    title: 'App Node (Agent A - Sender Client)',
    desc: 'Initiating client application. Establishes multiplexed gRPC/HTTP2 channels to its local broker (SLIM Node 1) and utilizes MLS session layers for payload encryption.'
  },
  'node_Agent_E': {
    title: 'App Node (Agent E - Local Subscriber)',
    desc: 'A local subscriber client connected to SLIM Node 1. Demonstrates low-latency local area routing resolved instantly via the broker\'s local subscription tables.'
  },
  'node_Agent_B': {
    title: 'App Node (Agent B - Subscriber / Server)',
    desc: 'Recipient subscriber node. Handles incoming messages, acts as a subscriber to chat topics, and executes RPC stubs over SLIMRPC (SRPC) protocol bindings.'
  },
  'node_Agent_C': {
    title: 'App Node (Agent C - Subscriber)',
    desc: 'Remote subscriber client connected to SLIM Node 2. Subscribes dynamically to message channels to receive replicated multicast streams.'
  },
  'node_Agent_D': {
    title: 'App Node (Agent D - Joiner)',
    desc: 'Subscriber client node. Demonstrates dynamic group membership updates by receiving MLS Commit and Welcome packages to securely join active sessions.'
  },
  'node_Node1': {
    title: 'SLIM Node 1 (Local Data Plane)',
    desc: 'Lightweight local gRPC router node. Establishes connection links, manages local client subscription tables, and forwards multi-hop messages over remote connection tunnels.'
  },
  'node_Node2': {
    title: 'SLIM Node 2 (Cloud Data Plane)',
    desc: 'Cloud-hosted gRPC router node. Handles peer subscriptions, duplicates multicast streams for active clients, and stores packets in store-and-forward buffers if recipients are offline.'
  },
  'node_Controller': {
    title: 'SLIM Controller (Control Plane)',
    desc: 'Out-of-band administration service. Pushes configurations and dynamically registers route mappings (e.g. via slimctl) without inspecting encrypted data-plane traffic.'
  },
  'node_MCP': {
    title: 'MCP Server (Model Context Protocol)',
    desc: 'Application-layer MCP server. Receives tool invocation request payloads routed securely over SLIM and returns the executed search or file data.'
  },
  'node_Operator': {
    title: 'Operator Terminal (Human Input / slimctl)',
    desc: 'Administrative CLI interface used by human operators to query routes, configure tunnels, and push out-of-band commands to the SLIM Controller or directly to local routing nodes.'
  }
};

// Logger whitelist parameters
const VALID_COMPONENTS = ['system', 'agent a', 'agent b', 'agent c', 'agent d', 'agent e', 'slim node 1', 'slim node 2', 'slim controller', 'mcp server', 'controller', 'operator terminal'];
const VALID_LEVELS = ['info', 'debug', 'warning', 'error', 'success', 'trace'];

// Whitelist of authentic log message patterns derived from the actual SLIM codebase
const AUTHENTIC_LOG_PATTERNS = [
  'tracing logs cleared.',
  'runner initialized successfully.',
  'dataplane server started',
  'started controlplane server',
  'client connected',
  'received publication',
  'forwarding message to connection',
  'forwarding to peers',
  'received message',
  'ack received',
  'received ack message',
  'test succeeded',
  'All acknowledgment tests passed!',
  'publish',
  'subscribe',
  'all acks received, remove timer',
  'Sending message',
  'session closed',
  'Session channel closed',
  'connection lost with remote endpoint, attempting to reconnect',
  'connection closed by peer',
  'there is no remote endopoint connected to the session, store the packet and send it later',
  'connection re-established successfully',
  'the message is still in the buffer, try to send it again to all the remotes',
  'starting data plane listener',
  'add message and try to release msgs',
  'Adding member to the MLS group',
  'MLS client initialization completed successfully',
  'pool insert',
  'received message from SLIM',
  'processing stored message',
  'processing stored commit',
  'processing stored proposal',
  'timer started',
  'add to rtx vector',
  'JS Error:',
  'switching to scenario workflow:',
  'RegisterNodeRequest',
  'RegisterNodeResponse',
  'ConfigurationCommand',
  'ConfigurationCommandAck',
  'RouteListRequest',
  'RouteListResponse',
  'DeregisterNodeRequest',
  'DeregisterNodeResponse',
  'slimctl'
];

// Edge keys map to SVG path ids via EDGE_PATH_MAP in app.js
const EDGE_PATH_MAP = {
  'agentA-slimNode1': 'path_A_to_Node1',
  'agentE-slimNode1': 'path_E_to_Node1',
  'mcpServer-slimNode1': 'path_Node1_to_MCP',
  'slimNode1-slimNode2': 'path_Node1_to_Node2',
  'slimNode2-agentB': 'path_Node2_to_B',
  'slimNode2-agentC': 'path_Node2_to_C',
  'slimNode2-agentD': 'path_Node2_to_D',
  'opTerminal-slimNode1': 'path_Operator_to_Node1',
  'slimController-slimNode1': 'path_Controller_to_Node1',
  'slimController-slimNode2': 'path_Controller_to_Node2'
};

// Architectural Scenarios & Step Definitions
const SCENARIOS = {
  // Use Case 1: Point-to-Point Message
  p2p: [
    {
      title: "Publish P2P Message",
      shortTitle: "Publish",
      activeEdges: ['agentA-slimNode1'],
      desc: "Agent A publishes a Point-to-Point message targeting Agent B (<code>agntcy/ns/AgentB</code>). The payload is pushed to the local **SLIM Node 1** over an HTTP/2 gRPC channel.",
      action: () => {
        logToTerminal('Agent A', 'info', 'slim_dataplane::service', 'Sending message');
        
        spawn2DParticle('path_A_to_Node1', 'var(--color-blue)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Second Node Forwarding (Node 1 -> Node 2)",
      shortTitle: "Forward",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2'],
      desc: "The local **SLIM Node 1** receives the envelope. It checks its routing table and forwards it to the second node (**SLIM Node 2**) in the cloud.",
      action: () => {
        flashNode('core_Node1', 'flash-amber');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');
        
        spawn2DParticle('path_Node1_to_Node2', 'var(--color-blue)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Cloud Node Routing (Node 2 -> Agent B)",
      shortTitle: "Route",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode2-agentB'],
      desc: "The cloud **SLIM Node 2** receives the envelope and routes it directly to its peer connection destination, Agent B.",
      action: () => {
        flashNode('core_Node2', 'flash-orange');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');
        
        spawn2DParticle('path_Node2_to_B', 'var(--color-blue)', 6, 0.025, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Message Delivery & Acknowledgment",
      shortTitle: "ACK",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode2-agentB'],
      desc: "Agent B processes the incoming packet. It generates a transaction acknowledgment (ACK) flowing back along the connection paths in reverse to Agent A.",
      action: () => {
        flashNode('core_Agent_B', 'flash-green');
        logToTerminal('Agent B', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('Agent B', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
        
        spawn2DParticle('path_Node2_to_B', 'var(--color-blue)', 5, 0.028, 'dot', () => {
          spawn2DParticle('path_Node1_to_Node2', 'var(--color-blue)', 5, 0.028, 'dot', () => {
            spawn2DParticle('path_A_to_Node1', 'var(--color-blue)', 5, 0.028, 'dot', () => {
              logToTerminal('Agent A', 'debug', 'slim_dataplane::session::subscription_manager', 'ack received');
              logToTerminal('System', 'info', 'slim_dataplane::system', 'test succeeded');
              triggerNextStep();
            }, true);
          }, true);
        }, true);
      }
    }
  ],

  // Use Case 2: Multicast Message
  multicast: [
    {
      title: "Publish Multicast Payload",
      shortTitle: "Publish",
      activeEdges: ['agentA-slimNode1'],
      desc: "Agent A publishes a multicast payload to channel <code>agntcy/ns/chat</code>. The message is pushed to the local **SLIM Node 1** over HTTP/2.",
      action: () => {
        logToTerminal('Agent A', 'info', 'slim_dataplane::service', 'publish');
        
        spawn2DParticle('path_A_to_Node1', 'var(--color-amber)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Multicast Forwarding (Node 1 -> Node 2)",
      shortTitle: "Forward",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2'],
      desc: "Local **SLIM Node 1** receives the publication and forwards the multicast envelope to the cloud **SLIM Node 2**.",
      action: () => {
        flashNode('core_Node1', 'flash-amber');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');
        
        spawn2DParticle('path_Node1_to_Node2', 'var(--color-amber)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Cloud Multicast Fanout",
      shortTitle: "Fanout",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode2-agentB', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: "The cloud **SLIM Node 2** receives the envelope. It matches the channel name against its routing table, replicates the packet, and streams it to all active subscribers (Agent B, Agent C, Agent D).",
      action: () => {
        flashNode('core_Node2', 'flash-orange');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'forwarding to peers');
        
        let done = 0;
        const onDelivery = () => {
          done++;
          if (done === 3) triggerNextStep();
        };
        spawn2DParticle('path_Node2_to_B', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
        spawn2DParticle('path_Node2_to_C', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
        spawn2DParticle('path_Node2_to_D', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
      }
    },
    {
      title: "Subscribers Receive Payload",
      shortTitle: "Receive",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode2-agentB', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: "Subscribed client nodes receive and parse the payload, returning acknowledgments back to Agent A.",
      action: () => {
        flashNode('core_Agent_B', 'flash-green');
        flashNode('core_Agent_C', 'flash-green');
        flashNode('core_Agent_D', 'flash-green');
        logToTerminal('Agent B', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('Agent C', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('Agent D', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('Agent B', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
        logToTerminal('Agent C', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
        logToTerminal('Agent D', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');

        let acksFromSubscribers = 0;
        const onSubscriberAck = () => {
          acksFromSubscribers++;
          if (acksFromSubscribers === 3) {
            spawnStaggeredReverseParticles(
              'path_Node1_to_Node2',
              'var(--color-amber)',
              5,
              0.028,
              3,
              () => {
                spawnStaggeredReverseParticles(
                  'path_A_to_Node1',
                  'var(--color-amber)',
                  5,
                  0.028,
                  3,
                  () => {
                    logToTerminal('Agent A', 'debug', 'slim_dataplane::session::subscription_manager', 'ack received');
                    logToTerminal('Agent A', 'info', 'slim_dataplane::service', 'All acknowledgment tests passed!');
                    logToTerminal('System', 'info', 'slim_dataplane::system', 'test succeeded');
                    triggerNextStep();
                  }
                );
              }
            );
          }
        };

        spawn2DParticle('path_Node2_to_B', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, true);
        spawn2DParticle('path_Node2_to_C', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, true);
        spawn2DParticle('path_Node2_to_D', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, true);
      }
    }
  ],
};
