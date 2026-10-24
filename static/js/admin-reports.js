// The admin reports index in the browser: DFilterControls filtering the
// listed reports as they are typed (any comma-separated term in a
// report's title or description) and by the group menu, the URL's
// `filter` and `group` kept in step with replaceState, the reset button
// and the no-results state.
(function () {
  "use strict";

  if (!Discourse.once("admin-reports")) {
    return;
  }

  // The index's AdminReports (admin-reports/index.gjs), not the single
  // report (admin-reports/show.gjs), which shares the route's wrapper but
  // has no filter controls: a page swap from the index keeps this script.
  function page() {
    var root = document.querySelector(".admin-contents.admin-reports");
    return root && root.querySelector(".admin-config-area__full-width > .d-filter-controls") ? root : null;
  }

  function terms(value) {
    return value
      .split(",")
      .map(function (term) {
        return term.trim().toLowerCase();
      })
      .filter(Boolean);
  }

  function matches(item, list) {
    if (!list.length) {
      return true;
    }
    var text = [".admin-section-landing-item__title", ".admin-section-landing-item__description"].map(function (sel) {
      var el = item.querySelector(sel);
      return el ? el.textContent.toLowerCase() : "";
    });
    return list.some(function (term) {
      return text.some(function (t) {
        return t.indexOf(term) !== -1;
      });
    });
  }

  function apply(root) {
    var input = root.querySelector(".d-filter-controls__input");
    var select = root.querySelector(".d-filter-controls__dropdown");
    var filter = input.value;
    var group = select.value;
    var list = terms(filter);
    var shown = 0;
    root.querySelectorAll(".admin-reports-group").forEach(function (section) {
      var inGroup = group === "all" || section.dataset.group === group;
      var any = false;
      section.querySelectorAll(".admin-section-landing-item").forEach(function (item) {
        var visible = inGroup && matches(item, list);
        item.style.display = visible ? "" : "none";
        any = any || visible;
      });
      section.style.display = any ? "" : "none";
      if (any) {
        shown += 1;
      }
    });
    var active = filter.length > 0 || group !== "all";
    select.classList.toggle("--active", group !== "all");
    Array.prototype.forEach.call(select.options, function (option) {
      option.classList.toggle("--selected", option.value === group);
    });
    root.querySelector(".d-filter-controls .d-filter-controls__reset").disabled = !active;
    var noResults = root.querySelector(".d-filter-controls__no-results");
    if (shown === 0 && active) {
      if (!noResults) {
        var template = root.querySelector("template.admin-reports-no-results");
        root.querySelector(".d-filter-controls").after(template.content.firstElementChild.cloneNode(true));
      }
    } else if (noResults) {
      noResults.remove();
    }
    var params = new URLSearchParams(location.search);
    if (filter) {
      params.set("filter", filter);
    } else {
      params.delete("filter");
    }
    if (group !== "all") {
      params.set("group", group);
    } else {
      params.delete("group");
    }
    var query = params.toString();
    history.replaceState(history.state, "", location.pathname + (query ? "?" + query : ""));
  }

  // The URL's filters, as the controls start from them.
  Discourse.onPage(function () {
    var root = page();
    if (root) {
      apply(root);
    }
  });

  document.addEventListener("input", function (event) {
    var root = page();
    if (root && event.target.matches(".admin-reports .d-filter-controls__input")) {
      apply(root);
    }
  });

  document.addEventListener("change", function (event) {
    var root = page();
    if (root && event.target.matches(".admin-reports .d-filter-controls__dropdown")) {
      apply(root);
    }
  });

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root || !event.target.closest(".admin-reports .d-filter-controls__reset")) {
      return;
    }
    root.querySelector(".d-filter-controls__input").value = "";
    root.querySelector(".d-filter-controls__dropdown").value = "all";
    apply(root);
  });
})();
