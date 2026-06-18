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
  if (!el) return;
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
    path.classList.remove('active', 'active-crypto', 'active-rpc', 'active-control');
  });

  document.querySelectorAll('.shield-outer').forEach(shield => {
    shield.classList.remove('active');
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

function buildStepTrack() {
  const track = document.getElementById('step-track');
  if (!track) return;

  track.innerHTML = '';
  const steps = SCENARIOS[currentJourney] || [];

  steps.forEach((step, index) => {
    if (index > 0) {
      const connector = document.createElement('div');
      connector.className = 'step-card-bridge';
      connector.setAttribute('aria-hidden', 'true');
      connector.innerHTML = '<i class="fa-solid fa-chevron-right"></i>';
      track.appendChild(connector);
    }

    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'step-card';
    btn.dataset.stepIndex = String(index);
    btn.setAttribute('role', 'tab');
    btn.setAttribute('aria-selected', index === currentStepIndex ? 'true' : 'false');
    btn.innerHTML =
      `<div class="step-card-indicator">Step ${index + 1} of ${steps.length}</div>` +
      `<div class="step-card-title">${step.title}</div>` +
      `<div class="step-card-desc">${formatStepDesc(step.desc)}</div>`;
    btn.addEventListener('click', () => {
      isPaused = false;
      updatePauseButton();
      goToStep(index, true);
    });
    track.appendChild(btn);
  });

  updateStepTrack();
}

function updateStepTrack() {
  const items = document.querySelectorAll('.step-card');
  items.forEach((btn, index) => {
    const isActive = index === currentStepIndex;
    btn.classList.toggle('active', isActive);
    btn.classList.toggle('completed', index < currentStepIndex);
    btn.setAttribute('aria-selected', isActive ? 'true' : 'false');
  });

  document.querySelectorAll('.step-card-bridge').forEach((connector, index) => {
    connector.classList.toggle('completed', index < currentStepIndex);
  });

  const activeItem = items[currentStepIndex];
  if (activeItem) {
    activeItem.scrollIntoView({ behavior: 'smooth', block: 'nearest', inline: 'center' });
  }
}

function updateStepUI() {
  updateStepTrack();
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

function goToStep(index, runAction = false) {
  const steps = SCENARIOS[currentJourney] || [];
  if (index < 0 || index >= steps.length) return;

  resetScenarioVisuals(currentJourney);
  currentStepIndex = index;
  updateStepUI();

  if (runAction) {
    executeStepAction();
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

function startScenario(name) {
  currentJourney = name;
  currentStepIndex = 0;
  isPaused = false;
  updatePauseButton();

  document.querySelectorAll('.journey-btn').forEach(btn => {
    btn.classList.toggle('active', btn.dataset.journey === name);
  });

  resetScenarioVisuals(name);
  buildStepTrack();

  logToTerminal('System', 'info', 'slim_dataplane::system', `switching to scenario workflow: "${name.toUpperCase()}"`);
  updateStepUI();
  executeStepAction();
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
};
