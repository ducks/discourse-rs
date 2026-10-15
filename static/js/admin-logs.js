// The admin logs pages in the browser. The staff action logs: an action,
// target user or subject clicked filters by it, the filter buttons clear
// one or all, the action menu and the dates refilter, a long detail opens
// its modal, the post and topic links go to them, and the load-more
// sentinel appends the next page. The route's state is the URL's query
// (`filters` as JSON, `startDate`, `endDate`); a change swaps in its page
// as a link would. Not ported, and saying so when used: the CSV export
// and the theme and tag group changes' details modal.
(function () {
  "use strict";

  if (!Discourse.once("admin-logs")) {
    return;
  }

  function staffPage() {
    return document.querySelector(".admin-contents.staff-action-logs");
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function unsupported(what) {
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  }

  function go(params) {
    var query = params.toString();
    var url = location.pathname + (query ? "?" + query : "");
    if (!window.htmx) {
      location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  function currentFilters(params) {
    try {
      return JSON.parse(params.get("filters") || "{}") || {};
    } catch (_) {
      return {};
    }
  }

  // AdminLogsStaffActionLogsController#changeFilters
  function changeFilters(props) {
    var params = new URLSearchParams(location.search);
    var filters = currentFilters(params);
    Object.keys(props).forEach(function (key) {
      if (props[key] === undefined || props[key] === null || props[key] === "") {
        delete filters[key];
      } else {
        filters[key] = props[key];
      }
    });
    params.delete("page");
    if (Object.keys(filters).length) {
      params.set("filters", JSON.stringify(filters));
    } else {
      params.delete("filters");
    }
    go(params);
  }

  function dateParam(input) {
    if (!input.value) {
      return null;
    }
    // The picked day's local midnight, as the range's moment gives it.
    return new Date(input.value + "T00:00:00").toISOString();
  }

  function loadMore(root) {
    var sentinel = root.querySelector(".load-more-sentinel");
    if (!sentinel || sentinel.dataset.loading) {
      return;
    }
    sentinel.dataset.loading = "1";
    var params = new URLSearchParams(location.search);
    var next = parseInt(sentinel.dataset.page || "0", 10) + 1;
    params.set("page", String(next));
    fetch(location.pathname + "?" + params.toString(), { credentials: "same-origin" })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var tbody = root.querySelector(".staff-logs tbody");
        doc.querySelectorAll(".staff-logs tbody tr").forEach(function (row) {
          tbody.appendChild(document.importNode(row, true));
        });
        var more = doc.querySelector(".load-more-sentinel");
        if (more) {
          sentinel.dataset.page = String(next);
          delete sentinel.dataset.loading;
        } else {
          sentinel.remove();
        }
      });
  }

  Discourse.onPage(function () {
    var root = staffPage();
    var sentinel = root && root.querySelector(".load-more-sentinel");
    if (!sentinel || !("IntersectionObserver" in window)) {
      return;
    }
    var observer = new IntersectionObserver(function (entries) {
      if (entries.some(function (e) { return e.isIntersecting; })) {
        loadMore(root);
      }
    });
    observer.observe(sentinel);
    return function () {
      observer.disconnect();
    };
  });

  // The screened lists.
  function screenedPage() {
    return document.querySelector(
      ".admin-contents.screened-emails, .admin-contents.screened-urls, .admin-contents.screened-ip-addresses"
    );
  }

  function strings() {
    var json = document.getElementById("admin-logs-strings");
    return json ? JSON.parse(json.textContent) : {};
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : "";
  }

  function send(method, path, data) {
    var body = new URLSearchParams();
    Object.keys(data || {}).forEach(function (k) {
      body.set(k, data[k]);
    });
    return fetch(base() + path, {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
      body: body.toString(),
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = JSON.parse(text);
        } catch (_) {}
        if (!response.ok) {
          var error = new Error("http");
          error.json = json;
          throw error;
        }
        return json;
      });
    });
  }

  // generic_error_with_reason with the response's errors, else
  // generic_error.
  function reportError(e) {
    var s = strings();
    var message =
      e.json && e.json.errors
        ? (s.generic_error_with_reason || "").replace("%{error}", e.json.errors.join(". "))
        : s.generic_error;
    Discourse.dialog.alert(message);
  }

  function reload() {
    go(new URLSearchParams(location.search));
  }

  var filterTimer = null;
  document.addEventListener("input", function (event) {
    if (!event.target.matches(".filter-screened-ip-address .ip-address-input")) {
      return;
    }
    var value = event.target.value;
    clearTimeout(filterTimer);
    filterTimer = setTimeout(function () {
      var params = new URLSearchParams(location.search);
      value ? params.set("filter", value) : params.delete("filter");
      history.replaceState(history.state, "", location.pathname + (params.toString() ? "?" + params : ""));
      go(params);
    }, 250);
  });

  document.addEventListener("submit", function (event) {
    var form = event.target.closest(".screened-ip-address-form");
    if (!form) {
      return;
    }
    event.preventDefault();
    var button = form.querySelector("button[type=submit]");
    var input = form.querySelector(".ip-address-input");
    button.disabled = true;
    input.disabled = true;
    send("POST", "/admin/logs/screened_ip_addresses.json", {
      ip_address: input.value,
      action_name: form.querySelector("summary").dataset.value,
    }).then(reload, function (e) {
      button.disabled = false;
      input.disabled = false;
      reportError(e);
    });
  });

  function ipRow(target) {
    return target.closest(".screened-ip-addresses tr[data-id]");
  }

  function saveIp(row, data) {
    return send("PUT", "/admin/logs/screened_ip_addresses/" + row.dataset.id + ".json", data);
  }

  document.addEventListener("click", function (event) {
    var root = screenedPage();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var target = event.target;
    var row;
    var el;
    if (target.closest("[data-export]")) {
      unsupported("the screened lists' CSV export");
    } else if ((el = target.closest(".screened-email-allow"))) {
      row = el.closest("tr[data-id]");
      send("DELETE", "/admin/logs/screened_emails/" + row.dataset.id + ".json").then(function () {
        row.remove();
      }, reportError);
    } else if ((el = target.closest(".screened-ip-address-form details"))) {
      event.preventDefault();
      var choice = target.closest(".select-kit-row");
      var body = el.querySelector(".select-kit-body");
      if (choice) {
        var summary = el.querySelector("summary");
        summary.dataset.value = choice.dataset.value;
        summary.dataset.name = choice.dataset.name;
        el.querySelector(".select-kit-selected-name .name").textContent = choice.dataset.name;
        el.open = false;
        body.innerHTML = "";
        return;
      }
      if (el.open) {
        el.open = false;
        body.innerHTML = "";
        return;
      }
      el.open = true;
      JSON.parse(el.dataset.options).forEach(function (o) {
        var item = document.createElement("div");
        item.className = "select-kit-row";
        item.setAttribute("role", "option");
        item.dataset.value = o.id;
        item.dataset.name = o.name;
        var name = document.createElement("span");
        name.className = "name";
        name.textContent = o.name;
        item.appendChild(name);
        body.appendChild(item);
      });
    } else if ((row = ipRow(target))) {
      var s = strings();
      if (target.closest(".inline-editable-field, .screened-ip-edit")) {
        event.preventDefault();
        if (row.querySelector(".ip-edit-input")) {
          return;
        }
        var cell = row.querySelector(".ip_address");
        cell.dataset.saved = cell.innerHTML;
        cell.innerHTML = '<input class="ip-edit-input" type="text">';
        var input = cell.querySelector("input");
        input.value = row.dataset.ipAddress;
        input.focus();
        var actions = row.querySelector(".actions");
        actions.dataset.saved = actions.innerHTML;
        actions.innerHTML =
          '<button class="btn btn-text btn-default screened-ip-save" type="button"><span class="d-button-label"></span></button> ' +
          '<button class="btn btn-text btn-flat screened-ip-cancel" type="button"><span class="d-button-label"></span></button>';
        actions.querySelector(".screened-ip-save .d-button-label").textContent = s["admin.logs.save"];
        actions.querySelector(".screened-ip-cancel .d-button-label").textContent = s.cancel;
      } else if (target.closest(".screened-ip-cancel")) {
        var ipCell = row.querySelector(".ip_address");
        ipCell.innerHTML = ipCell.dataset.saved;
        var actionsCell = row.querySelector(".actions");
        actionsCell.innerHTML = actionsCell.dataset.saved;
      } else if (target.closest(".screened-ip-save")) {
        saveIp(row, {
          ip_address: row.querySelector(".ip-edit-input").value,
          action_name: row.dataset.actionName,
        }).then(reload, reportError);
      } else if (target.closest(".screened-ip-allow")) {
        saveIp(row, { ip_address: row.dataset.ipAddress, action_name: "do_nothing" }).then(reload, reportError);
      } else if (target.closest(".screened-ip-block")) {
        saveIp(row, { ip_address: row.dataset.ipAddress, action_name: "block" }).then(reload, reportError);
      } else if (target.closest(".screened-ip-delete")) {
        Discourse.dialog.confirm({
          message: (s["admin.logs.screened_ips.delete_confirm"] || "").replace("%{ip_address}", row.dataset.ipAddress),
          confirmLabel: s.yes_value,
          cancelLabel: s.no_value,
          didConfirm: function () {
            send("DELETE", "/admin/logs/screened_ip_addresses/" + row.dataset.id + ".json").then(function () {
              row.remove();
            }, reportError);
          },
        });
      }
    }
  });

  // The search logs: the period and the search type refilter.
  document.addEventListener("click", function (event) {
    var root = document.querySelector(".admin-contents.admin-search-logs");
    if (!root || !root.contains(event.target)) {
      return;
    }
    var params = new URLSearchParams(location.search);
    var row = event.target.closest(".period-chooser .select-kit-row");
    if (row) {
      event.preventDefault();
      params.set("period", row.dataset.value);
      go(params);
      return;
    }
    var combo = event.target.closest(".search-logs-filter");
    if (!combo) {
      return;
    }
    event.preventDefault();
    var choice = event.target.closest(".select-kit-row");
    if (choice) {
      choice.dataset.value === "all" ? params.delete("searchType") : params.set("searchType", choice.dataset.value);
      go(params);
      return;
    }
    var body = combo.querySelector(".select-kit-body");
    if (combo.open) {
      combo.open = false;
      body.innerHTML = "";
      return;
    }
    combo.open = true;
    JSON.parse(combo.dataset.options).forEach(function (o) {
      var item = document.createElement("div");
      item.className = "select-kit-row";
      item.setAttribute("role", "option");
      item.dataset.value = o.id;
      var name = document.createElement("span");
      name.className = "name";
      name.textContent = o.name;
      item.appendChild(name);
      body.appendChild(item);
    });
  });

  document.addEventListener("change", function (event) {
    var root = staffPage();
    if (!root || !event.target.matches(".date-filter-container .date-picker")) {
      return;
    }
    var params = new URLSearchParams(location.search);
    var from = dateParam(root.querySelector(".d-date-time-input.from .date-picker"));
    var to = dateParam(root.querySelector(".d-date-time-input.to .date-picker"));
    params.delete("page");
    from ? params.set("startDate", from) : params.delete("startDate");
    to ? params.set("endDate", to) : params.delete("endDate");
    go(params);
  });

  document.addEventListener("click", function (event) {
    var root = staffPage();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var target = event.target;
    var el;
    if ((el = target.closest("[data-filter-action]"))) {
      event.preventDefault();
      changeFilters({
        action_name: el.dataset.filterAction,
        action_id: el.dataset.actionId ? Number(el.dataset.actionId) : null,
        custom_type: el.dataset.customType || null,
      });
    } else if ((el = target.closest("[data-filter-target-user]"))) {
      event.preventDefault();
      changeFilters({ target_user: el.dataset.filterTargetUser });
    } else if ((el = target.closest("[data-filter-subject]"))) {
      event.preventDefault();
      changeFilters({ subject: el.dataset.filterSubject });
    } else if (target.closest(".clear-filters")) {
      var params = new URLSearchParams(location.search);
      params.delete("filters");
      params.delete("page");
      go(params);
    } else if ((el = target.closest("[data-clear-filter]"))) {
      var key = el.dataset.clearFilter;
      if (key === "actionFilter") {
        changeFilters({ action_name: null, action_id: null, custom_type: null });
      } else {
        var props = {};
        props[key] = null;
        changeFilters(props);
      }
    } else if ((el = target.closest(".staff-log-details"))) {
      event.preventDefault();
      var title = root.dataset.detailsTitle || "";
      Discourse.modal.show({
        className: "log-details-modal",
        title: el.closest(".staff-actions").dataset.modalTitle || title,
        body: function (body) {
          var pre = document.createElement("pre");
          pre.textContent = el.dataset.details;
          body.appendChild(pre);
        },
        footer: function (footer, close) {
          var button = document.createElement("button");
          button.className = "btn btn-text btn-default";
          button.type = "button";
          button.innerHTML = '<span class="d-button-label"></span>';
          button.querySelector("span").textContent = el.closest(".staff-actions").dataset.closeLabel || "";
          button.addEventListener("click", close);
          footer.appendChild(button);
        },
      });
    } else if (target.closest(".staff-log-custom-details")) {
      event.preventDefault();
      unsupported("the theme and tag group changes' details");
    } else if (target.closest(".export-staff-action-logs")) {
      unsupported("the staff action logs' CSV export");
    } else if ((el = target.closest("[data-link-post-id]"))) {
      event.preventDefault();
      location.assign(base() + "/p/" + el.dataset.linkPostId);
    } else if ((el = target.closest("[data-link-topic-id]"))) {
      event.preventDefault();
      location.assign(base() + "/t/" + el.dataset.linkTopicId);
    } else if ((el = target.closest("#staff-action-logs-action-filter"))) {
      event.preventDefault();
      var row = target.closest(".select-kit-row");
      if (row) {
        changeFilters({ action_name: row.dataset.value, action_id: Number(row.dataset.actionId) });
        return;
      }
      if (el.open) {
        el.open = false;
        el.classList.remove("is-expanded");
        el.querySelector(".select-kit-body").innerHTML = "";
        return;
      }
      var options = JSON.parse(el.dataset.options);
      el.open = true;
      el.classList.add("is-expanded");
      var collection = document.createElement("div");
      collection.className = "select-kit-collection";
      collection.setAttribute("role", "listbox");
      options.forEach(function (o) {
        var item = document.createElement("div");
        item.className = "select-kit-row";
        item.setAttribute("role", "option");
        item.tabIndex = 0;
        item.dataset.value = o.id;
        item.dataset.actionId = o.action_id;
        item.dataset.name = o.name;
        var name = document.createElement("span");
        name.className = "name";
        name.textContent = o.name;
        item.appendChild(name);
        collection.appendChild(item);
      });
      el.querySelector(".select-kit-body").appendChild(collection);
    }
  });
})();
