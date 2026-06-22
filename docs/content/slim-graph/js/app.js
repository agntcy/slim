// Simulation Runtime States
let isPaused = false;
let particles = [];
let currentJourney = 'p2p';
let currentStepIndex = 0;
const pathDuration = 3.0;
let customTimers = [];

function setSimulationTimeout(callback, delayMs) {
  customTimers.push({
    callback: callback,
    remaining: delayMs
  });
}

function clearSimulation() {
  customTimers = [];
  particles.forEach(p => p.destroy());
  particles = [];
}

function flashNode(elementId, colorClass) {
  const el = document.getElementById(elementId);
  if (!el || el.classList.contains('route-active')) return;
  el.classList.add(colorClass);
  setTimeout(() => {
    el.classList.remove(colorClass);
  }, 500);
}

function updateBadge(nodeId, text, color = 'var(--color-text-secondary)') {
  const badge = document.getElementById(`badge_${nodeId}`);
  if (badge) {
    badge.innerHTML = text;
    badge.style.fill = color;
  }
}

function resetScenarioVisuals(name) {
  clearSimulation();

  document.querySelectorAll('.connection-path').forEach(path => {
    path.classList.remove('active', 'active-crypto', 'active-rpc', 'active-control', 'route-active');
  });

  document.querySelectorAll('.node-rect').forEach(rect => {
    rect.classList.remove('route-active');
  });

  const ACTIVE_NODES_BY_SCENARIO = {
    p2p: ['node_Agent_A', 'node_Agent_B', 'node_Node1', 'node_Node2'],
    multicast: ['node_Agent_A', 'node_Agent_B', 'node_Agent_C', 'node_Agent_D', 'node_Node1', 'node_Node2']
  };

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
}

function formatStepDesc(text) {
  return text.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
}

const TOOLTIP_EMBEDDED = window.parent !== window;
const TOOLTIP_OFFSET = 14;
let activeTooltipContent = null;

function postTooltipToParent(payload) {
  if (!TOOLTIP_EMBEDDED) return;
  try {
    window.parent.postMessage(
      { type: 'slim-graph-tooltip', ...payload },
      window.location.origin
    );
  } catch (_error) {
    /* standalone or cross-origin */
  }
}

function postStepTooltipToParent(payload) {
  if (!TOOLTIP_EMBEDDED) return;
  try {
    window.parent.postMessage(
      { type: 'slim-graph-step-tooltip', ...payload },
      window.location.origin
    );
  } catch (_error) {
    /* standalone or cross-origin */
  }
}

function reportEmbedHeight() {
  if (!TOOLTIP_EMBEDDED) return;
  const root = document.querySelector('.app-grid');
  const height = root
    ? Math.ceil(root.getBoundingClientRect().height)
    : document.documentElement.scrollHeight;
  try {
    window.parent.postMessage(
      { type: 'slim-graph-resize', height },
      window.location.origin
    );
  } catch (_error) {
    /* standalone or cross-origin */
  }
}

window.slimGraphReportHeight = reportEmbedHeight;

function positionNodeTooltip(event) {
  const tooltipEl = document.getElementById('tooltip');
  const x = event.clientX + TOOLTIP_OFFSET;
  const y = event.clientY + TOOLTIP_OFFSET;

  if (TOOLTIP_EMBEDDED) {
    if (!activeTooltipContent) return;
    postTooltipToParent({
      visible: true,
      x: event.clientX,
      y: event.clientY,
      title: activeTooltipContent.title,
      desc: activeTooltipContent.desc
    });
    return;
  }

  if (!tooltipEl) return;
  tooltipEl.style.left = x + 'px';
  tooltipEl.style.top = y + 'px';
}

function showNodeTooltip(event, data) {
  activeTooltipContent = data;
  const tooltipEl = document.getElementById('tooltip');
  const tooltipTitleEl = document.getElementById('tooltip-title');
  const tooltipDescEl = document.getElementById('tooltip-desc');

  if (TOOLTIP_EMBEDDED) {
    postTooltipToParent({
      visible: true,
      x: event.clientX,
      y: event.clientY,
      title: data.title,
      desc: data.desc
    });
    return;
  }

  if (!tooltipEl || !tooltipTitleEl || !tooltipDescEl) return;

  tooltipTitleEl.innerHTML = `<i class="fa-solid fa-circle-info"></i> ${data.title}`;
  tooltipDescEl.textContent = data.desc;
  tooltipEl.hidden = false;
  positionNodeTooltip(event);
}

function hideNodeTooltip() {
  activeTooltipContent = null;

  if (TOOLTIP_EMBEDDED) {
    postTooltipToParent({ visible: false });
    return;
  }

  const tooltipEl = document.getElementById('tooltip');
  if (tooltipEl) {
    tooltipEl.hidden = true;
  }
}

function updateActiveEdges() {
  const steps = SCENARIOS[currentJourney] || [];
  const step = steps[currentStepIndex];
  if (!step || !step.activeEdges) return;

  const activePathIds = new Set();
  const activeCoreIds = new Set();

  step.activeEdges.forEach((edgeKey) => {
    const pathId = EDGE_PATH_MAP[edgeKey];
    if (pathId) activePathIds.add(pathId);

    const [from, to] = edgeKey.split('-');
    const fromNode = EDGE_NODE_MAP[from];
    const toNode = EDGE_NODE_MAP[to];
    if (fromNode) activeCoreIds.add(`core_${fromNode}`);
    if (toNode) activeCoreIds.add(`core_${toNode}`);
  });

  document.querySelectorAll('.connection-path').forEach((path) => {
    const shouldBeActive = activePathIds.has(path.id);
    const isControl = path.classList.contains('control');
    if (shouldBeActive) {
      path.classList.add(isControl ? 'active-control' : 'route-active');
    } else {
      path.classList.remove('route-active', 'active-control');
    }
  });

  document.querySelectorAll('.node-rect').forEach((rect) => {
    if (activeCoreIds.has(rect.id)) {
      rect.classList.add('route-active');
    } else {
      rect.classList.remove('route-active');
    }
  });
}

const EDGE_NODE_MAP = {
  agentA: 'Agent_A',
  agentE: 'Agent_E',
  agentB: 'Agent_B',
  agentC: 'Agent_C',
  agentD: 'Agent_D',
  slimNode1: 'Node1',
  slimNode2: 'Node2',
  mcpServer: 'MCP',
  opTerminal: 'Operator',
  slimController: 'Controller'
};

function buildArrowStepper() {
  const track = document.getElementById('arrow-stepper');
  if (!track) return;

  track.innerHTML = '';
  const steps = SCENARIOS[currentJourney] || [];
  const arrowDepth = 10;

  steps.forEach((step, index) => {
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'arrow-step';
    btn.dataset.stepIndex = String(index);
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-selected', index === currentStepIndex ? 'true' : 'false');
    btn.style.setProperty('--arrow-depth', `${arrowDepth}px`);
    btn.style.zIndex = String(steps.length - index);

    if (index === 0) {
      btn.classList.add('arrow-step--first');
    }
    if (index === steps.length - 1) {
      btn.classList.add('arrow-step--last');
    }

    btn.innerHTML =
      `<span class="arrow-step__badge" aria-hidden="true">${index + 1}</span>` +
      `<span class="arrow-step__title">${step.title}</span>`;

    btn.addEventListener('click', () => {
      isPaused = false;
      updatePauseButton();
      goToStep(index, true);
    });

    btn.addEventListener('mouseenter', () => showStepTooltip(index, btn));
    btn.addEventListener('mouseleave', hideStepTooltip);
    btn.addEventListener('focus', () => showStepTooltip(index, btn));
    btn.addEventListener('blur', hideStepTooltip);

    track.appendChild(btn);
  });

  updateArrowStepper();
}

function showStepTooltip(index, anchor) {
  const steps = SCENARIOS[currentJourney] || [];
  const step = steps[index];
  const tooltip = document.getElementById('step-tooltip');
  const titleEl = document.getElementById('step-tooltip-title');
  const descEl = document.getElementById('step-tooltip-desc');
  if (!step || !tooltip || !titleEl || !descEl || !anchor) return;

  if (TOOLTIP_EMBEDDED) {
    const rect = anchor.getBoundingClientRect();
    postStepTooltipToParent({
      visible: true,
      title: step.title,
      desc: formatStepDesc(step.desc),
      anchorRect: {
        left: rect.left,
        top: rect.top,
        width: rect.width,
        height: rect.height
      }
    });
    return;
  }

  titleEl.textContent = step.title;
  descEl.innerHTML = formatStepDesc(step.desc);
  tooltip.hidden = false;

  const tooltipWidth = 220;
  const rect = anchor.getBoundingClientRect();
  const left = Math.min(
    Math.max(8, rect.left + rect.width / 2 - tooltipWidth / 2),
    window.innerWidth - tooltipWidth - 8
  );

  tooltip.style.width = `${tooltipWidth}px`;
  tooltip.style.left = `${left}px`;
  tooltip.style.top = 'auto';
  tooltip.style.bottom = 'auto';

  requestAnimationFrame(() => {
    const top = Math.max(8, rect.top - tooltip.offsetHeight - 8);
    tooltip.style.top = `${top}px`;
    tooltip.style.left = `${left}px`;
  });
}

function hideStepTooltip() {
  if (TOOLTIP_EMBEDDED) {
    postStepTooltipToParent({ visible: false });
    return;
  }

  const tooltip = document.getElementById('step-tooltip');
  if (tooltip) {
    tooltip.hidden = true;
  }
}

function updateArrowStepper() {
  const items = document.querySelectorAll('.arrow-step');
  items.forEach((btn, index) => {
    const isActive = index === currentStepIndex;
    const isDone = index < currentStepIndex;
    btn.classList.toggle('active', isActive);
    btn.classList.toggle('completed', isDone);
    btn.setAttribute('aria-selected', isActive ? 'true' : 'false');

    const badge = btn.querySelector('.arrow-step__badge');
    if (badge) {
      badge.innerHTML = isDone
        ? '<svg width="8" height="8" viewBox="0 0 12 12" fill="none" aria-hidden="true"><path d="M2 6l3 3 5-5" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/></svg>'
        : String(index + 1);
    }
  });
}

function updateStepUI() {
  updateArrowStepper();
  updateActiveEdges();
}

function updatePauseButton() {
  const pauseBtn = document.getElementById('btn-step-pause');
  const pauseIcon = document.getElementById('btn-step-pause-icon');
  if (!pauseBtn || !pauseIcon) return;

  if (isPaused) {
    pauseBtn.classList.add('is-paused');
    pauseBtn.title = 'Play';
    pauseBtn.setAttribute('aria-label', 'Resume playback');
    pauseIcon.className = 'fa-solid fa-play';
  } else {
    pauseBtn.classList.remove('is-paused');
    pauseBtn.title = 'Pause';
    pauseBtn.setAttribute('aria-label', 'Pause playback');
    pauseIcon.className = 'fa-solid fa-pause';
  }
}

function advanceStep(runAction = true) {
  const steps = SCENARIOS[currentJourney] || [];
  if (currentStepIndex >= steps.length - 1) return;

  currentStepIndex++;
  updateStepUI();

  if (runAction) {
    executeStepAction();
  }
}

function goToStep(index, runAction = false) {
  const steps = SCENARIOS[currentJourney] || [];
  if (index < 0 || index >= steps.length) return;

  clearSimulation();
  document.querySelectorAll('.node-rect').forEach((rect) => {
    rect.classList.remove(
      'flash-blue', 'flash-amber', 'flash-orange', 'flash-teal',
      'flash-purple', 'flash-pink', 'flash-green'
    );
  });
  document.querySelectorAll('.connection-path').forEach((path) => {
    path.classList.remove('active', 'active-crypto', 'active-rpc');
  });

  currentStepIndex = index;
  updateStepUI();

  if (runAction) {
    executeStepAction();
  }
}

function executeStepAction() {
  const steps = SCENARIOS[currentJourney];
  const step = steps[currentStepIndex];
  if (!step || typeof step.action !== 'function') return;
  step.action();
}

function executeStep() {
  updateStepUI();
  executeStepAction();
}

function triggerNextStep() {
  if (isPaused) return;

  const steps = SCENARIOS[currentJourney];
  if (currentStepIndex < steps.length - 1) {
    setSimulationTimeout(() => {
      advanceStep(true);
    }, 1200);
    return;
  }

  setSimulationTimeout(() => {
    if (!isPaused) {
      startScenario(currentJourney);
    }
  }, 5000);
}

function togglePause() {
  isPaused = !isPaused;
  updatePauseButton();
}

function updateJourneyChrome() {
  const wrap = document.querySelector('.arrow-stepper-wrap');
  if (wrap) {
    wrap.classList.toggle('arrow-stepper-wrap--multicast', currentJourney === 'multicast');
  }
}

function startScenario(name) {
  currentJourney = name;
  currentStepIndex = 0;
  isPaused = false;
  updatePauseButton();

  document.querySelectorAll('.journey-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.journey === name);
  });

  updateJourneyChrome();
  resetScenarioVisuals(name);
  buildArrowStepper();

  logToTerminal('System', 'info', 'slim_dataplane::system', `switching to scenario workflow: "${name.toUpperCase()}"`);
  updateStepUI();
  executeStepAction();
  reportEmbedHeight();
}

document.addEventListener('DOMContentLoaded', () => {
  document.querySelectorAll('.interactive-node').forEach(node => {
    node.addEventListener('mouseenter', (e) => {
      const nodeId = e.currentTarget.id;
      const data = NODE_METADATA[nodeId];
      if (data) {
        showNodeTooltip(e, data);
      }
    });

    node.addEventListener('mousemove', (e) => {
      positionNodeTooltip(e);
    });

    node.addEventListener('mouseleave', () => {
      hideNodeTooltip();
    });
  });

  document.querySelectorAll('.journey-btn').forEach(btn => {
    btn.addEventListener('click', (e) => {
      document.querySelectorAll('.journey-btn').forEach((tab) => {
        tab.setAttribute('aria-selected', tab === e.currentTarget ? 'true' : 'false');
      });
      startScenario(e.currentTarget.dataset.journey);
    });
  });

  document.getElementById('btn-step-pause')?.addEventListener('click', togglePause);
});

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

window.onerror = (message, source, lineno) => {
  logToTerminal('System', 'error', 'slim_dataplane::system', `JS Error: ${message} at line ${lineno}`);
};

window.onload = () => {
  if (window.slimGraphSyncTheme) {
    window.slimGraphSyncTheme();
  }
  requestAnimationFrame(updateFrame);
  logToTerminal('System', 'info', 'slim_dataplane::runner', 'runner initialized successfully.');
  logToTerminal('SLIM Node 1', 'info', 'slim_dataplane::service', 'dataplane server started endpoint=127.0.0.1:50051');
  logToTerminal('SLIM Node 2', 'info', 'slim_dataplane::service', 'started controlplane server endpoint=0.0.0.0:50052');
  startScenario('p2p');
  reportEmbedHeight();

  if (TOOLTIP_EMBEDDED && typeof ResizeObserver !== 'undefined') {
    const root = document.querySelector('.app-grid');
    if (root) {
      const resizeObserver = new ResizeObserver(() => reportEmbedHeight());
      resizeObserver.observe(root);
    }
  }
};
