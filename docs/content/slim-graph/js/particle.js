// Particle Graphic Simulation Class
class Particle {
  constructor({ pathId, color, size = 6, speed = 0.015, type = 'dot', onComplete, reverse = false }) {
    this.pathId = pathId;
    this.path = document.getElementById(pathId);
    if (!this.path) {
      console.error(`Path not found: ${pathId}`);
      return;
    }
    this.color = color;
    this.size = size;
    this.speed = speed;
    this.type = type;
    this.onComplete = onComplete;
    this.reverse = reverse;
    
    this.progress = reverse ? 1 : 0;
    this.length = this.path.getTotalLength();
    
    // Create element
    this.el = document.createElementNS('http://www.w3.org/2000/svg', 'g');
    this.el.setAttribute('class', 'particle-node');
    
    const createSVG = (tag, attrs) => {
      const e = document.createElementNS('http://www.w3.org/2000/svg', tag);
      for (const [k, v] of Object.entries(attrs)) {
        e.setAttribute(k, v);
      }
      return e;
    };

    if (type === 'lock') {
      this.el.appendChild(createSVG('circle', { r: '12', style: `fill: ${color};`, opacity: '0.2' }));
      this.el.appendChild(createSVG('path', { d: 'M-5,-1 L-5,6 L5,6 L5,-1 Z', style: `fill: ${color};` }));
      this.el.appendChild(createSVG('path', { d: 'M-3,-1 L-3,-5 C-3,-7 3,-7 3,-5 L3,-1', fill: 'none', style: `stroke: ${color};`, 'stroke-width': '1.8', 'stroke-linecap': 'round' }));
    } else if (type === 'protobuf') {
      this.el.appendChild(createSVG('circle', { r: '12', style: `fill: ${color};`, opacity: '0.2' }));
      this.el.appendChild(createSVG('polygon', { points: '0,-8 7,-4 7,4 0,8 -7,4 -7,-4', style: `fill: ${color};` }));
    } else {
      this.el.appendChild(createSVG('circle', { r: size + 4, style: `fill: ${color};`, opacity: '0.3' }));
      this.el.appendChild(createSVG('circle', { r: size, style: `fill: ${color};` }));
    }
    
    document.getElementById('particles-group').appendChild(this.el);
    this.updatePosition();
  }

  updatePosition() {
    const dist = this.progress * this.length;
    const pt = this.path.getPointAtLength(dist);
    this.el.setAttribute('transform', `translate(${pt.x}, ${pt.y})`);
  }

  update(elapsed) {
    // Calculate progress change based on elapsed time, standard speed, and pathDuration setting.
    const speedFactor = 1.2 / pathDuration;
    const frameRatio = elapsed / 16.67;
    const deltaProgress = this.speed * frameRatio * speedFactor;

    if (this.reverse) {
      this.progress -= deltaProgress;
      if (this.progress <= 0) {
        this.progress = 0;
        this.updatePosition();
        this.destroy();
        if (this.onComplete) this.onComplete();
        return false;
      }
    } else {
      this.progress += deltaProgress;
      if (this.progress >= 1.0) {
        this.progress = 1.0;
        this.updatePosition();
        this.destroy();
        if (this.onComplete) this.onComplete();
        return false;
      }
    }
    this.updatePosition();
    return true;
  }

  destroy() {
    this.el.remove();
  }
}

// Particle Spawning Helper
function spawn2DParticle(pathId, color, size, speed, type, onComplete, reverse = false) {
  if (isPaused) return null;
  
  const line = document.getElementById(pathId);
  if (line) {
    if (color === 'var(--color-teal)') {
      line.classList.add('active-crypto');
    } else if (color === 'var(--color-purple)') {
      line.classList.add('active-rpc');
    } else if (color === 'var(--color-pink)') {
      line.classList.add('active-control');
    } else {
      line.classList.add('active');
    }
  }

  const p = new Particle({
    pathId,
    color,
    size,
    speed,
    type,
    onComplete: () => {
      if (line) {
        line.classList.remove('active', 'active-crypto', 'active-rpc', 'active-control');
      }
      if (onComplete) onComplete();
    },
    reverse
  });
  particles.push(p);
  return p;
}
