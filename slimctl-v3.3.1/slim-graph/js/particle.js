// Particle Graphic Simulation Class
const PARTICLE_REFERENCE_SPEED = 0.02;
const PARTICLE_SPEED_SCALE = 1.1;
const PARTICLE_REFERENCE_LENGTH = 200;

function getParticlePixelsPerSecond(speed) {
  return (PARTICLE_REFERENCE_LENGTH / pathDuration)
    * (speed / PARTICLE_REFERENCE_SPEED)
    * PARTICLE_SPEED_SCALE;
}

class Particle {
  constructor({ pathId, color, size = 6, speed = 0.015, type = 'dot', onComplete, reverse = false, startDistance = null }) {
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

    this.length = this.path.getTotalLength();
    if (startDistance !== null) {
      this.distance = Math.max(0, Math.min(this.length, startDistance));
    } else {
      this.distance = reverse ? this.length : 0;
    }
    
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
    const dist = Math.max(0, Math.min(this.length, this.distance));
    const pt = this.path.getPointAtLength(dist);
    this.el.setAttribute('transform', `translate(${pt.x}, ${pt.y})`);
  }

  update(elapsed) {
    const deltaDistance = getParticlePixelsPerSecond(this.speed) * (elapsed / 1000);

    if (this.reverse) {
      this.distance -= deltaDistance;
      if (this.distance <= 0) {
        this.distance = 0;
        this.updatePosition();
        this.destroy();
        if (this.onComplete) this.onComplete();
        return false;
      }
    } else {
      this.distance += deltaDistance;
      if (this.distance >= this.length) {
        this.distance = this.length;
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
function spawn2DParticle(pathId, color, size, speed, type, onComplete, reverse = false, startDistance = null) {
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
    reverse,
    startDistance
  });
  particles.push(p);
  return p;
}

let staggerSpawnTimeouts = [];

function clearStaggerSpawnTimeouts() {
  staggerSpawnTimeouts.forEach((id) => clearTimeout(id));
  staggerSpawnTimeouts = [];
}

function spawnStaggeredReverseParticles(pathId, color, size, speed, count, onAllComplete) {
  const path = document.getElementById(pathId);
  if (!path) {
    if (onAllComplete) onAllComplete();
    return;
  }

  const pathLen = path.getTotalLength();
  const pixelsPerSecond = getParticlePixelsPerSecond(speed);
  const launchIntervalMs = Math.max(
    220,
    Math.round((pathLen / count / pixelsPerSecond) * 1000)
  );
  let done = 0;

  for (let i = 0; i < count; i++) {
    const timeoutId = setTimeout(() => {
      spawn2DParticle(pathId, color, size, speed, 'dot', () => {
        done++;
        if (done === count && onAllComplete) {
          onAllComplete();
        }
      }, true, pathLen);
    }, launchIntervalMs * i);
    staggerSpawnTimeouts.push(timeoutId);
  }
}
