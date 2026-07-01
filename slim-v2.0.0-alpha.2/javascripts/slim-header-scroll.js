/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Collapse the nav tab row on scroll; keep the logo/search header visible. */

(function () {
  var tabs = document.querySelector('[data-md-component="tabs"]');
  if (!tabs) {
    return;
  }

  var threshold = 32;
  var collapsed = null;
  var ticking = false;

  function update() {
    ticking = false;
    var shouldCollapse = window.scrollY > threshold;
    if (shouldCollapse === collapsed) {
      return;
    }
    collapsed = shouldCollapse;
    document.documentElement.classList.toggle("slim-tabs-collapsed", shouldCollapse);
    tabs.setAttribute("aria-hidden", shouldCollapse ? "true" : "false");
  }

  function onScroll() {
    if (!ticking) {
      ticking = true;
      window.requestAnimationFrame(update);
    }
  }

  update();
  window.addEventListener("scroll", onScroll, { passive: true });
})();
