// The admin's users list (AdminUsersListShowController) in the browser:
// a column header sorts by it (DTableHeaderToggle: descending, then
// ascending, then descending again), the text filter loads the matching
// users after INPUT_DELAY, the dropdown (account type, activation) and
// Show emails load the list again, Reset clears the filters, DLoadMore's
// sentinel appends the next page, and bulk select checks rows for the
// bulk actions. Each change is the page for the URL with the
// controller's state as query params, swapped in as a link's is. Not
// ported, and saying so when used: the invite and bulk action modals and
// the user list export.
(function () {
  "use strict";

  if (!Discourse.once("admin-users")) {
    return;
  }

  var INPUT_DELAY = 250;
  var MAX_BULK_SELECT_LIMIT = 100;
  var timer = null;
  var loading = false;

  function list() {
    return document.querySelector(".admin-contents.admin-users-list");
  }

  function params() {
    return new URLSearchParams(location.search);
  }

  function urlWith(changes) {
    var p = params();
    Object.keys(changes).forEach(function (key) {
      if (changes[key] === null || changes[key] === undefined || changes[key] === "") {
        p.delete(key);
      } else {
        p.set(key, changes[key]);
      }
    });
    p.delete("page");
    var query = p.toString();
    return location.pathname + (query ? "?" + query : "");
  }

  // The page for the URL, swapped in as a link's is (hx-boost), the
  // filter's focus kept.
  function go(url, keepFocus) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    if (keepFocus) {
      link.setAttribute("hx-push-url", "false");
      history.replaceState(history.state, "", url);
      Discourse.adminUsersRefocus = true;
    }
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function selectedCount() {
    return document.querySelectorAll("input.directory-table__cell-bulk-select:checked").length;
  }

  function syncBulkActions() {
    var root = list();
    var controls = root && root.querySelector(".d-filter-controls");
    if (!controls) {
      return;
    }
    var existing = controls.querySelector(".bulk-actions-dropdown");
    if (selectedCount() > 0 && !existing) {
      var template = document.getElementById("admin-users-bulk-actions");
      if (template) {
        controls.appendChild(template.content.cloneNode(true));
      }
    } else if (selectedCount() === 0 && existing) {
      existing.remove();
    }
  }

  function toggleBulkSelect(root) {
    var on = !root.classList.contains("--bulk-select");
    root.classList.toggle("--bulk-select", on);
    var wrapper = root.querySelector(".directory-table__column-header-wrapper");
    var template = document.getElementById("admin-users-bulk-header");
    wrapper.querySelectorAll(".bulk-select-all, .bulk-clear-all").forEach(function (b) {
      b.remove();
    });
    root.querySelectorAll(".directory-table__cell-bulk-select, .bulk-select-unavailable").forEach(function (el) {
      el.remove();
    });
    if (on && template) {
      var bulk = wrapper.querySelector(".bulk-select");
      bulk.after(template.content.cloneNode(true));
      root.querySelectorAll(".directory-table__row").forEach(function (row) {
        var cell = row.querySelector(".directory-table__cell.username");
        var input = document.createElement("input");
        input.type = "checkbox";
        input.className = "directory-table__cell-bulk-select";
        input.dataset.userId = row.dataset.userId;
        if (row.dataset.canBeActioned !== "true") {
          input.disabled = true;
          var span = document.createElement("span");
          span.className = "fk-d-tooltip__trigger bulk-select-unavailable";
          span.dataset.tooltipContent = template.dataset.staffCantBeActioned;
          span.appendChild(input);
          cell.prepend(span);
        } else {
          cell.prepend(input);
        }
      });
    }
    syncBulkActions();
  }

  function loadMore(root) {
    var sentinel = root.querySelector(".load-more-sentinel");
    if (!sentinel || loading) {
      return;
    }
    loading = true;
    var page = parseInt(sentinel.dataset.page || "1", 10) + 1;
    var p = params();
    p.set("page", String(page));
    fetch(location.pathname + "?" + p.toString(), { credentials: "same-origin" })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var rows = doc.querySelectorAll(".directory-table__body .directory-table__row");
        var body = root.querySelector(".directory-table__body");
        rows.forEach(function (row) {
          body.appendChild(document.importNode(row, true));
        });
        if (doc.querySelector(".load-more-sentinel")) {
          sentinel.dataset.page = String(page);
        } else {
          sentinel.remove();
        }
      })
      .catch(function () {})
      .then(function () {
        loading = false;
      });
  }

  Discourse.onPage(function () {
    var root = list();
    if (!root) {
      return;
    }
    if (Discourse.adminUsersRefocus) {
      Discourse.adminUsersRefocus = false;
      var input = root.querySelector(".d-filter-controls__input");
      if (input) {
        input.focus();
        input.setSelectionRange(input.value.length, input.value.length);
      }
    }
    var sentinel = root.querySelector(".load-more-sentinel");
    if (!sentinel || !("IntersectionObserver" in window)) {
      return;
    }
    var observer = new IntersectionObserver(function (entries) {
      if (entries.some(function (e) {
        return e.isIntersecting;
      })) {
        loadMore(root);
      }
    });
    observer.observe(sentinel);
    return function () {
      observer.disconnect();
    };
  });

  document.addEventListener("input", function (event) {
    var input = event.target.closest(".admin-users-list .d-filter-controls__input");
    if (!input) {
      return;
    }
    clearTimeout(timer);
    timer = setTimeout(function () {
      go(urlWith({ filter: input.value, username: null }), true);
    }, INPUT_DELAY);
  });

  document.addEventListener("change", function (event) {
    var select = event.target.closest(".admin-users-list .d-filter-controls__dropdown");
    if (select) {
      var key = location.pathname.match(/\/staff$/) ? "account_type" : "activation";
      var value = select.value;
      var defaults = { account_type: "human", activation: "all" };
      var change = {};
      change[key] = value === defaults[key] ? null : value;
      go(urlWith(change));
      return;
    }
    var check = event.target.closest(".admin-users-list input.directory-table__cell-bulk-select");
    if (check) {
      if (check.checked && selectedCount() > MAX_BULK_SELECT_LIMIT) {
        check.checked = false;
      }
      syncBulkActions();
    }
  });

  function sortBy(header) {
    var field = header.dataset.order;
    var p = params();
    var asc = p.get("order") === field && p.get("asc") !== "true" ? "true" : null;
    go(urlWith({ order: field, asc: asc }));
  }

  document.addEventListener("keypress", function (event) {
    var header = event.target.closest(".admin-users-list .directory-table__column-header.sortable");
    if (header && event.key === "Enter") {
      sortBy(header);
    }
  });

  document.addEventListener("click", function (event) {
    var root = event.target.closest(".admin-contents.admin-users");
    if (!root) {
      return;
    }
    var header = event.target.closest(".directory-table__column-header.sortable");
    if (header) {
      sortBy(header);
      return;
    }
    if (event.target.closest(".d-filter-controls__reset")) {
      go(location.pathname);
      return;
    }
    if (event.target.closest(".admin-users__subheader-show-emails")) {
      go(urlWith({ show_emails: "true" }));
      return;
    }
    if (event.target.closest(".admin-users__subheader-hide-emails")) {
      go(urlWith({ show_emails: null }));
      return;
    }
    if (event.target.closest(".bulk-select")) {
      toggleBulkSelect(root);
      return;
    }
    if (event.target.closest(".bulk-select-all")) {
      var unchecked = root.querySelectorAll("input.directory-table__cell-bulk-select:not(:checked):not([disabled])");
      var remaining = MAX_BULK_SELECT_LIMIT - selectedCount();
      Array.prototype.slice.call(unchecked, 0, remaining).forEach(function (input) {
        input.checked = true;
      });
      syncBulkActions();
      return;
    }
    if (event.target.closest(".bulk-clear-all")) {
      root.querySelectorAll("input.directory-table__cell-bulk-select:checked").forEach(function (input) {
        input.checked = false;
      });
      syncBulkActions();
      return;
    }
    if (event.target.closest(".bulk-actions-dropdown")) {
      unsupported("the bulk suspend and delete confirmations");
      return;
    }
    if (event.target.closest(".admin-users__header-send-invites")) {
      unsupported("the send invites modal");
      return;
    }
    if (event.target.closest(".admin-users__header-export-users")) {
      unsupported("the user list export");
    }
  });
})();
