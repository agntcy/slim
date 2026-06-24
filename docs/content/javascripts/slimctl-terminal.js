/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Home-page slimctl terminal: scripted Start Node and Send a Message demos. */
(function () {
  var CURSOR = '<span class="slimctl-terminal-cursor">|</span>';
  var TYPE_MS = 32;

  function escapeHtml(text) {
    return text
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");
  }

  function prefersReducedMotion() {
    return (
      window.matchMedia &&
      window.matchMedia("(prefers-reduced-motion: reduce)").matches
    );
  }

  function getData() {
    return window.SlimctlDemoData || {};
  }

  function mountIntroGroup(section) {
    var side = section.querySelector(".slimctl-terminal-side");
    var introGroup = document.getElementById("slimctl-terminal-intros");
    if (!side || !introGroup) {
      return;
    }

    if (introGroup.parentElement === side) {
      introGroup.setAttribute("data-slimctl-mounted", "1");
      return;
    }

    if (introGroup.getAttribute("data-slimctl-mounted") === "1") {
      return;
    }

    side.insertBefore(introGroup, side.firstChild);
    introGroup.setAttribute("data-slimctl-mounted", "1");
  }

  function createTerminal(section) {
    if (section.getAttribute("data-slimctl-bound") === "1") {
      return null;
    }

    mountIntroGroup(section);

    var terminal = section.querySelector(".slimctl-terminal");
    var output = section.querySelector(".slimctl-terminal-output");
    var titleEl = section.querySelector(".slimctl-terminal-title");
    var introEls = section.querySelectorAll(".slimctl-terminal-intro[data-intro-level]");

    if (!terminal || !output) {
      return null;
    }

    section.setAttribute("data-slimctl-bound", "1");

    var state = {
      demoLevel: "node",
      demoTimer: null,
      demoObserver: null,
      demoRunning: false,
    };

    var demoLineMeta = {
      command: { prefix: "$ ", className: "slimctl-terminal-cmd" },
      user: { prefix: "> ", className: "slimctl-terminal-user" },
      agent: { prefix: "● ", className: "slimctl-terminal-agent" },
      tool: { prefix: "→ ", className: "slimctl-terminal-tool" },
      python: { prefix: ">>> ", className: "slimctl-terminal-tool" },
    };

    function getActiveScript() {
      var data = getData();
      if (state.demoLevel === "node") {
        return data.nodeDemoScript || [];
      }
      return data.messageDemoScript || [];
    }

    function updateDemoChrome() {
      var data = getData();
      var titles = data.demoTitles || {};
      if (titleEl) {
        if (state.demoLevel === "message") {
          titleEl.textContent = titles.message || "python";
        } else {
          titleEl.textContent = titles.node || "user@slim:~";
        }
      }

      introEls.forEach(function (el) {
        el.hidden = el.getAttribute("data-intro-level") !== state.demoLevel;
      });

      section.querySelectorAll("[data-demo-level]").forEach(function (btn) {
        btn.classList.toggle(
          "is-active",
          btn.getAttribute("data-demo-level") === state.demoLevel
        );
      });
    }

    function formatDemoLine(step) {
      var meta = demoLineMeta[step.type];
      if (!meta) {
        return null;
      }
      return (
        '<span class="' + meta.className + '">' + meta.prefix + escapeHtml(step.text) + "</span>"
      );
    }

    function clearDemoTimer() {
      if (state.demoTimer) {
        clearTimeout(state.demoTimer);
        state.demoTimer = null;
      }
    }

    function renderDemoBlock(doneLines, partial) {
      var html = doneLines.join("\n");
      if (partial) {
        html += (doneLines.length ? "\n" : "") + partial + CURSOR;
      } else if (state.demoRunning) {
        html += (doneLines.length ? "\n" : "") + CURSOR;
      }
      output.innerHTML = html;
      output.scrollTop = output.scrollHeight;
    }

    function runDemoScript() {
      var script = getActiveScript();
      var doneLines = [];
      var stepIndex = 0;
      state.demoRunning = true;

      function finishStep() {
        stepIndex++;
        runStep();
      }

      function runStep() {
        if (stepIndex >= script.length) {
          state.demoTimer = setTimeout(function () {
            doneLines = [];
            stepIndex = 0;
            runStep();
          }, 0);
          return;
        }

        var step = script[stepIndex];

        if (step.type === "pause") {
          state.demoTimer = setTimeout(finishStep, step.ms || 1500);
          return;
        }

        if (step.type === "output") {
          doneLines.push(
            '<span class="slimctl-terminal-out">' + escapeHtml(step.text) + "</span>"
          );
          renderDemoBlock(doneLines, null);
          state.demoTimer = setTimeout(finishStep, prefersReducedMotion() ? 0 : 400);
          return;
        }

        if (demoLineMeta[step.type]) {
          var lineMeta = demoLineMeta[step.type];
          if (prefersReducedMotion()) {
            doneLines.push(formatDemoLine(step));
            renderDemoBlock(doneLines, null);
            finishStep();
            return;
          }

          var prefixHtml =
            '<span class="' + lineMeta.className + '">' + lineMeta.prefix + "</span>";
          var text = step.text;
          var charIndex = 0;

          function typeChar() {
            var partial =
              prefixHtml +
              '<span class="' +
              lineMeta.className +
              '">' +
              escapeHtml(text.slice(0, charIndex)) +
              "</span>";
            renderDemoBlock(doneLines, partial);
            if (charIndex <= text.length) {
              charIndex++;
              state.demoTimer = setTimeout(typeChar, TYPE_MS);
            } else {
              doneLines.push(formatDemoLine(step));
              finishStep();
            }
          }

          typeChar();
        }
      }

      if (prefersReducedMotion()) {
        script.forEach(function (step) {
          if (demoLineMeta[step.type]) {
            doneLines.push(formatDemoLine(step));
          } else if (step.type === "output") {
            doneLines.push(
              '<span class="slimctl-terminal-out">' + escapeHtml(step.text) + "</span>"
            );
          }
        });
        renderDemoBlock(doneLines, null);
        state.demoRunning = false;
        return;
      }

      runStep();
    }

    function resetDemo() {
      clearDemoTimer();
      output.innerHTML = "";
      runDemoScript();
    }

    function setDemoLevel(level) {
      if (state.demoLevel === level) {
        return;
      }
      state.demoLevel = level;
      updateDemoChrome();
      resetDemo();
    }

    function startDemoObserver() {
      if (state.demoObserver) {
        state.demoObserver.disconnect();
      }
      if (!("IntersectionObserver" in window)) {
        resetDemo();
        return;
      }
      state.demoObserver = new IntersectionObserver(
        function (entries) {
          entries.forEach(function (entry) {
            if (entry.isIntersecting) {
              resetDemo();
            }
          });
        },
        { threshold: 0.45 }
      );
      state.demoObserver.observe(output);
    }

    section.querySelectorAll("[data-demo-level]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        setDemoLevel(btn.getAttribute("data-demo-level") || "node");
      });
    });

    updateDemoChrome();
    startDemoObserver();

    return {
      destroy: function () {
        clearDemoTimer();
        if (state.demoObserver) {
          state.demoObserver.disconnect();
        }
        section.removeAttribute("data-slimctl-bound");
      },
    };
  }

  function initTerminals() {
    document.querySelectorAll(".slimctl-terminal-section").forEach(function (section) {
      createTerminal(section);
    });
  }

  if (typeof document$ !== "undefined") {
    document$.subscribe(initTerminals);
  } else {
    document.addEventListener("DOMContentLoaded", initTerminals);
  }
})();
