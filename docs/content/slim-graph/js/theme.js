/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Sync visualizer theme with the parent MkDocs Material palette. */
(function () {
  var GRID_STROKES = {
    light: "rgba(2, 81, 175, 0.08)",
    dark: "rgba(255, 255, 255, 0.04)"
  };

  function resolveTheme() {
    try {
      var scheme = window.parent.document.documentElement.getAttribute("data-md-color-scheme");
      if (scheme === "slate") {
        return "dark";
      }
      if (scheme === "default") {
        return "light";
      }
    } catch (_error) {
      /* Standalone open or cross-origin parent — fall back locally. */
    }

    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }

  function updateGridPattern(theme) {
    var gridPath = document.querySelector("#grid path");
    if (gridPath) {
      gridPath.setAttribute("stroke", GRID_STROKES[theme] || GRID_STROKES.light);
    }
  }

  function applyTheme(theme) {
    document.documentElement.setAttribute("data-theme", theme);
    updateGridPattern(theme);
  }

  function syncTheme() {
    applyTheme(resolveTheme());
  }

  window.slimGraphSyncTheme = syncTheme;

  syncTheme();

  try {
    var observer = new MutationObserver(syncTheme);
    observer.observe(window.parent.document.documentElement, {
      attributes: true,
      attributeFilter: ["data-md-color-scheme"]
    });
  } catch (_error) {
    window.matchMedia("(prefers-color-scheme: dark)").addEventListener("change", syncTheme);
  }
})();
