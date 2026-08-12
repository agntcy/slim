/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Collapse the nav tab row on scroll; keep the logo/search header visible. */

(function () {
  var tabs = document.querySelector('[data-md-component="tabs"]');
  if (!tabs) {
    return;
  }

  var collapseAt = 88;
  var expandAt = 16;
  var collapsed = null;
  var ticking = false;

  function shouldCollapse(scrollY) {
    if (collapsed) {
      return scrollY > expandAt;
    }
    return scrollY > collapseAt;
  }

  function update() {
    ticking = false;
    var shouldCollapseNow = shouldCollapse(window.scrollY);
    if (shouldCollapseNow === collapsed) {
      return;
    }
    collapsed = shouldCollapseNow;
    document.documentElement.classList.toggle("slim-tabs-collapsed", shouldCollapseNow);
    tabs.setAttribute("aria-hidden", shouldCollapseNow ? "true" : "false");
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
