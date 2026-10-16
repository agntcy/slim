/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

(function () {
  var PHRASES = [
    "One overlay for all agent protocols",
    "Encrypted messages across any network topology",
    "Reachable by name, without exposing every agent to the internet",
  ];

  var HOLD_MS = 3800;

  function prefersReducedMotion() {
    return (
      window.matchMedia &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    );
  }

  function setActive(root, lines, bars, nextIndex) {
    lines.forEach(function (line, lineIndex) {
      line.classList.toggle("is-active", lineIndex === nextIndex);
    });

    bars.forEach(function (bar, barIndex) {
      var isActive = barIndex === nextIndex;
      var isComplete = barIndex < nextIndex;

      bar.classList.toggle("is-active", isActive);
      bar.classList.toggle("is-complete", isComplete);
      bar.setAttribute("aria-current", isActive ? "true" : "false");

      if (isActive && !prefersReducedMotion()) {
        bar.classList.remove("is-progressing");
        void bar.offsetWidth;
        bar.classList.add("is-progressing");
      } else {
        bar.classList.remove("is-progressing");
      }
    });

    root.setAttribute("data-slim-hero-tagline-index", String(nextIndex));
  }

  function initRotate(root) {
    if (!root || root.getAttribute("data-slim-hero-tagline-bound") === "1") {
      return;
    }

    root.setAttribute("data-slim-hero-tagline-bound", "1");
    root.style.setProperty("--slim-tagline-hold", HOLD_MS + "ms");

    var textSlot = root.querySelector(".slim-hero-tagline-rotate__text");
    if (!textSlot) {
      return;
    }

    var track = document.createElement("span");
    track.className = "slim-hero-tagline-rotate__track";
    track.setAttribute("aria-live", "polite");

    PHRASES.forEach(function (phrase, index) {
      var line = document.createElement("span");
      line.className = "slim-hero-tagline-rotate__phrase";
      if (index === 0) {
        line.classList.add("is-active");
      }
      line.textContent = phrase;
      track.appendChild(line);
    });

    textSlot.textContent = "";
    textSlot.appendChild(track);

    var indicators = document.createElement("div");
    indicators.className = "slim-hero-tagline-rotate__indicators";
    indicators.setAttribute("aria-label", "Tagline progress");

    PHRASES.forEach(function (_phrase, index) {
      var bar = document.createElement("button");
      bar.type = "button";
      bar.className = "slim-hero-tagline-rotate__bar";
      bar.setAttribute("aria-label", "Show tagline " + (index + 1) + " of " + PHRASES.length);
      if (index === 0) {
        bar.classList.add("is-active", "is-progressing");
        bar.setAttribute("aria-current", "true");
      }
      bar.addEventListener("click", function () {
        if (index === activeIndex) {
          return;
        }
        restartCycle(index);
      });
      indicators.appendChild(bar);
    });

    root.appendChild(indicators);

    var lines = track.querySelectorAll(".slim-hero-tagline-rotate__phrase");
    var bars = indicators.querySelectorAll(".slim-hero-tagline-rotate__bar");
    var activeIndex = 0;
    var timer = null;
    var reducedMotion = prefersReducedMotion();

    function scheduleNext() {
      if (reducedMotion || PHRASES.length < 2) {
        return;
      }

      timer = window.setTimeout(function () {
        var nextIndex = (activeIndex + 1) % PHRASES.length;
        restartCycle(nextIndex);
      }, HOLD_MS);
    }

    function restartCycle(nextIndex) {
      if (timer) {
        window.clearTimeout(timer);
        timer = null;
      }

      activeIndex = nextIndex;
      setActive(root, lines, bars, activeIndex);
      scheduleNext();
    }

    if (reducedMotion || PHRASES.length < 2) {
      setActive(root, lines, bars, 0);
      return;
    }

    scheduleNext();

    root._slimHeroTaglineCleanup = function () {
      if (timer) {
        window.clearTimeout(timer);
      }
    };
  }

  function initAll() {
    document.querySelectorAll("[data-slim-hero-tagline]").forEach(initRotate);
  }

  if (typeof document$ !== "undefined") {
    document$.subscribe(initAll);
  } else if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", initAll);
  } else {
    initAll();
  }
})();
