/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

(function () {
  const TOOLTIP_OFFSET = 14;
  const STEP_TOOLTIP_WIDTH = 220;
  const STEP_TOOLTIP_GAP = 8;
  let portalEl = null;
  let stepTooltipActive = false;

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
    stepTooltipActive = false;
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

  function setIframeHeight(iframe, height) {
    if (!iframe || !height) {
      return;
    }
    iframe.style.height = Math.max(height, 320) + 'px';
    iframe.dataset.resized = 'true';
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
        if (stepTooltipActive) {
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

      stepTooltipActive = true;
      requestAnimationFrame(() => {
        positionStepPortal(iframe, data.anchorRect || { left: 0, top: 0, width: 0, height: 0 });
      });
      return;
    }

    if (data.type !== 'slim-graph-tooltip') {
      return;
    }

    if (!data.visible) {
      if (!stepTooltipActive) {
        hidePortal();
      }
      return;
    }

    stepTooltipActive = false;
    const portal = ensurePortal();
    portal.classList.remove('slim-graph-tooltip-portal--step');
    const titleEl = portal.querySelector('.slim-graph-tooltip-portal__title');
    const descEl = portal.querySelector('.slim-graph-tooltip-portal__desc');

    if (titleEl) {
      titleEl.textContent = data.title || '';
    }
    if (descEl) {
      descEl.textContent = data.desc || '';
    }

    positionPortal(iframe, data.x, data.y);
  });

  window.addEventListener('scroll', hidePortal, true);
  window.addEventListener('resize', hidePortal);

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
