// Helper to format component slug names to CSS classes
function getCompTagClass(component) {
  switch (component.toLowerCase()) {
    case 'agent a': return 'comp-agent-a';
    case 'agent b': return 'comp-agent-b';
    case 'agent c': return 'comp-agent-c';
    case 'agent d': return 'comp-agent-d';
    case 'agent e': return 'comp-agent-b';
    case 'slim node 1': return 'comp-slim-node-1';
    case 'slim node 2': return 'comp-slim-node-2';
    case 'controller': return 'comp-controller';
    case 'operator terminal': return 'comp-controller';
    case 'mcp server': return 'comp-mcp';
    default: return 'comp-system';
  }
}

// Structured Telemetry Log Handler
function logToTerminal(component, level, target, message) {
  const terminalBody = document.getElementById('terminal-body');
  if (!terminalBody) return;

  // Validate component, level, and target structure
  if (!VALID_COMPONENTS.includes(component.toLowerCase()) || 
      !VALID_LEVELS.includes(level.toLowerCase()) ||
      typeof target !== 'string' || 
      (!target.startsWith('slim_') && !target.startsWith('slim_dataplane::') && target !== 'slimctl')) {
    console.warn(`Blocked invalid log attempt: component=${component}, level=${level}, target=${target}`);
    return;
  }

  // Strictly validate that only authentic logs (whitelisted) can appear in the terminal
  const isAuthentic = AUTHENTIC_LOG_PATTERNS.some(pat => 
    message.toLowerCase().startsWith(pat.toLowerCase())
  );
  if (!isAuthentic) {
    console.warn(`Blocked non-authentic log attempt: message="${message}"`);
    return;
  }

  const line = document.createElement('div');
  line.className = 'terminal-line';
  
  const now = new Date();
  const timeStr = now.toTimeString().split(' ')[0];
  const spanName = component.toLowerCase().replace(/ /g, '_');
  const tagClass = getCompTagClass(component);
  
  // Map pseudo targets to authentic crate modules
  let finalTarget = target;
  if (target.startsWith('slim_dataplane::')) {
    const sub = target.substring('slim_dataplane::'.length);
    if (sub.startsWith('session')) {
      finalTarget = 'slim_session::' + sub.substring('session::'.length || 0);
    } else if (sub === 'service') {
      finalTarget = 'slim_service::service';
    } else if (sub === 'datapath') {
      finalTarget = 'slim_datapath::datapath';
    } else if (sub === 'system') {
      finalTarget = 'slim_service::system';
    } else if (sub === 'controller') {
      finalTarget = 'slim_controller::controller';
    } else {
      finalTarget = 'slim_' + sub;
    }
  }
  
  line.innerHTML = `<span class="log-time">${timeStr}</span> <span class="log-span ${tagClass}">${spanName}</span> <span class="log-target">${finalTarget}</span> <span class="log-msg">${message}</span>`;
  
  terminalBody.appendChild(line);
  terminalBody.scrollTop = terminalBody.scrollHeight;
}

// Bind Clear Logs functionality once loaded
document.addEventListener('DOMContentLoaded', () => {
  const btnClear = document.getElementById('btn-clear-term');
  if (btnClear) {
    btnClear.addEventListener('click', () => {
      const terminalBody = document.getElementById('terminal-body');
      if (terminalBody) {
        terminalBody.innerHTML = '';
        logToTerminal('System', 'info', 'slim_dataplane::system', 'tracing logs cleared.');
      }
    });
  }
});
