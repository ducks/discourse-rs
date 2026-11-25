// A badge's page (/badges/:id/:slug) with more grants than its first
// page: BadgesShowController#loadMore. When the sentinel scrolls into view
// the page is asked for again at the next offset and its grants are
// appended; once none are left the spinner goes and the "others" link
// (with ?username=) shows.
(function () {
  "use strict";

  if (!Discourse.once("badge-page")) {
    return;
  }

  function loadMore(root, sentinel) {
    if (sentinel.dataset.loading || !sentinel.dataset.nextOffset) {
      return;
    }
    sentinel.dataset.loading = "1";
    var params = new URLSearchParams(location.search);
    params.set("offset", sentinel.dataset.nextOffset);
    fetch(location.pathname + "?" + params.toString(), { credentials: "same-origin" })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var granted = root.querySelector(".badges-granted");
        doc.querySelectorAll(".show-badge .badges-granted > .badge-info").forEach(function (row) {
          granted.appendChild(document.importNode(row, true));
        });
        var fresh = doc.querySelector(".show-badge .load-more-sentinel");
        delete sentinel.dataset.loading;
        if (fresh && fresh.dataset.nextOffset) {
          sentinel.dataset.nextOffset = fresh.dataset.nextOffset;
          return;
        }
        delete sentinel.dataset.nextOffset;
        var others = doc.querySelector("#show-others-with-badge-link");
        if (others) {
          root.querySelector(".user-badges").appendChild(
            document.importNode(others.parentNode, true)
          );
        }
        var spinner = root.querySelector(".loading-container");
        if (spinner) {
          spinner.className = "loading-container";
          spinner.removeAttribute("data-loading");
          spinner.innerHTML = "";
        }
      });
  }

  Discourse.onPage(function () {
    var root = document.querySelector(".show-badge");
    var sentinel = root && root.querySelector(".load-more-sentinel[data-next-offset]");
    if (!sentinel || !("IntersectionObserver" in window)) {
      return;
    }
    var observer = new IntersectionObserver(function (entries) {
      if (entries.some(function (e) { return e.isIntersecting; })) {
        loadMore(root, sentinel);
      }
    });
    observer.observe(sentinel);
    return function () {
      observer.disconnect();
    };
  });
})();
