/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Sync visualizer theme with the parent MkDocs Material palette. */
(function () {
  function readParentScheme() {
    try {
      var parentDoc = window.parent.document;
      return (
        parentDoc.documentElement.getAttribute("data-md-color-scheme") ||
        parentDoc.body.getAttribute("data-md-color-scheme")
      );
    } catch (_error) {
      return null;
    }
  }

  function resolveTheme() {
    var scheme = readParentScheme();
    if (scheme === "slate") {
      return "dark";
    }
    if (scheme === "default") {
      return "light";
    }

    return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
  }

  function applyTheme(theme) {
    var root = document.documentElement;
    root.setAttribute("data-theme", theme);
    root.style.colorScheme = theme;
  }

  function syncTheme() {
    applyTheme(resolveTheme());
  }

  function markEmbedded() {
    try {
      if (window.parent !== window) {
        document.documentElement.setAttribute("data-embedded", "true");
      }
    } catch (_error) {
      /* cross-origin */
    }
  }

  window.slimGraphSyncTheme = syncTheme;

  markEmbedded();

  syncTheme();

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", syncTheme);
  }

  window.addEventListener("load", syncTheme);

  try {
    var parentRoot = window.parent.document.documentElement;
    var observer = new MutationObserver(syncTheme);
    observer.observe(parentRoot, {
      attributes: true,
      attributeFilter: ["data-md-color-scheme", "class"]
    });

    if (window.parent.document.body) {
      observer.observe(window.parent.document.body, {
        attributes: true,
        attributeFilter: ["data-md-color-scheme", "class"]
      });
    }
  } catch (_error) {
    window
      .matchMedia("(prefers-color-scheme: dark)")
      .addEventListener("change", syncTheme);
  }
})();
