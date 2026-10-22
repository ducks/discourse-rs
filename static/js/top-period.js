// A top list's PeriodChooser (DiscoveryTopics): a details element whose
// body floats under it as select-kit's does; a row goes to the list for
// its period (DiscoveryListController#changePeriod sets the `period`
// query param, which reloads the list from its first page).
(function () {
  "use strict";

  // Swapped-in pages run this again; the listeners stay from the first.
  if (!Discourse.once("top-period")) {
    return;
  }

  var CHOOSER = ".top-lists .period-chooser";

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto angle-icon svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
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

  // The list's URL with the period: its other params kept, its page dropped.
  function periodUrl(period) {
    var params = new URLSearchParams(location.search);
    params.delete("page");
    params.set("period", period);
    return location.pathname + "?" + params.toString();
  }

  function closeAll(except) {
    document.querySelectorAll(CHOOSER + "[open]").forEach(function (chooser) {
      if (chooser !== except) {
        chooser.open = false;
      }
    });
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(CHOOSER)) {
        return;
      }
      var open = chooser.open;
      chooser.classList.toggle("is-expanded", open);
      var caret = chooser.querySelector(".angle-icon");
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : "angle-down");
      }
      var header = chooser.querySelector("summary");
      var body = chooser.querySelector(".select-kit-body");
      if (!open || !body || !window.FloatingUIDOM) {
        return;
      }
      // SelectKit#updateFloatingUiPosition with its verticalOffset of 3;
      // under 451px wide the body spans the screen less 10px a side.
      var ui = window.FloatingUIDOM;
      var small = window.innerWidth <= 450;
      var headerOffset = parseInt(document.documentElement.style.getPropertyValue("--header-offset"), 10) || 0;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = small ? "" : Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
      body.style.width = small ? window.innerWidth - 20 + "px" : "";
      ui.computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [
          ui.flip({ padding: { top: 3 + headerOffset, bottom: 3 }, fallbackStrategy: "initialPlacement" }),
          ui.shift({ limiter: ui.limitShift() }),
          ui.offset(3),
        ],
      }).then(function (result) {
        var x = small ? 10 : Math.round(result.x);
        body.style.transform = "translate(" + x + "px," + Math.round(result.y) + "px)";
      });
    },
    true
  );

  document.addEventListener("click", function (event) {
    var row = event.target.closest(CHOOSER + " .select-kit-row");
    if (row) {
      var chooser = row.closest(CHOOSER);
      chooser.open = false;
      if (row.dataset.value !== chooser.querySelector("summary").dataset.value) {
        go(periodUrl(row.dataset.value));
      }
      return;
    }
    // A click outside closes it.
    closeAll(event.target.closest(CHOOSER));
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeAll(null);
    }
    if (event.key === "Enter" && event.target.closest) {
      var row = event.target.closest(CHOOSER + " .select-kit-row");
      if (row) {
        row.click();
      }
    }
  });
})();
