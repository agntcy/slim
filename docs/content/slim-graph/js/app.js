// ponytail: Sequence classic scripts loaded to avoid CORS issues and eliminate complex build tools (Webpack/Vite) for local file opening.
// Simulation Runtime States
let isPaused = false;
let particles = [];
let currentJourney = 'p2p';
let currentStepIndex = 0;
const pathDuration = 3.0;
let customTimers = [];

// Simulation timer wrapper
function setSimulationTimeout(callback, delayMs) {
  customTimers.push({
    callback: callback,
    remaining: delayMs
  });
}

// Node flash styling helpers
function flashNode(elementId, colorClass) {
  const el = document.getElementById(elementId);
  if (!el) return;
  el.classList.add(colorClass);
  setTimeout(() => {
    el.classList.remove(colorClass);
  }, 500);
}

// Update node HTML status values
function updateBadge(nodeId, text, color = 'var(--color-text-secondary)') {
  const badge = document.getElementById(`badge_${nodeId}`);
  if (badge) {
    badge.innerHTML = text;
    badge.style.fill = color;
  }
}

// ----------------------------------------------------
// Interactive Tooltips
// ----------------------------------------------------
document.addEventListener('DOMContentLoaded', () => {
  const tooltipEl = document.getElementById('tooltip');
  const tooltipTitleEl = document.getElementById('tooltip-title');
  const tooltipDescEl = document.getElementById('tooltip-desc');

  if (tooltipEl && tooltipTitleEl && tooltipDescEl) {
    document.querySelectorAll('.interactive-node').forEach(node => {
      node.addEventListener('mouseenter', (e) => {
        const nodeId = e.currentTarget.id;
        const data = NODE_METADATA[nodeId];
        if (data) {
          tooltipTitleEl.innerHTML = `<i class="fa-solid fa-circle-info"></i> ${data.title}`;
          tooltipDescEl.innerHTML = data.desc;
          tooltipEl.style.display = 'block';
        }
      });

      node.addEventListener('mousemove', (e) => {
        const viewport = document.querySelector('.viewport-stage');
        if (!viewport) return;
        const viewportRect = viewport.getBoundingClientRect();
        const mouseX = e.clientX - viewportRect.left;
        const mouseY = e.clientY - viewportRect.top;
        
        tooltipEl.style.left = (mouseX + 15) + 'px';
        tooltipEl.style.top = (mouseY + 15) + 'px';
      });

      node.addEventListener('mouseleave', () => {
        tooltipEl.style.display = 'none';
      });
    });
  }

  // Journey selector click bindings
  document.querySelectorAll('.journey-btn').forEach(btn => {
    btn.addEventListener('click', (e) => {
      const name = e.currentTarget.dataset.journey;
      startScenario(name);
    });
  });
});

// ----------------------------------------------------
// Scenario Journey Controller
// ----------------------------------------------------
function startScenario(name) {
  // Clear timers and particles
  customTimers = [];
  particles.forEach(p => p.destroy());
  particles = [];

  // Reset static visual path states
  document.querySelectorAll('.connection-path').forEach(path => {
    path.classList.remove('active', 'active-crypto', 'active-rpc', 'active-control');
  });

  // Reset shields visually
  document.querySelectorAll('.shield-outer').forEach(shield => {
    shield.classList.remove('active');
  });

  // Map of active nodes per scenario to dynamically gray out unused nodes
  const ACTIVE_NODES_BY_SCENARIO = {
    p2p: ['node_Agent_A', 'node_Agent_B', 'node_Node1', 'node_Node2'],
    multicast: ['node_Agent_A', 'node_Agent_B', 'node_Agent_C', 'node_Agent_D', 'node_Node1', 'node_Node2']
  };

  // Reset opacities dynamically (gray out unused components)
  const activeNodes = ACTIVE_NODES_BY_SCENARIO[name] || [];
  document.querySelectorAll('.interactive-node').forEach(node => {
    if (activeNodes.includes(node.id)) {
      node.removeAttribute('opacity');
    } else {
      node.setAttribute('opacity', '0.4');
    }
  });

  updateBadge('Agent_A', 'MLS: Inactive');
  updateBadge('Agent_B', 'MLS: Inactive');
  updateBadge('Agent_C', 'MLS: Inactive');
  updateBadge('Agent_D', 'MLS: Inactive');
  updateBadge('Agent_E', 'MLS: Inactive');
  updateBadge('MCP', 'Files & Search');
  updateBadge('Operator', 'Local CLI');
  updateBadge('Node1', 'Active (3)');
  updateBadge('Node2', 'Active (4)');

  currentJourney = name;
  currentStepIndex = 0;

  // Highlight active journey button
  document.querySelectorAll('.journey-btn').forEach(btn => {
    btn.classList.remove('active');
    if (btn.dataset.journey === name) {
      btn.classList.add('active');
    }
  });

  logToTerminal('System', 'info', 'slim_dataplane::system', `switching to scenario workflow: "${name.toUpperCase()}"`);
  executeStep();
}

function executeStep() {
  const steps = SCENARIOS[currentJourney];
  const step = steps[currentStepIndex];

  const infoStep = document.getElementById('info-step');
  const infoTitle = document.getElementById('info-title');
  const infoDesc = document.getElementById('info-desc');

  if (infoStep) infoStep.innerHTML = `Step ${currentStepIndex + 1} of ${steps.length}`;
  if (infoTitle) infoTitle.innerHTML = step.title;
  if (infoDesc) infoDesc.innerHTML = step.desc;

  step.action();
}

function triggerNextStep() {
  const steps = SCENARIOS[currentJourney];
  if (currentStepIndex < steps.length - 1) {
    currentStepIndex++;
    setSimulationTimeout(() => {
      executeStep();
    }, 1200);
  } else {
    // Automatically restart scenario after 5 seconds of idle display
    setSimulationTimeout(() => {
      startScenario(currentJourney);
    }, 5000);
  }
}

// ----------------------------------------------------
// Frame Render Loop
// ----------------------------------------------------
let lastFrameTime = null;
function updateFrame(timestamp) {
  requestAnimationFrame(updateFrame);

  if (!timestamp) timestamp = performance.now();
  if (lastFrameTime === null) {
    lastFrameTime = timestamp;
    return;
  }

  const elapsed = timestamp - lastFrameTime;
  lastFrameTime = timestamp;

  if (isPaused) return;

  const currentParticles = [...particles];
  particles = [];
  for (const p of currentParticles) {
    if (p.update(elapsed)) {
      particles.push(p);
    }
  }

  // Update custom timers
  const currentTimers = [...customTimers];
  customTimers = [];
  for (const timer of currentTimers) {
    timer.remaining -= elapsed;
    if (timer.remaining <= 0) {
      timer.callback();
    } else {
      customTimers.push(timer);
    }
  }
}

// Output all runtime JS errors directly to the networking log terminal on the screen
window.onerror = (message, source, lineno) => {
  logToTerminal('System', 'error', 'slim_dataplane::system', `JS Error: ${message} at line ${lineno}`);
};

window.onload = () => {
  // Start loop
  requestAnimationFrame(updateFrame);
  logToTerminal('System', 'info', 'slim_dataplane::runner', 'runner initialized successfully.');
  logToTerminal('SLIM Node 1', 'info', 'slim_dataplane::service', 'dataplane server started endpoint=127.0.0.1:50051');
  logToTerminal('SLIM Node 2', 'info', 'slim_dataplane::service', 'started controlplane server endpoint=0.0.0.0:50052');
  startScenario('p2p');
};
