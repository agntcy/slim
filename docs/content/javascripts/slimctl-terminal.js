/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Home-page slimctl terminal: scripted demos + slimctl try-it shell. */
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
    var inputForm = section.querySelector(".slimctl-terminal-input");
    var input = section.querySelector(".slimctl-terminal-command");
    var tryBtn = section.querySelector('[data-mode-switch="try"]');
    var titleEl = section.querySelector(".slimctl-terminal-title");
    var promptEl = section.querySelector(".slimctl-terminal-prompt");
    var introEls = section.querySelectorAll(".slimctl-terminal-intro[data-intro-level]");

    if (!terminal || !output) {
      return null;
    }

    section.setAttribute("data-slimctl-bound", "1");

    var state = {
      mode: "demo",
      demoLevel: "message",
      nodeRunning: false,
      tryHistory: [],
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

    function updateActionChrome() {
      var activeLevel = state.mode === "try" ? "try" : state.demoLevel;
      introEls.forEach(function (el) {
        el.hidden = el.getAttribute("data-intro-level") !== activeLevel;
      });
      section.querySelectorAll("[data-demo-level]").forEach(function (btn) {
        btn.classList.toggle(
          "is-active",
          state.mode === "demo" &&
            btn.getAttribute("data-demo-level") === state.demoLevel
        );
      });
      if (tryBtn) {
        tryBtn.classList.toggle("is-active", state.mode === "try");
      }
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
      updateActionChrome();
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

    function updateTryChrome() {
      var data = getData();
      var titles = data.demoTitles || {};
      if (titleEl) {
        titleEl.textContent = titles.node || "user@slim:~";
      }
      if (promptEl) {
        promptEl.textContent = "user@slim:~$";
      }
      if (input) {
        input.setAttribute("aria-label", "Enter a slimctl command");
      }
    }

    function setMode(mode) {
      clearDemoTimer();
      state.mode = mode;
      terminal.setAttribute("data-mode", mode);

      if (mode === "try") {
        state.demoRunning = false;
        if (state.demoObserver) {
          state.demoObserver.disconnect();
          state.demoObserver = null;
        }
        output.innerHTML = "";
        if (inputForm) {
          inputForm.hidden = false;
        }
        updateTryChrome();
        updateActionChrome();
        if (input) {
          input.focus();
        }
        appendTryLine("Explore the CLI — type a command or enter help for suggestions.", "muted");
      } else {
        if (inputForm) {
          inputForm.hidden = true;
        }
        output.innerHTML = "";
        updateDemoChrome();
        startDemoObserver();
      }
    }

    function setDemoLevel(level) {
      if (state.mode === "try") {
        state.demoLevel = level;
        setMode("demo");
        return;
      }
      if (state.demoLevel === level) {
        return;
      }
      state.demoLevel = level;
      updateDemoChrome();
      resetDemo();
    }

    function appendTryLine(text, kind) {
      var className = "slimctl-terminal-out";
      var prefix = "";
      if (kind === "muted") {
        className = "slimctl-terminal-muted";
      } else if (kind === "err") {
        className = "slimctl-terminal-err";
      } else if (kind === "command") {
        className = "slimctl-terminal-cmd";
        prefix = "$ ";
      }
      output.innerHTML +=
        '<span class="' + className + '">' + prefix + escapeHtml(text) + "</span>\n";
      output.scrollTop = output.scrollHeight;
    }

    function tokenize(line) {
      var tokens = [];
      var current = "";
      var inQuote = false;
      var quote = "";

      for (var i = 0; i < line.length; i++) {
        var ch = line[i];
        if (inQuote) {
          if (ch === quote) {
            inQuote = false;
            tokens.push(current);
            current = "";
          } else {
            current += ch;
          }
          continue;
        }
        if (ch === '"' || ch === "'") {
          inQuote = true;
          quote = ch;
          continue;
        }
        if (/\s/.test(ch)) {
          if (current) {
            tokens.push(current);
            current = "";
          }
          continue;
        }
        current += ch;
      }
      if (current) {
        tokens.push(current);
      }
      return tokens;
    }

    function requireNode() {
      if (!state.nodeRunning) {
        appendTryLine(
          "error: SLIM node is not running. Try: slimctl slim start",
          "err"
        );
        return false;
      }
      return true;
    }

    function handleCliTryInput(line) {
      var trimmed = line.trim();
      var lower = trimmed.toLowerCase();
      var data = getData();

      appendTryLine(trimmed, "command");
      state.tryHistory.push(trimmed);

      if (lower === "help") {
        appendTryLine(data.helpText || "Enter a slimctl command.");
        return;
      }
      if (lower === "clear") {
        output.innerHTML = "";
        return;
      }

      var tokens = tokenize(trimmed);
      if (tokens[0] !== "slimctl") {
        appendTryLine("command not found: " + trimmed, "err");
        return;
      }

      if (tokens.length === 1 || (tokens.length === 2 && tokens[1] === "--help")) {
        appendTryLine(data.slimctlHelp || "slimctl help");
        return;
      }

      var sub = tokens[1];

      if (sub === "slim" && (tokens.length === 2 || tokens[2] === "--help")) {
        appendTryLine(data.slimSlimHelp || "slimctl slim start");
        return;
      }

      if (sub === "slim" && tokens[2] === "start") {
        state.nodeRunning = true;
        appendTryLine(
          "INFO slim-data-plane: dataplane listening on 0.0.0.0:46357\n" +
            "INFO slim-controller: controller API on 0.0.0.0:46358"
        );
        return;
      }

      if (sub === "controller") {
        if (!requireNode()) {
          return;
        }
        if (tokens[2] === "node" && tokens[3] === "list") {
          appendTryLine(data.controllerNodeList || "(no nodes)");
          return;
        }
        if (tokens[2] === "route" && tokens[3] === "list") {
          appendTryLine(data.controllerRouteList || "(no routes)");
          return;
        }
        if (tokens[2] === "route" && tokens[3] === "add") {
          var routeName = tokens[4];
          if (!routeName) {
            appendTryLine(
              "error: route name required. Usage: slimctl controller route add <name> via <node-id> --node-id <id>",
              "err"
            );
            return;
          }
          appendTryLine("Route added: " + routeName);
          return;
        }
        if (tokens[2] === "route" && tokens[3] === "del") {
          var deleteRoute = tokens[4];
          if (!deleteRoute) {
            appendTryLine(
              "error: route name required. Usage: slimctl controller route del <name> via <node-id> --node-id <id>",
              "err"
            );
            return;
          }
          appendTryLine("Route deleted: " + deleteRoute);
          return;
        }
        appendTryLine(
          'slimctl controller: unknown subcommand. Try "slimctl controller node list"',
          "err"
        );
        return;
      }

      if (sub === "node") {
        if (!requireNode()) {
          return;
        }
        if (tokens[2] === "connection" && tokens[3] === "list") {
          appendTryLine(data.nodeConnectionList || "(no connections)");
          return;
        }
        if (tokens[2] === "route" && tokens[3] === "list") {
          appendTryLine(data.nodeRouteList || "(no routes)");
          return;
        }
        if (tokens[2] === "route" && tokens[3] === "add") {
          var nodeRouteName = tokens[4];
          if (!nodeRouteName) {
            appendTryLine(
              "error: route name required. Usage: slimctl node route add <name> via <config>",
              "err"
            );
            return;
          }
          appendTryLine("Route added: " + nodeRouteName);
          return;
        }
        appendTryLine('slimctl node: unknown subcommand. Try "slimctl node connection list"', "err");
        return;
      }

      appendTryLine('slimctl: unknown command "' + sub + '". Try: slimctl --help', "err");
    }

    function handleTryInput(line) {
      if (!line.trim()) {
        appendTryLine("", "command");
        return;
      }
      handleCliTryInput(line);
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
      if (state.mode !== "demo") {
        return;
      }

      var script = getActiveScript();
      var doneLines = [];
      var stepIndex = 0;
      state.demoRunning = true;

      function finishStep() {
        stepIndex++;
        runStep();
      }

      function runStep() {
        if (state.mode !== "demo") {
          state.demoRunning = false;
          return;
        }

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
            if (state.mode !== "demo") {
              return;
            }
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
            if (entry.isIntersecting && state.mode === "demo") {
              resetDemo();
            }
          });
        },
        { threshold: 0.45 }
      );
      state.demoObserver.observe(output);
    }

    if (tryBtn) {
      tryBtn.addEventListener("click", function () {
        state.demoLevel = "node";
        setMode("try");
      });
    }

    section.querySelectorAll("[data-demo-level]").forEach(function (btn) {
      btn.addEventListener("click", function () {
        setDemoLevel(btn.getAttribute("data-demo-level") || "message");
      });
    });

    if (inputForm && input) {
      inputForm.addEventListener("submit", function (event) {
        event.preventDefault();
        var value = input.value;
        input.value = "";
        handleTryInput(value);
      });
    }

    updateDemoChrome();
    setMode("demo");

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
