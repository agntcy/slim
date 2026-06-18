/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

(function () {
  const TOOLTIP_OFFSET = 14;
  let portalEl = null;

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
    }
  }

  function positionPortal(iframe, clientX, clientY) {
    const portal = ensurePortal();
    const iframeRect = iframe.getBoundingClientRect();

    portal.style.left = iframeRect.left + clientX + TOOLTIP_OFFSET + 'px';
    portal.style.top = iframeRect.top + clientY + TOOLTIP_OFFSET + 'px';
    portal.hidden = false;
  }

  window.addEventListener('message', (event) => {
    if (event.origin !== window.location.origin) {
      return;
    }

    const data = event.data;
    if (!data || data.type !== 'slim-graph-tooltip') {
      return;
    }

    const iframe = getIframe();
    if (!iframe || event.source !== iframe.contentWindow) {
      return;
    }

    if (!data.visible) {
      hidePortal();
      return;
    }

    const portal = ensurePortal();
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
