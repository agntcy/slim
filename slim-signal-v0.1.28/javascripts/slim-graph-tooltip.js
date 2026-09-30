/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

(function () {
  const TOOLTIP_OFFSET = 14;
  const STEP_TOOLTIP_WIDTH = 220;
  const STEP_TOOLTIP_GAP = 8;
  let portalEl = null;
  let nodeTooltipActive = false;
  let stepTooltipActive = false;
  let lastNodeTooltip = null;
  let lastStepAnchorRect = null;

  function getIframe() {
    return document.querySelector('.slim-graph-frame');
  }

  function ensurePortal() {
    if (portalEl) {
      return portalEl;
    }

    portalEl = document.createElement('div');
    portalEl.id = 'slim-graph-tooltip-portal';
    portalEl.className = 'slim-graph-tooltip-portal';
    portalEl.hidden = true;
    portalEl.innerHTML =
      '<div class="slim-graph-tooltip-portal__title"></div>' +
      '<div class="slim-graph-tooltip-portal__desc"></div>';
    document.body.appendChild(portalEl);
    return portalEl;
  }

  function hidePortal() {
    if (portalEl) {
      portalEl.hidden = true;
      portalEl.classList.remove('slim-graph-tooltip-portal--step');
    }
    nodeTooltipActive = false;
    stepTooltipActive = false;
    lastNodeTooltip = null;
    lastStepAnchorRect = null;
  }

  function positionPortal(iframe, clientX, clientY) {
    const portal = ensurePortal();
    const iframeRect = iframe.getBoundingClientRect();

    portal.style.width = '';
    portal.style.left = iframeRect.left + clientX + TOOLTIP_OFFSET + 'px';
    portal.style.top = iframeRect.top + clientY + TOOLTIP_OFFSET + 'px';
    portal.hidden = false;
  }

  function positionStepPortal(iframe, anchorRect) {
    const portal = ensurePortal();
    const iframeRect = iframe.getBoundingClientRect();
    const tooltipWidth = STEP_TOOLTIP_WIDTH;

    portal.style.width = tooltipWidth + 'px';
    portal.hidden = false;

    let left =
      iframeRect.left + anchorRect.left + anchorRect.width / 2 - tooltipWidth / 2;
    left = Math.max(8, Math.min(left, window.innerWidth - tooltipWidth - 8));

    const top =
      iframeRect.top + anchorRect.top + anchorRect.height + STEP_TOOLTIP_GAP;

    portal.style.left = left + 'px';
    portal.style.top = top + 'px';
  }

  function repositionActivePortal() {
    const iframe = getIframe();
    if (!iframe || !portalEl || portalEl.hidden) {
      return;
    }

    if (stepTooltipActive && lastStepAnchorRect) {
      positionStepPortal(iframe, lastStepAnchorRect);
      return;
    }

    if (nodeTooltipActive && lastNodeTooltip) {
      positionPortal(iframe, lastNodeTooltip.x, lastNodeTooltip.y);
    }
  }

  function setIframeHeight(iframe, height) {
    if (!iframe || !height) {
      return;
    }
    iframe.style.height = Math.max(height, 320) + 'px';
    iframe.dataset.resized = 'true';
    requestAnimationFrame(repositionActivePortal);
  }

  window.addEventListener('message', (event) => {
    if (event.origin !== window.location.origin) {
      return;
    }

    const data = event.data;
    if (!data || !data.type) {
      return;
    }

    const iframe = getIframe();
    if (!iframe || event.source !== iframe.contentWindow) {
      return;
    }

    if (data.type === 'slim-graph-resize') {
      setIframeHeight(iframe, data.height);
      return;
    }

    if (data.type === 'slim-graph-step-tooltip') {
      if (!data.visible) {
        stepTooltipActive = false;
        lastStepAnchorRect = null;
        if (!nodeTooltipActive) {
          hidePortal();
        }
        return;
      }

      const portal = ensurePortal();
      portal.classList.add('slim-graph-tooltip-portal--step');
      const titleEl = portal.querySelector('.slim-graph-tooltip-portal__title');
      const descEl = portal.querySelector('.slim-graph-tooltip-portal__desc');

      if (titleEl) {
        titleEl.textContent = data.title || '';
      }
      if (descEl) {
        descEl.innerHTML = data.desc || '';
      }

      nodeTooltipActive = false;
      lastNodeTooltip = null;
      stepTooltipActive = true;
      lastStepAnchorRect = data.anchorRect || { left: 0, top: 0, width: 0, height: 0 };
      requestAnimationFrame(() => {
        positionStepPortal(iframe, lastStepAnchorRect);
      });
      return;
    }

    if (data.type !== 'slim-graph-tooltip') {
      return;
    }

    if (!data.visible) {
      nodeTooltipActive = false;
      lastNodeTooltip = null;
      if (!stepTooltipActive) {
        hidePortal();
      }
      return;
    }

    stepTooltipActive = false;
    lastStepAnchorRect = null;
    nodeTooltipActive = true;
    lastNodeTooltip = {
      x: data.x,
      y: data.y,
      title: data.title || '',
      desc: data.desc || ''
    };

    const portal = ensurePortal();
    portal.classList.remove('slim-graph-tooltip-portal--step');
    const titleEl = portal.querySelector('.slim-graph-tooltip-portal__title');
    const descEl = portal.querySelector('.slim-graph-tooltip-portal__desc');

    if (titleEl) {
      titleEl.textContent = lastNodeTooltip.title;
    }
    if (descEl) {
      descEl.textContent = lastNodeTooltip.desc;
    }

    positionPortal(iframe, lastNodeTooltip.x, lastNodeTooltip.y);
  });

  window.addEventListener('scroll', repositionActivePortal, true);
  window.addEventListener('resize', repositionActivePortal);

  function bindIframeListeners() {
    const iframe = getIframe();
    if (!iframe || iframe.dataset.slimGraphTooltipBound === 'true') {
      return;
    }

    iframe.dataset.slimGraphTooltipBound = 'true';
    iframe.addEventListener('mouseleave', hidePortal);
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', bindIframeListeners);
  } else {
    bindIframeListeners();
  }
})();
