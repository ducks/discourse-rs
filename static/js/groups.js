// The groups pages: what the Ember groups and group routes do in the
// browser. On /groups the name filter loads the matching groups after
// 500ms (GroupsIndexController#onFilterChanged) and the type menu filters
// by type; on /g/:name the group menu goes to the chosen group, the
// username filter reloads the members after 500ms and a column header
// sorts by it (DTableHeaderToggle). Join and Leave call the group's
// join and leave, then draw the page again.
(function () {
  "use strict";

  if (!Discourse.once("groups")) {
    return;
  }

  var MENUS = ".groups-header-filters-type, .group-dropdown";

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
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

  // The page's URL with these query params set (null drops one), keys
  // sorted.
  function withParams(changes) {
    var params = new URLSearchParams(location.search);
    Object.keys(changes).forEach(function (key) {
      if (changes[key] === null || changes[key] === "") {
        params.delete(key);
      } else {
        params.set(key, changes[key]);
      }
    });
    var keys = Array.from(params.keys()).sort();
    var query = keys
      .map(function (key) {
        return encodeURIComponent(key) + "=" + encodeURIComponent(params.get(key));
      })
      .join("&");
    return location.pathname + (query ? "?" + query : "");
  }

  function basePath() {
    var link = document.querySelector("a.group-box, .group-nav a");
    if (!link) {
      return "";
    }
    var href = link.getAttribute("href");
    var at = href.indexOf("/g/");
    return at > 0 ? href.slice(0, at) : "";
  }

  Discourse.onPage(function () {
    // GroupCard's descriptions with links, inside the card's link as
    // Ember's DOM has them.
    document.querySelectorAll("template.group-description-html").forEach(function (template) {
      template.replaceWith(template.content.cloneNode(true));
    });
    var cleanups = [];
    [
      [".groups-header-filters-name", "filter"],
      [".group-username-filter", "filter"],
    ].forEach(function (pair) {
      var input = document.querySelector(pair[0]);
      if (!input) {
        return;
      }
      var timer = null;
      var onInput = function () {
        clearTimeout(timer);
        timer = setTimeout(function () {
          var change = {};
          change[pair[1]] = input.value;
          go(withParams(change));
        }, 500);
      };
      input.addEventListener("input", onInput);
      cleanups.push(function () {
        clearTimeout(timer);
        input.removeEventListener("input", onInput);
      });
    });
    return function () {
      cleanups.forEach(function (fn) {
        fn();
      });
    };
  });

  // The menus: details elements whose body floats under them as
  // select-kit's does.
  document.addEventListener(
    "toggle",
    function (event) {
      var menu = event.target;
      if (!menu.classList || !menu.matches(MENUS)) {
        return;
      }
      var open = menu.open;
      menu.classList.toggle("is-expanded", open);
      var caret = menu.querySelector(".angle-icon");
      var closed = menu.matches(".group-dropdown") ? "angle-right" : "angle-down";
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : closed).replace(
          "svg-icon fa-width-auto",
          "svg-icon fa-width-auto angle-icon"
        );
      }
      var header = menu.querySelector("summary");
      var body = menu.querySelector(".select-kit-body");
      if (!open || !body || !window.FloatingUIDOM) {
        return;
      }
      var ui = window.FloatingUIDOM;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
      ui.computePosition(header, body, {
        placement: "bottom-start",
        strategy: "fixed",
        middleware: [ui.flip(), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
      }).then(function (result) {
        body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      });
    },
    true
  );

  function membership(button, method, path) {
    button.disabled = true;
    fetch(basePath() + "/groups/" + button.dataset.groupId + path, {
      method: method,
      credentials: "same-origin",
      headers: csrfHeaders(),
    })
      .then(function (r) {
        if (r.ok) {
          go(location.pathname + location.search);
          return;
        }
        return r
          .json()
          .then(function (body) {
            window.alert((body.errors || [r.statusText]).join("\n"));
          })
          .catch(function () {
            window.alert(r.statusText);
          });
      })
      .finally(function () {
        button.disabled = false;
      });
  }

  document.addEventListener("click", function (event) {
    var typeRow = event.target.closest(".groups-header-filters-type .select-kit-row");
    if (typeRow) {
      go(withParams({ type: typeRow.dataset.value || null }));
      return;
    }
    var groupRow = event.target.closest(".group-dropdown .select-kit-row");
    if (groupRow) {
      go(basePath() + "/g/" + encodeURIComponent(groupRow.dataset.value));
      return;
    }
    document.querySelectorAll(".groups-header-filters-type[open], .group-dropdown[open]").forEach(function (menu) {
      if (!menu.contains(event.target)) {
        menu.open = false;
      }
    });
    var join = event.target.closest(".group-index-join");
    if (join) {
      event.preventDefault();
      if (!document.body.getAttribute("hx-headers")) {
        go(basePath() + "/login");
        return;
      }
      membership(join, "PUT", "/join.json");
      return;
    }
    var leave = event.target.closest(".group-index-leave");
    if (leave) {
      event.preventDefault();
      membership(leave, "DELETE", "/leave.json");
      return;
    }
    // DTableHeaderToggle#toggleProperties: ascending only when it was
    // descending on this field.
    var header = event.target.closest(".group-members .directory-table__column-header.sortable");
    if (header) {
      var params = new URLSearchParams(location.search);
      var field = header.dataset.order;
      var asc = params.get("order") === field && params.get("asc") === "false";
      go(withParams({ order: field, asc: asc ? null : "false" }));
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      document.querySelectorAll(".groups-header-filters-type[open], .group-dropdown[open]").forEach(function (menu) {
        menu.open = false;
      });
    }
    if (event.key === "Enter" && event.target.closest) {
      var target = event.target.closest(".group-members .header-contents, .groups-header-filters-type .select-kit-row, .group-dropdown .select-kit-row");
      if (target) {
        target.click();
      }
    }
  });
})();
