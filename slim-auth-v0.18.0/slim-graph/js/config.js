const AI_AGENT_DESC =
  'Exposed through A2A (LangChain, CrewAI, ADK) and communicates with other A2A agents and MCP tools.';

const SHOW_PROTOCOL_LOG = false;

// SLIM registration names (graph subtitles, tooltips, step descriptions)
const AGENT_NAMES = {
  agentA: 'agntcy/edge/cli',
  agentE: 'agntcy/edge/agent-a',
  agentB: 'agntcy/cloud/agent-b',
  agentC: 'agntcy/cloud/agent-c',
  agentD: 'agntcy/cloud/agent-d',
  chat: 'agntcy/cloud/chat',
  mcp: 'agntcy/edge/mcp-tools',
};

// Display names — publisher (CLI/IDE) vs channel subscribers (AI Agent *)
const AGENT_ROLES = {
  publisher: 'CLI/IDE Agent',
  subscriberLocal: 'AI Agent A',
  agentB: 'AI Agent B',
  agentC: 'AI Agent C',
  agentD: 'AI Agent D',
};

// Node details for mouse over tooltips
const NODE_METADATA = {
  'node_Agent_A': {
    title: 'CLI/IDE Agent',
    desc: 'A local agent that connects to SLIM to communicate with other agents and MCP tools.'
  },
  'node_Agent_E': {
    title: 'AI Agent A',
    desc: 'A local AI agent connected through the edge data plane. ' + AI_AGENT_DESC
  },
  'node_Agent_B': {
    title: 'AI Agent B',
    desc: 'A cloud-hosted AI agent connected through the cloud data plane. ' + AI_AGENT_DESC
  },
  'node_Agent_C': {
    title: 'AI Agent C',
    desc: 'A cloud-hosted AI agent connected through the cloud data plane. ' + AI_AGENT_DESC
  },
  'node_Agent_D': {
    title: 'AI Agent D',
    desc: 'A cloud-hosted AI agent connected through the cloud data plane. ' + AI_AGENT_DESC
  },
  'node_Node1': {
    title: 'SLIM Node 1 (Local Data Plane)',
    desc: 'A local SLIM data plane node that establishes an outbound connection to the cloud data plane.'
  },
  'node_Node2': {
    title: 'SLIM Node 2 (Cloud Data Plane)',
    desc: 'A cloud-hosted SLIM data plane node that receives inbound connections from other data plane nodes. Cloud-hosted agents connect here without being publicly exposed.'
  },
  'node_MCP': {
    title: 'MCP Server (Model Context Protocol)',
    desc: 'An on-prem MCP server that receives tool invocation requests routed securely over SLIM and returns executed search or file data.'
  }
};

const JOURNEY_LABELS = {
  p2p: { label: 'Point-to-Point Messaging (A2A)', icon: 'fa-message' },
  'p2p-mcp': { label: 'Point-to-Point Messaging (MCP)', icon: 'fa-plug' },
  multicast: { label: 'Multicast Messaging (A2A)', icon: 'fa-share-nodes' },
};

const NODE_DISPLAY = {
  node_Agent_A: { title: 'CLI/IDE Agent', subtitle: AGENT_NAMES.agentA, icon: 'claude' },
  node_Agent_E: { title: 'AI Agent A', subtitle: AGENT_NAMES.agentE, icon: 'langchain' },
  node_Agent_B: { title: 'AI Agent B', subtitle: AGENT_NAMES.agentB, icon: 'crewai' },
  node_Agent_C: { title: 'AI Agent C', subtitle: AGENT_NAMES.agentC, icon: 'langgraph' },
  node_Agent_D: { title: 'AI Agent D', subtitle: AGENT_NAMES.agentD, icon: 'opencode' },
  node_Node1: { title: 'SLIM Node 1', subtitle: 'Local Data Plane', icon: 'slim' },
  node_Node2: { title: 'SLIM Node 2', subtitle: 'Cloud Data Plane', icon: 'slim' },
  node_MCP: { title: 'MCP Server', subtitle: AGENT_NAMES.mcp, icon: 'mcp' },
};

const VALID_COMPONENTS = ['system', 'cli/ide agent', 'ai agent a', 'ai agent b', 'ai agent c', 'ai agent d', 'slim node 1', 'slim node 2', 'mcp server'];
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
  'slimNode1-mcpServer': 'path_Node1_to_MCP',
  'slimNode1-slimNode2': 'path_Node1_to_Node2',
  'slimNode2-agentB': 'path_Node2_to_B',
  'slimNode2-agentC': 'path_Node2_to_C',
  'slimNode2-agentD': 'path_Node2_to_D',
  'agentB-slimNode2': 'path_Node2_to_B',
  'slimNode1-agentE': 'path_E_to_Node1',
  'slimNode2-slimNode1': 'path_Node1_to_Node2',
};

// Architectural Scenarios & Step Definitions
const SCENARIOS = {
  // Use Case 1: Point-to-Point Message
  p2p: [
    {
      title: "Publish P2P Message",
      shortTitle: "Publish",
      activeEdges: ['agentA-slimNode1'],
      desc: `**${AGENT_ROLES.publisher}** publishes a Point-to-Point message targeting **${AGENT_ROLES.agentB}** (**${AGENT_NAMES.agentB}**). The payload is pushed to **SLIM Node 1**`,
      action: () => {
        logToTerminal(AGENT_ROLES.publisher, 'info', 'slim_dataplane::service', 'Sending message');
        
        spawn2DParticle('path_A_to_Node1', 'var(--color-blue)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Second Node Forwarding",
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
      title: "Cloud Node Routing",
      shortTitle: "Route",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode2-agentB'],
      desc: `The cloud **SLIM Node 2** receives the envelope and routes it directly to its peer connection destination, **${AGENT_ROLES.agentB}**.`,
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
      desc: `**${AGENT_ROLES.agentB}** processes the incoming packet. It generates a transaction acknowledgment (ACK) flowing back along the connection paths in reverse to the **${AGENT_ROLES.publisher}**.`,
      action: () => {
        flashNode('core_Agent_B', 'flash-green');
        logToTerminal(AGENT_ROLES.agentB, 'info', 'slim_dataplane::service', 'received message');
        logToTerminal(AGENT_ROLES.agentB, 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
        
        spawn2DParticle('path_Node2_to_B', 'var(--color-blue)', 5, 0.028, 'dot', () => {
          spawn2DParticle('path_Node1_to_Node2', 'var(--color-blue)', 5, 0.028, 'dot', () => {
            spawn2DParticle('path_A_to_Node1', 'var(--color-blue)', 5, 0.028, 'dot', () => {
              logToTerminal(AGENT_ROLES.publisher, 'debug', 'slim_dataplane::session::subscription_manager', 'ack received');
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
      title: "Create Channel",
      shortTitle: "Create",
      activeEdges: ['agentA-slimNode1'],
      desc: `**${AGENT_ROLES.publisher}** creates channel **${AGENT_NAMES.chat}**, establishing routes in **SLIM Node 1** for local and remote fanout.`,
      action: () => {
        flashNode('core_Agent_A', 'flash-amber');
        logToTerminal(AGENT_ROLES.publisher, 'info', 'slim_dataplane::service', 'publish');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'received publication');

        spawn2DParticle('path_A_to_Node1', 'var(--color-amber)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Invite Channel Members",
      shortTitle: "Invite",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode1-agentE', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: `**${AGENT_ROLES.publisher}** invites **${AGENT_ROLES.subscriberLocal}** (local), **${AGENT_ROLES.agentC}**, and **${AGENT_ROLES.agentD}** (remote) to the channel. Invite acknowledgments flow through **SLIM Node 1** and **SLIM Node 2**.`,
      action: () => {
        flashNode('core_Node1', 'flash-amber');
        flashNode('core_Node2', 'flash-orange');
        logToTerminal(AGENT_ROLES.publisher, 'info', 'slim_dataplane::service', 'Adding member to the MLS group');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');

        spawn2DParticle('path_A_to_Node1', 'var(--color-amber)', 5, 0.022, 'dot', () => {
          let invitesDone = 0;
          const onInviteDone = () => {
            invitesDone++;
            if (invitesDone === 3) triggerNextStep();
          };

          logToTerminal(AGENT_ROLES.subscriberLocal, 'info', 'slim_dataplane::service', 'received message');
          spawn2DParticle('path_E_to_Node1', 'var(--color-amber)', 5, 0.022, 'dot', onInviteDone, true);

          spawn2DParticle('path_Node1_to_Node2', 'var(--color-amber)', 5, 0.022, 'dot', () => {
            logToTerminal(AGENT_ROLES.agentC, 'info', 'slim_dataplane::service', 'received message');
            logToTerminal(AGENT_ROLES.agentD, 'info', 'slim_dataplane::service', 'received message');
            spawn2DParticle('path_Node2_to_C', 'var(--color-amber)', 5, 0.022, 'dot', onInviteDone);
            spawn2DParticle('path_Node2_to_D', 'var(--color-amber)', 5, 0.022, 'dot', onInviteDone);
          });
        });
      }
    },
    {
      title: "Publish Multicast Payload",
      shortTitle: "Publish",
      activeEdges: ['agentA-slimNode1'],
      desc: `**${AGENT_ROLES.publisher}** publishes a multicast payload to **${AGENT_NAMES.chat}**. The message is pushed to the local **SLIM Node 1**.`,
      action: () => {
        logToTerminal(AGENT_ROLES.publisher, 'info', 'slim_dataplane::service', 'publish');

        spawn2DParticle('path_A_to_Node1', 'var(--color-amber)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "Multicast Forwarding",
      shortTitle: "Forward",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode1-agentE', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: `Local **SLIM Node 1** forwards the multicast envelope to **SLIM Node 2** and delivers locally to **${AGENT_ROLES.subscriberLocal}**. **SLIM Node 2** replicates the packet to remote subscribers **${AGENT_ROLES.agentC}** and **${AGENT_ROLES.agentD}**.`,
      action: () => {
        flashNode('core_Node1', 'flash-amber');
        flashNode('core_Node2', 'flash-orange');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'forwarding to peers');

        let done = 0;
        const onDelivery = () => {
          done++;
          if (done === 3) triggerNextStep();
        };

        spawn2DParticle('path_Node1_to_Node2', 'var(--color-amber)', 6, 0.02, 'dot', () => {
          spawn2DParticle('path_Node2_to_C', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
          spawn2DParticle('path_Node2_to_D', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
        });
        spawn2DParticle('path_E_to_Node1', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery, true);
      }
    },
    {
      title: "Subscribers Receive Payload",
      shortTitle: "Receive",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode1-agentE', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: `**${AGENT_ROLES.subscriberLocal}**, **${AGENT_ROLES.agentC}**, and **${AGENT_ROLES.agentD}** receive the payload and return acknowledgments to the **${AGENT_ROLES.publisher}** (publisher).`,
      action: () => {
        flashNode('core_Agent_E', 'flash-green');
        flashNode('core_Agent_C', 'flash-green');
        flashNode('core_Agent_D', 'flash-green');
        logToTerminal(AGENT_ROLES.subscriberLocal, 'info', 'slim_dataplane::service', 'received message');
        logToTerminal(AGENT_ROLES.agentC, 'info', 'slim_dataplane::service', 'received message');
        logToTerminal(AGENT_ROLES.agentD, 'info', 'slim_dataplane::service', 'received message');
        logToTerminal(AGENT_ROLES.subscriberLocal, 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
        logToTerminal(AGENT_ROLES.agentC, 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
        logToTerminal(AGENT_ROLES.agentD, 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');

        let acksFromSubscribers = 0;
        const onSubscriberAck = () => {
          acksFromSubscribers++;
          if (acksFromSubscribers === 3) {
            spawnStaggeredReverseParticles(
              'path_Node1_to_Node2',
              'var(--color-amber)',
              5,
              0.028,
              2,
              () => {
                spawnStaggeredReverseParticles(
                  'path_A_to_Node1',
                  'var(--color-amber)',
                  5,
                  0.028,
                  3,
                  () => {
                    logToTerminal(AGENT_ROLES.publisher, 'debug', 'slim_dataplane::session::subscription_manager', 'ack received');
                    logToTerminal(AGENT_ROLES.publisher, 'info', 'slim_dataplane::service', 'All acknowledgment tests passed!');
                    logToTerminal('System', 'info', 'slim_dataplane::system', 'test succeeded');
                    triggerNextStep();
                  }
                );
              }
            );
          }
        };

        spawn2DParticle('path_E_to_Node1', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, false);
        spawn2DParticle('path_Node2_to_C', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, true);
        spawn2DParticle('path_Node2_to_D', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, true);
      }
    }
  ],

  // Use Case 3: Cloud agent invoking on-prem MCP server
  'p2p-mcp': [
    {
      title: "Publish MCP Tool Request",
      shortTitle: "Publish",
      activeEdges: ['slimNode2-agentB'],
      desc: `Cloud **${AGENT_ROLES.agentB}** sends a point-to-point MCP tool invocation targeting **${AGENT_NAMES.mcp}** on the on-prem **SLIM Node 1**.`,
      action: () => {
        logToTerminal(AGENT_ROLES.agentB, 'info', 'slim_dataplane::service', 'Sending message');

        spawn2DParticle('path_Node2_to_B', 'var(--color-teal)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        }, true);
      }
    },
    {
      title: "Cloud to On-Prem Forwarding",
      shortTitle: "Forward",
      activeEdges: ['slimNode2-agentB', 'slimNode2-slimNode1'],
      desc: "The cloud **SLIM Node 2** receives the envelope and forwards it inbound to the on-prem **SLIM Node 1** over the established data-plane connection.",
      action: () => {
        flashNode('core_Node2', 'flash-teal');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');

        spawn2DParticle('path_Node1_to_Node2', 'var(--color-teal)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        }, true);
      }
    },
    {
      title: "On-Prem Route to MCP Server",
      shortTitle: "Route",
      activeEdges: ['slimNode2-agentB', 'slimNode2-slimNode1', 'slimNode1-mcpServer'],
      desc: "The on-prem **SLIM Node 1** matches the destination name against its routing table and forwards the envelope to the co-located **MCP Server**.",
      action: () => {
        flashNode('core_Node1', 'flash-teal');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');

        spawn2DParticle('path_Node1_to_MCP', 'var(--color-teal)', 6, 0.02, 'dot', () => {
          triggerNextStep();
        });
      }
    },
    {
      title: "MCP Tool Execution",
      shortTitle: "Execute",
      activeEdges: ['slimNode2-agentB', 'slimNode2-slimNode1', 'slimNode1-mcpServer'],
      desc: "The on-prem **MCP Server** receives the tool call, executes the handler (for example <code>search_files</code>), and prepares the JSON response payload.",
      action: () => {
        flashNode('core_MCP', 'flash-teal');
        updateBadge('MCP', 'Executing: search_files', 'var(--color-teal)');
        logToTerminal('MCP Server', 'info', 'slim_dataplane::service', 'received message');

        setSimulationTimeout(() => {
          updateBadge('MCP', AGENT_NAMES.mcp);
          triggerNextStep();
        }, 900);
      }
    },
    {
      title: "Response Delivery & Acknowledgment",
      shortTitle: "ACK",
      activeEdges: ['slimNode2-agentB', 'slimNode2-slimNode1', 'slimNode1-mcpServer'],
      desc: `The MCP response travels back through **SLIM Node 1** and **SLIM Node 2** to cloud **${AGENT_ROLES.agentB}**, completing the cross-environment MCP exchange.`,
      action: () => {
        spawn2DParticle('path_Node1_to_MCP', 'var(--color-teal)', 5, 0.028, 'dot', () => {
          spawn2DParticle('path_Node1_to_Node2', 'var(--color-teal)', 5, 0.028, 'dot', () => {
            spawn2DParticle('path_Node2_to_B', 'var(--color-teal)', 5, 0.028, 'dot', () => {
              flashNode('core_Agent_B', 'flash-green');
              logToTerminal(AGENT_ROLES.agentB, 'info', 'slim_dataplane::service', 'received message');
              logToTerminal(AGENT_ROLES.agentB, 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
              logToTerminal(AGENT_ROLES.agentB, 'debug', 'slim_dataplane::session::subscription_manager', 'ack received');
              logToTerminal('System', 'info', 'slim_dataplane::system', 'test succeeded');
              triggerNextStep();
            });
          });
        }, true);
      }
    }
  ],
};
