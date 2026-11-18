/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* RepoBeats-style GitHub activity panel (data from slim-repobeats-data.js). */
document$.subscribe(function () {
  document.querySelectorAll("[data-slim-repobeats]").forEach(function (root) {
    if (root.dataset.slimRepobeatsInit === "true") {
      return;
    }
    root.dataset.slimRepobeatsInit = "true";

    var repo = root.dataset.repo || "agntcy/slim";
    var panel = root.querySelector(".slim-repobeats");
    if (!panel) {
      return;
    }

    var data = window.__SLIM_REPOBEATS_DATA__;
    if (!data) {
      showRepobeatsError(panel, repo);
      return;
    }

    if (data.embedId) {
      panel.innerHTML =
        '<img class="slim-repobeats__embed" src="https://repobeats.axiom.co/api/embed/' +
        encodeURIComponent(data.embedId) +
        '.svg" alt="GitHub repository metrics" decoding="async" loading="lazy" width="814" height="318" />';
      panel.removeAttribute("aria-busy");
      return;
    }

    renderRepobeatsPanel(panel, data);
  });
});

function showRepobeatsError(panel, repo) {
  panel.innerHTML =
    '<p class="slim-repobeats__error">Could not load repository metrics. ' +
    '<a href="https://github.com/' +
    repo +
    '/pulse?period=monthly" target="_blank" rel="noopener noreferrer">View activity on GitHub</a>.</p>';
  panel.removeAttribute("aria-busy");
}

function renderRepobeatsPanel(panel, data) {
  var recentCommits = data.recentCommits;
  if (!recentCommits && data.commitBars && data.commitBars.length) {
    recentCommits = data.commitBars.slice(-5).reduce(function (sum, value) {
      return sum + value;
    }, 0);
  }

  panel.innerHTML =
    renderSummaryRow(
      "commits",
      recentCommits + " Contributions in the Last 30 Days",
      data.commitBars || [],
      "rgba(236, 72, 153, 1)"
    ) +
    renderSummaryRow(
      "prs",
      data.openPrs + " Pull Requests Opened",
      data.commitBars || [],
      "rgba(123, 88, 201, 1)"
    ) +
    '<div class="slim-repobeats__split">' +
    renderSplitCard(
      "issues",
      "Issues",
      data.issuesOpened + " opened",
      data.issuesClosed + " closed",
      data.issueWeeks || [],
      "rgba(64, 108, 196, 1)",
      "rgba(34, 197, 94, 1)"
    ) +
    renderSplitCard(
      "pulls",
      "Pull Requests",
      data.prsOpened + " opened",
      data.prsMerged + " merged",
      data.prWeeks || [],
      "rgba(123, 88, 201, 1)",
      "rgba(34, 197, 94, 1)"
    ) +
    "</div>" +
    renderContributors(data.contributors || [], data.repo || "agntcy/slim");
  panel.removeAttribute("aria-busy");
}

function renderSummaryRow(kind, title, values, color) {
  return (
    '<div class="slim-repobeats__row slim-repobeats__row--' +
    kind +
    '">' +
    '<div class="slim-repobeats__row-head">' +
    '<span class="slim-repobeats__icon slim-repobeats__icon--' +
    kind +
    '" style="color:' +
    color +
    '"></span>' +
    '<span class="slim-repobeats__title">' +
    escapeHtml(title) +
    "</span>" +
    "</div>" +
    renderBars(values, color) +
    "</div>"
  );
}

function renderSplitCard(kind, label, opened, closed, values, openColor, closedColor) {
  return (
    '<div class="slim-repobeats__card slim-repobeats__card--' +
    kind +
    '">' +
    '<div class="slim-repobeats__card-head">' +
    '<span class="slim-repobeats__card-label">' +
    escapeHtml(label) +
    "</span>" +
    "</div>" +
    '<div class="slim-repobeats__card-stats">' +
    '<span class="slim-repobeats__card-stat" style="color:' +
    openColor +
    '">' +
    escapeHtml(opened) +
    "</span>" +
    '<span class="slim-repobeats__card-stat" style="color:' +
    closedColor +
    '">' +
    escapeHtml(closed) +
    "</span>" +
    "</div>" +
    renderBars(values, openColor, "slim-repobeats__bars--compact") +
    "</div>"
  );
}

function renderContributors(contributors, repo) {
  var maxVisible = 12;

  if (!contributors.length) {
    return (
      '<div class="slim-repobeats__contributors">' +
      '<span class="slim-repobeats__contributors-label">Top contributors</span>' +
      '<img class="slim-repobeats__contrib" src="https://contrib.rocks/image?repo=' +
      encodeURIComponent(repo) +
      "&amp;max=" +
      maxVisible +
      "&amp;columns=" +
      maxVisible +
      '" alt="Top contributors to ' +
      repo +
      '" decoding="async" loading="lazy" />' +
      "</div>"
    );
  }

  var items = contributors
    .slice(0, maxVisible)
    .map(function (contributor) {
      return (
        '<img class="slim-repobeats__avatar" src="' +
        escapeHtml(contributor.avatar_url) +
        '" alt="' +
        escapeHtml(contributor.login) +
        '" title="' +
        escapeHtml(contributor.login) +
        '" loading="lazy" width="32" height="32" />'
      );
    })
    .join("");

  return (
    '<div class="slim-repobeats__contributors">' +
    '<span class="slim-repobeats__contributors-label">Top contributors</span>' +
    '<div class="slim-repobeats__contributor-list">' +
    items +
    "</div>" +
    "</div>"
  );
}

function renderBars(values, color, extraClass) {
  var bars = normalizeBars(values)
    .map(function (level) {
      return (
        '<span class="slim-repobeats__bar" style="--level:' +
        level +
        ";--bar-color:" +
        color +
        '"></span>'
      );
    })
    .join("");

  return (
    '<div class="slim-repobeats__bars' +
    (extraClass ? " " + extraClass : "") +
    '" aria-hidden="true">' +
    bars +
    "</div>"
  );
}

function normalizeBars(values) {
  var max = 0;
  values.forEach(function (value) {
    if (value > max) {
      max = value;
    }
  });
  if (!max) {
    return values.map(function () {
      return 0.08;
    });
  }
  return values.map(function (value) {
    return Math.max(0.08, value / max);
  });
}

function escapeHtml(value) {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}
