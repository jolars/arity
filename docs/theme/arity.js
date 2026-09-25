// Adapt mdBook's shared controls without maintaining a copy of its runtime.
(function () {
  "use strict";

  // mdBook's old Firefox workaround disables the back/forward cache.
  window.onunload = null;

  const toggle = document.getElementById("mdbook-sidebar-toggle");
  const checkbox = document.getElementById("mdbook-sidebar-toggle-anchor");
  toggle.addEventListener("click", function () {
    checkbox.checked = !checkbox.checked;
    checkbox.dispatchEvent(new Event("change"));
  });

  function enhanceContent() {
    // Header navigation is populated by mdBook at DOMContentLoaded.
    document.querySelectorAll("a.chapter-fold-toggle").forEach(function (link) {
      const item = link.closest("li");
      const button = document.createElement("button");
      button.type = "button";
      button.className = link.className;
      button.innerHTML = link.innerHTML;
      const heading = link.previousElementSibling.textContent.trim();
      button.setAttribute("aria-label", "Toggle sections under " + heading);
      function syncExpanded() {
        button.setAttribute(
          "aria-expanded",
          item.classList.contains("expanded"),
        );
      }
      syncExpanded();
      button.addEventListener("click", function () {
        item.classList.toggle("expanded");
        syncExpanded();
      });
      // mdBook also expands the current section as the reader scrolls.
      new MutationObserver(syncExpanded).observe(item, {
        attributes: true,
        attributeFilter: ["class"],
      });
      link.replaceWith(button);
    });

    // Tables and code can overflow only after resizing or opening <details>.
    // Keep these regions reachable without relying on the initial layout.
    document
      .querySelectorAll(".table-wrapper, pre > code")
      .forEach(function (el) {
        el.tabIndex = 0;
      });
  }

  if (document.readyState === "complete") {
    enhanceContent();
  } else {
    // Deferred scripts run before mdBook's DOMContentLoaded handlers.
    document.addEventListener("DOMContentLoaded", enhanceContent);
  }
})();
