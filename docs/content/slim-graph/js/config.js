const AI_AGENT_DESC =
  'An AI Agent exposed through A2A (Langchain, CrewAI, ADK) which communicates with other A2A Agents and MCP tools';

const SHOW_PROTOCOL_LOG = false;

// Node details for mouse over tooltips
const NODE_METADATA = {
  'node_Agent_A': {
    title: 'CLI/IDE Agent',
    desc: 'Registered as agntcy/demo/agent-a. A local coding Agent which wants to access MCP tools and remote Agents.'
  },
  'node_Agent_E': {
    title: 'AI Agent A',
    desc: AI_AGENT_DESC
  },
  'node_Agent_B': {
    title: 'AI Agent B',
    desc: 'Registered as agntcy/ns/AgentB. ' + AI_AGENT_DESC
  },
  'node_Agent_C': {
    title: 'AI Agent C',
    desc: AI_AGENT_DESC
  },
  'node_Agent_D': {
    title: 'AI Agent D',
    desc: AI_AGENT_DESC
  },
  'node_Node1': {
    title: 'SLIM Node 1 (Local Data Plane)',
    desc: 'A local SLIM Data Plane Node. Establishes an outbound connection to the Cloud data plane.'
  },
  'node_Node2': {
    title: 'SLIM Node 2 (Cloud Data Plane)',
    desc: 'Cloud-hosted SLIM Data plane node. Receives inbound connections from other data plane nodes. Cloud hosted Agents connect here without being publicly exposed.'
  },
  'node_MCP': {
    title: 'MCP Server (Model Context Protocol)',
    desc: 'Registered as agntcy/demo/mcp-tools. Receives tool invocation request payloads routed securely over SLIM and returns the executed search or file data.'
  }
};

const JOURNEY_LABELS = {
  p2p: { label: 'Point-to-Point Messaging (A2A)', icon: 'fa-message' },
  'p2p-mcp': { label: 'Point-to-Point Messaging (MCP)', icon: 'fa-plug' },
  multicast: { label: 'Multicast Messaging (A2A)', icon: 'fa-share-nodes' },
};

const NODE_DISPLAY = {
  node_Agent_A: { title: 'CLI/IDE Agent', subtitle: 'agntcy/demo/agent-a', icon: 'claude' },
  node_Agent_E: { title: 'AI Agent A', subtitle: 'agntcy/ns/chat', icon: 'langchain' },
  node_Agent_B: { title: 'AI Agent B', subtitle: 'agntcy/ns/AgentB', icon: 'crewai' },
  node_Agent_C: { title: 'AI Agent C', subtitle: 'agntcy/ns/chat', icon: 'langgraph' },
  node_Agent_D: { title: 'AI Agent D', subtitle: 'agntcy/ns/chat', icon: 'opencode' },
  node_Node1: { title: 'SLIM Node 1', subtitle: 'Local Data Plane', icon: 'slim' },
  node_Node2: { title: 'SLIM Node 2', subtitle: 'Cloud Data Plane', icon: 'slim' },
  node_MCP: { title: 'MCP Server', subtitle: 'agntcy/demo/mcp-tools', icon: 'mcp' },
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
      desc: "Agent A publishes a Point-to-Point message targeting Agent B (<code>agntcy/ns/AgentB</code>). The payload is pushed to **SLIM Node 1**",
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
      title: "Create Channel",
      shortTitle: "Create",
      activeEdges: ['agentA-slimNode1'],
      desc: "Agent A creates channel <code>agntcy/ns/chat</code>, establishing routes in **SLIM Node 1** for local and remote fanout.",
      action: () => {
        flashNode('core_Agent_A', 'flash-amber');
        logToTerminal('Agent A', 'info', 'slim_dataplane::service', 'publish');
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
      desc: "Agent A invites **AI Agent A** (local), **AI Agent C**, and **AI Agent D** (remote) to the channel. Invite acknowledgments flow through **SLIM Node 1** and **SLIM Node 2**.",
      action: () => {
        flashNode('core_Node1', 'flash-amber');
        flashNode('core_Node2', 'flash-orange');
        logToTerminal('Agent A', 'info', 'slim_dataplane::service', 'Adding member to the MLS group');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');

        spawn2DParticle('path_A_to_Node1', 'var(--color-amber)', 5, 0.022, 'dot', () => {
          let invitesDone = 0;
          const onInviteDone = () => {
            invitesDone++;
            if (invitesDone === 3) triggerNextStep();
          };

          logToTerminal('AI Agent A', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
          spawn2DParticle('path_E_to_Node1', 'var(--color-amber)', 5, 0.022, 'dot', onInviteDone, true);

          spawn2DParticle('path_Node1_to_Node2', 'var(--color-amber)', 5, 0.022, 'dot', () => {
            logToTerminal('Agent C', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
            logToTerminal('Agent D', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
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
      desc: "Agent A publishes a multicast payload to <code>agntcy/ns/chat</code>. The message is pushed to the local **SLIM Node 1**.",
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
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode1-agentE', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: "**SLIM Node 1** delivers locally to **AI Agent A**. **SLIM Node 2** replicates the envelope to remote subscribers **AI Agent C** and **AI Agent D**.",
      action: () => {
        flashNode('core_Node1', 'flash-amber');
        flashNode('core_Node2', 'flash-orange');
        logToTerminal('SLIM Node 1', 'debug', 'slim_dataplane::datapath', 'forwarding message to connection');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'received publication');
        logToTerminal('SLIM Node 2', 'debug', 'slim_dataplane::datapath', 'forwarding to peers');
        
        let done = 0;
        const onDelivery = () => {
          done++;
          if (done === 3) triggerNextStep();
        };
        spawn2DParticle('path_E_to_Node1', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery, true);
        spawn2DParticle('path_Node2_to_C', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
        spawn2DParticle('path_Node2_to_D', 'var(--color-amber)', 6, 0.025, 'dot', onDelivery);
      }
    },
    {
      title: "Subscribers Receive Payload",
      shortTitle: "Receive",
      activeEdges: ['agentA-slimNode1', 'slimNode1-slimNode2', 'slimNode1-agentE', 'slimNode2-agentC', 'slimNode2-agentD'],
      desc: "Subscribers on local and cloud nodes receive the payload and return acknowledgments back to Agent A.",
      action: () => {
        flashNode('core_Agent_E', 'flash-green');
        flashNode('core_Agent_C', 'flash-green');
        flashNode('core_Agent_D', 'flash-green');
        logToTerminal('AI Agent A', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('Agent C', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('Agent D', 'info', 'slim_dataplane::service', 'received message');
        logToTerminal('AI Agent A', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
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

        spawn2DParticle('path_E_to_Node1', 'var(--color-amber)', 5, 0.028, 'dot', onSubscriberAck, true);
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
      desc: "Cloud **AI Agent B** sends a point-to-point MCP tool invocation targeting <code>agntcy/demo/mcp-tools</code> on the on-prem **SLIM Node 1**.",
      action: () => {
        logToTerminal('Agent B', 'info', 'slim_dataplane::service', 'Sending message');

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
          updateBadge('MCP', 'agntcy/demo/mcp-tools');
          triggerNextStep();
        }, 900);
      }
    },
    {
      title: "Response Delivery & Acknowledgment",
      shortTitle: "ACK",
      activeEdges: ['slimNode2-agentB', 'slimNode2-slimNode1', 'slimNode1-mcpServer'],
      desc: "The MCP response travels back through **SLIM Node 1** and **SLIM Node 2** to cloud **AI Agent B**, completing the cross-environment MCP exchange.",
      action: () => {
        spawn2DParticle('path_Node1_to_MCP', 'var(--color-teal)', 5, 0.028, 'dot', () => {
          spawn2DParticle('path_Node1_to_Node2', 'var(--color-teal)', 5, 0.028, 'dot', () => {
            spawn2DParticle('path_Node2_to_B', 'var(--color-teal)', 5, 0.028, 'dot', () => {
              flashNode('core_Agent_B', 'flash-green');
              logToTerminal('Agent B', 'info', 'slim_dataplane::service', 'received message');
              logToTerminal('Agent B', 'debug', 'slim_dataplane::session::subscription_manager', 'received ack message');
              logToTerminal('Agent B', 'debug', 'slim_dataplane::session::subscription_manager', 'ack received');
              logToTerminal('System', 'info', 'slim_dataplane::system', 'test succeeded');
              triggerNextStep();
            });
          });
        }, true);
      }
    }
  ],
};
