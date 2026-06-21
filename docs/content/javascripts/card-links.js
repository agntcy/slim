/* Copyright AGNTCY Contributors (https://github.com/agntcy) */
/* SPDX-License-Identifier: Apache-2.0 */

/* Grid card enhancements: featured highlight and single-link stretch targets. */
document$.subscribe(function () {
  document.querySelectorAll(".md-typeset .grid.cards > ul > li").forEach(function (li) {
    var links = li.querySelectorAll("a[href]");
    li.classList.toggle("card-single-link", links.length === 1);

    var title = li.querySelector("strong");
    if (title && title.textContent.trim() === "Join the Federation Testbed") {
      li.classList.add("card-featured");
    }
  });

  document.querySelectorAll(".landing-explore__toggle").forEach(function (button) {
    if (button.dataset.exploreBound === "true") {
      return;
    }
    button.dataset.exploreBound = "true";

    button.addEventListener("click", function () {
      var item = button.closest(".landing-explore__item");
      var body = item.querySelector(".landing-explore__body");
      var isOpen = button.getAttribute("aria-expanded") === "true";

      button.setAttribute("aria-expanded", isOpen ? "false" : "true");
      item.classList.toggle("is-open", !isOpen);

      if (isOpen) {
        body.setAttribute("hidden", "");
      } else {
        body.removeAttribute("hidden");
      }
    });
  });
});
