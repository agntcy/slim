/* Open off-site links in a new tab site-wide (nav, content, footer). */
document$.subscribe(function () {
  document.querySelectorAll("a[href]").forEach(function (link) {
    var href = link.getAttribute("href");
    if (!href || href.charAt(0) === "#" || href.indexOf("javascript:") === 0) {
      return;
    }

    var url;
    try {
      url = new URL(href, window.location.href);
    } catch (e) {
      return;
    }

    if (url.protocol === "mailto:" || url.protocol === "tel:") {
      return;
    }

    if (url.origin === window.location.origin) {
      return;
    }

    link.setAttribute("target", "_blank");
    link.setAttribute("rel", "noopener noreferrer");
  });
});
