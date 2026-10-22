// EmptyTopicFilter's call to action in a member's empty list footer
// (DiscoveryTopics): "Browse latest topics" goes to the discovery.latest
// route; under unified new, a /new subset with nothing left offers the
// other subset (changeNewListSubset sets the `subset` query param, which
// reloads the list from its first page).
(function () {
  "use strict";

  // Swapped-in pages run this again; the listener stays from the first.
  if (!Discourse.once("empty-topic-filter")) {
    return;
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function go(url) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  document.addEventListener("click", function (event) {
    var button = event.target.closest(".--empty-topic-filter .empty-state__cta .btn");
    if (!button) {
      return;
    }
    event.preventDefault();
    var subset = button.getAttribute("data-subset");
    if (subset) {
      var params = new URLSearchParams(location.search);
      params.delete("page");
      params.set("subset", subset);
      go(location.pathname + "?" + params.toString());
    } else {
      go(basePath() + "/latest");
    }
  });
})();
