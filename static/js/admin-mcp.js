// The MCP server pages in the browser (src/admin_mcp_view.rs draws them):
// AdminMcp's actions for each section. The overview's endpoint copy; the
// access rules' delete and their form (GroupChooser, the scopes'
// DMultiSelect, save); the capabilities' filters, groups, visible
// selection, save and emergency blocks; the OAuth clients' filter, next
// pages, block and refresh, the new client's presets and form; the
// authorizations' filter, next pages and revoke; the activity's filters,
// date range and next pages. The lists' rows are drawn by the page
// (`fragment`).
(function () {
  "use strict";

  if (!Discourse.once("admin-mcp")) {
    return;
  }

  var INPUT_DELAY = 250;

  function ui() {
    return Discourse.adminUi;
  }

  function t(key, args) {
    return ui().t(key, args);
  }

  function esc(s) {
    return ui().escapeHtml(s);
  }

  function root() {
    return document.querySelector(".admin-contents.mcp .admin-mcp");
  }

  function base() {
    return ui().base();
  }

  function go(path) {
    window.location = base() + path;
  }

  // The section, from the route's path.
  function section() {
    var path = location.pathname.slice(base().length).replace(/\/$/, "");
    var rest = path.replace(/^\/admin\/config\/mcp/, "");
    if (rest === "") return "overview";
    if (rest === "/capabilities") return "primitives";
    if (rest === "/access") return "access";
    if (rest === "/access/new") return "access-new";
    if (/^\/access\/[^/]+\/edit$/.test(rest)) return "access-edit";
    if (rest === "/clients") return "clients";
    if (rest === "/clients/new") return "client-new";
    if (/^\/clients\/[^/]+$/.test(rest)) return "client-detail";
    if (rest === "/authorizations") return "authorizations";
    if (rest === "/activity") return "activity";
    return null;
  }

  var data = {};

  function load() {
    var el = document.getElementById("admin-mcp-data");
    try {
      data = el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      data = {};
    }
  }

  function toast(key, args) {
    ui().toast(t(key, args));
  }

  function count(key, n) {
    return t(key + (n === 1 ? ".one" : ".other"), { count: n });
  }

  // Overview

  function copyEndpoint() {
    var code = root().querySelector(".admin-mcp__endpoint code");
    if (!code || !navigator.clipboard) {
      return;
    }
    navigator.clipboard.writeText(code.textContent).then(function () {
      toast("admin.config.mcp.endpoint_copied");
    });
  }

  // Access

  function deleteAccessRule(row) {
    var name = row.querySelector(".d-table__overview-name").textContent;
    Discourse.dialog.confirm({
      message: t("admin.config.mcp.access.confirm_delete", { group: name }),
      didConfirm: function () {
        ui()
          .ajax("DELETE", "/admin/mcp/access/" + encodeURIComponent(row.dataset.groupId) + ".json")
          .then(function () {
            row.remove();
          }, ui().failed);
      },
    });
  }

  // GroupChooser (one group): the groups without a rule.
  function chosenGroup(details) {
    return (details.dataset.value || "").split("|").filter(Boolean)[0];
  }

  function groupRows(details, term) {
    var list = details.querySelector(".select-kit-collection");
    var filter = (term || "").toLowerCase();
    var chosen = chosenGroup(details);
    var groups = (data.groups || []).filter(function (g) {
      return !filter || g.name.toLowerCase().indexOf(filter) !== -1;
    });
    list.innerHTML = groups.length
      ? groups
          .map(function (g) {
            return (
              '<li class="select-kit-row' +
              (g.value === chosen ? " is-selected" : "") +
              '" role="menuitemradio" tabindex="0" data-value="' +
              esc(g.value) +
              '" data-name="' +
              esc(g.name) +
              '" title="' +
              esc(g.name) +
              '"><span class="name">' +
              esc(g.name) +
              "</span></li>"
            );
          })
          .join("")
      : '<li class="select-kit-row no-content">' + esc(t("select_kit.no_content")) + "</li>";
  }

  // The scopes' options with their primitive counts.
  function scopeOptions(term) {
    var counts = {};
    ((data.model && data.model.primitives) || []).forEach(function (p) {
      (p.required_scopes || []).forEach(function (s) {
        counts[s] = (counts[s] || 0) + 1;
      });
    });
    var query = (term || "").trim().toLowerCase();
    return ((data.model && data.model.available_scopes) || [])
      .filter(function (s) {
        return s.toLowerCase().indexOf(query) !== -1;
      })
      .map(function (s) {
        return { value: s, name: s, count: counts[s] || 0 };
      });
  }

  function saveAccessRule(form) {
    var details = form.querySelector("details.group-chooser");
    var groupId = data.rule ? data.rule.group_id : details && chosenGroup(details);
    var scopes = Array.prototype.map.call(
      form.querySelectorAll(".admin-mcp__scope-select .d-multi-select-trigger__selected-item"),
      function (b) {
        return b.dataset.value || b.title;
      }
    );
    var errors = [];
    if (!groupId) {
      errors.push("group_ids");
    }
    if (!scopes.length) {
      errors.push("scopes");
    }
    form.querySelectorAll(".form-kit__errors").forEach(function (e) {
      e.remove();
    });
    if (errors.length) {
      errors.forEach(function (name) {
        var field = form.querySelector('[data-name="' + name + '"] .form-kit__container-content');
        if (field) {
          field.insertAdjacentHTML(
            "beforeend",
            '<p class="form-kit__errors">' + esc(t("form_kit.errors.required")) + "</p>"
          );
        }
      });
      return;
    }
    var button = form.querySelector("button[type=submit]");
    button.disabled = true;
    ui()
      .ajax("PUT", "/admin/mcp/access/" + encodeURIComponent(groupId) + ".json", { scopes: scopes })
      .then(function () {
        toast("admin.config.mcp.access.saved");
        go("/admin/config/mcp/access");
      }, ui().failed)
      .then(function () {
        button.disabled = false;
      });
  }

  // Capabilities

  var groupBy = "scope";
  var selectedGroup = "all";

  function primitives() {
    return data.primitives || [];
  }

  function enabledState(p) {
    var box = (articleOf(p) && articleOf(p).querySelector(".form-kit__control-checkbox"));
    return box ? box.checked : Boolean(p.enabled);
  }

  function groupValues(p) {
    if (groupBy === "provider") return p.provider ? [p.provider] : [];
    if (groupBy === "kind") return p.kind ? [p.kind] : [];
    return p.required_scopes || [];
  }

  function groupIds() {
    var ids = [];
    if (groupBy === "scope") {
      (data.available_scopes || []).forEach(function (s) {
        if (
          primitives().some(function (p) {
            return groupValues(p).indexOf(s) !== -1;
          })
        ) {
          ids.push(s);
        }
      });
      return ids;
    }
    primitives().forEach(function (p) {
      groupValues(p).forEach(function (v) {
        if (ids.indexOf(v) === -1) ids.push(v);
      });
    });
    return ids;
  }

  function groupLabel(id) {
    if (id === "all") return t("admin.config.mcp.primitives.all_primitives");
    return groupBy === "kind" ? t("admin.config.mcp.values.primitive_kind." + id) : id;
  }

  function filterValue(name) {
    var el = root().querySelector('[name="' + name + '"]');
    return el ? el.value : "";
  }

  function matches(p) {
    var filter = filterValue("primitiveFilter").trim().toLowerCase();
    var text = [p.id, p.name, p.title, p.description, p.provider]
      .concat(p.required_scopes || [])
      .filter(Boolean)
      .join(" ")
      .toLowerCase();
    var risk = filterValue("primitiveRisk") || "all";
    var state = filterValue("primitiveState") || "all";
    var on = enabledState(p);
    return (
      (!filter || text.indexOf(filter) !== -1) &&
      (selectedGroup === "all" || groupValues(p).indexOf(selectedGroup) !== -1) &&
      (risk === "all" || p.risk === risk) &&
      (state === "all" ||
        (state === "enabled" && p.enabled) ||
        (state === "disabled" && !p.enabled) ||
        (state === "unavailable" && !p.available) ||
        (state === "blocked" && p.emergency_blocked))
    );
  }

  function drawGroups() {
    var list = root().querySelector(".admin-mcp__primitive-group-list");
    if (!list) return;
    var ids = ["all"].concat(groupIds());
    list.innerHTML = ids
      .map(function (id) {
        var members =
          id === "all"
            ? primitives()
            : primitives().filter(function (p) {
                return groupValues(p).indexOf(id) !== -1;
              });
        var enabled = members.filter(enabledState).length;
        var selected = id === selectedGroup;
        return (
          "<li><button" +
          (selected ? ' aria-pressed=""' : "") +
          ' class="admin-mcp__primitive-group' +
          (selected ? " is-selected" : "") +
          '" data-primitive-group-id="' +
          esc(id) +
          '" type="button"><span>' +
          esc(groupLabel(id)) +
          "</span> <small>" +
          esc(t("admin.config.mcp.primitives.group_count", { enabled: enabled, total: members.length })) +
          "</small></button></li>"
        );
      })
      .join("");
  }

  // The primitives' articles as drawn, kept while filters take them out.
  var articles = {};

  function articleOf(p) {
    return articles[p.id];
  }

  // filteredPrimitives: only the matching articles in the list, the
  // empty message when none.
  function applyPrimitiveFilters() {
    var shown = primitives()
      .filter(matches)
      .map(articleOf)
      .filter(Boolean);
    var listEl = root().querySelector(".admin-mcp__primitive-list");
    if (shown.length) {
      listEl.replaceChildren.apply(listEl, shown);
    } else {
      listEl.innerHTML =
        '<p class="admin-mcp__empty">' + esc(t("admin.config.mcp.primitives.no_results")) + "</p>";
    }
    var visible = shown.length;
    root().querySelector(".admin-mcp__results-count").textContent = t(
      "admin.config.mcp.primitives.results_count",
      { visible: visible, total: primitives().length }
    );
    root().querySelector("#mcp-primitive-group-title").textContent = groupLabel(selectedGroup);
  }

  // positionPrimitiveActions: the floating actions as wide as the form,
  // the form told their height.
  function positionPrimitiveActions() {
    var form = root() && root().querySelector(".admin-mcp__primitive-selection-form");
    var actions = form && form.querySelector(".form-kit__actions");
    if (!actions) return;
    actions.style.width = form.getBoundingClientRect().width + "px";
    form.style.setProperty("--mcp-primitive-actions-height", actions.getBoundingClientRect().height + "px");
  }

  window.addEventListener("resize", function () {
    if (root()) positionPrimitiveActions();
  });

  function primitivesDirty() {
    return primitives().some(function (p) {
      return enabledState(p) !== Boolean(p.enabled);
    });
  }

  function markDirty() {
    var form = root().querySelector(".admin-mcp__primitive-selection-form");
    var dirty = primitivesDirty();
    form.classList.toggle("has-floating-actions", dirty);
    form.querySelector(".form-kit__actions").classList.toggle("is-floating", dirty);
  }

  function savePrimitives(form) {
    var ids = primitives()
      .filter(enabledState)
      .map(function (p) {
        return p.id;
      });
    var button = form.querySelector("button[type=submit]");
    button.disabled = true;
    ui()
      .ajax("PUT", "/admin/mcp/capabilities.json", { primitive_ids: ids })
      .then(function () {
        primitives().forEach(function (p) {
          p.enabled = ids.indexOf(p.id) !== -1;
        });
        markDirty();
        drawGroups();
        toast("admin.config.mcp.primitives_saved");
      }, ui().failed)
      .then(function () {
        button.disabled = false;
      });
  }

  function setBlocked(p, blocked) {
    ui()
      .ajax("PUT", "/admin/mcp/capabilities/emergency-block.json", { primitive_id: p.id, blocked: blocked })
      .then(function () {
        p.emergency_blocked = blocked;
        var article = articleOf(p);
        var box = article.querySelector(".form-kit__control-checkbox");
        box.disabled = !p.available || blocked;
        // The blocked note (the unavailable one is empty).
        Array.prototype.forEach.call(article.querySelectorAll("p.admin-mcp__unavailable"), function (el) {
          if (el.textContent) el.remove();
        });
        if (blocked) {
          article
            .querySelector(".admin-mcp__primitive-actions")
            .insertAdjacentHTML(
              "beforebegin",
              '<p class="admin-mcp__unavailable">' + esc(t("admin.config.mcp.primitives.emergency_blocked")) + "</p>"
            );
        }
        var button = article.querySelector(".admin-mcp__primitive-actions button");
        button.title = t(
          blocked ? "admin.config.mcp.actions.unblock_primitive" : "admin.config.mcp.actions.block_primitive"
        );
        button.querySelector("svg").outerHTML = ui().icon(blocked ? "unlock" : "ban");
        toast(blocked ? "admin.config.mcp.primitive_blocked" : "admin.config.mcp.primitive_unblocked", {
          name: p.title || p.name,
        });
      }, ui().failed);
  }

  function togglePrimitiveBlock(article) {
    var p = primitives().find(function (x) {
      return x.id === article.dataset.primitiveId;
    });
    if (!p) return;
    var blocked = !p.emergency_blocked;
    if (blocked) {
      Discourse.dialog.confirm({
        message: t("admin.config.mcp.confirm_emergency_block", { name: p.title || p.name }),
        didConfirm: function () {
          setBlocked(p, true);
        },
      });
    } else {
      setBlocked(p, false);
    }
  }

  // Lists drawn by the page: clients, authorizations, activity.

  var nextCursor = null;
  var loading = false;
  var observer = null;
  var timer = null;

  function listQuery(cursor) {
    var params = new URLSearchParams(location.search);
    params.set("fragment", "1");
    var filterName = { clients: "clientFilter", authorizations: "authorizationFilter", activity: "activityFilter" }[
      section()
    ];
    var filter = filterValue(filterName).trim();
    if (filter) params.set("filter", filter);
    else params.delete("filter");
    if (section() === "activity") {
      var outcome = filterValue("activityOutcome");
      if (outcome && outcome !== "all") params.set("outcome", outcome);
      else params.delete("outcome");
    }
    if (cursor) params.set("cursor", cursor);
    else params.delete("cursor");
    return location.pathname + "?" + params.toString();
  }

  function fetchRows(cursor) {
    loading = true;
    return fetch(listQuery(cursor), { credentials: "same-origin", headers: { "X-Requested-With": "fragment" } })
      .then(function (response) {
        if (!response.ok) throw new Error(response.statusText);
        return response.json();
      })
      .then(function (json) {
        nextCursor = json.next_cursor || null;
        return json;
      })
      .finally(function () {
        loading = false;
      });
  }

  function tbody() {
    return root().querySelector(".admin-mcp__load-more tbody");
  }

  // The authorizations' ids, in their rows' order.
  var rowIds = [];

  function reloadList() {
    fetchRows(null).then(function (json) {
      tbody().innerHTML = json.html;
      rowIds = json.ids || [];
      if (section() === "activity") {
        var metrics = root().querySelector(".admin-mcp__activity-metrics");
        if (metrics) metrics.remove();
        if (json.metrics) {
          root().querySelector(".admin-mcp__load-more").insertAdjacentHTML("beforebegin", json.metrics);
        }
      }
      watch();
    }, ui().failed);
  }

  function watch() {
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    var sentinel = root() && root().querySelector(".load-more-sentinel");
    if (!sentinel || !window.IntersectionObserver) return;
    observer = new IntersectionObserver(
      function (entries) {
        if (!entries[0].isIntersecting || loading || !nextCursor) return;
        fetchRows(nextCursor).then(function (json) {
          tbody().insertAdjacentHTML("beforeend", json.html);
          rowIds = rowIds.concat(json.ids || []);
        }, ui().failed);
      },
      { rootMargin: "0px 0px 250px 0px" }
    );
    observer.observe(sentinel);
  }

  // Clients

  function clientId(el) {
    var link = el.closest("tr") && el.closest("tr").querySelector(".d-table__overview-link");
    if (link) return link.getAttribute("href").split("/").pop();
    return data.client && data.client.id;
  }

  function afterClientChange() {
    if (section() === "client-detail") {
      location.reload();
    } else {
      reloadList();
    }
  }

  function toggleClientBlock(button) {
    var id = clientId(button);
    var blocked = section() === "client-detail" ? !data.client.blocked : button.classList.contains("btn-danger");
    var name =
      section() === "client-detail"
        ? data.client.name
        : button.closest("tr").querySelector(".d-table__overview-name").textContent;
    var update = function () {
      return ui()
        .ajax("PUT", "/admin/mcp/clients/" + encodeURIComponent(id) + "/block.json", { blocked: blocked })
        .then(afterClientChange);
    };
    if (blocked) {
      Discourse.dialog.confirm({
        message: t("admin.config.mcp.confirm_block_client", { name: name }),
        didConfirm: function () {
          update().catch(ui().failed);
        },
      });
    } else {
      update().catch(ui().failed);
    }
  }

  function refreshClient(button) {
    ui()
      .ajax("POST", "/admin/mcp/clients/" + encodeURIComponent(clientId(button)) + "/refresh.json")
      .then(afterClientChange, ui().failed);
  }

  var presetsHtml = null;

  function selectPreset(preset) {
    var main = root();
    fetch(location.pathname + "?fragment=1&preset=" + encodeURIComponent(preset), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "fragment" },
    })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        presetsHtml = presetsHtml || main.innerHTML;
        main.innerHTML = html;
      }, ui().failed);
  }

  function createClient(form) {
    var value = function (name) {
      return form.querySelector('[name="' + name + '"]').value;
    };
    var uris = value("redirect_uris")
      .split(/[\n,]/)
      .map(function (s) {
        return s.trim();
      })
      .filter(Boolean);
    var button = form.querySelector("button[type=submit]");
    button.disabled = true;
    ui()
      .ajax("POST", "/admin/mcp/clients.json", {
        client: { name: value("name"), client_id: value("client_id"), redirect_uris: uris },
      })
      .then(function (json) {
        toast("admin.config.mcp.clients.created");
        go("/admin/config/mcp/clients/" + json.client.id);
      }, ui().failed)
      .then(function () {
        button.disabled = false;
      });
  }

  // Authorizations

  function revokeAuthorization(button) {
    var row = button.closest("tr");
    var id = rowIds[Array.prototype.indexOf.call(tbody().children, row)];
    Discourse.dialog.confirm({
      message: t("admin.config.mcp.confirm_revoke_authorization", {
        name: row.querySelector(".d-table__overview-name").textContent,
      }),
      didConfirm: function () {
        ui()
          .ajax("DELETE", "/admin/mcp/authorizations/" + id + ".json")
          .then(function () {
            var status = row.querySelector(".admin-mcp__status");
            status.dataset.state = "revoked";
            status.textContent = t("admin.config.mcp.values.authorization_status.revoked");
            button.remove();
          }, ui().failed);
      },
    });
  }

  // Activity: the date range (DashboardDateRange) in the URL.

  function setRange(query) {
    var params = new URLSearchParams(location.search);
    ["range", "start_date", "end_date"].forEach(function (k) {
      params.delete(k);
    });
    Object.keys(query).forEach(function (k) {
      params.set(k, query[k]);
    });
    location.search = params.toString();
  }

  document.addEventListener("click", function (event) {
    if (!root()) return;
    var target = event.target;
    var sec = section();
    if (target.closest(".admin-mcp__endpoint button")) {
      copyEndpoint();
      return;
    }
    if (target.closest(".d-page-subheader .d-page-action-button")) {
      var label = target.closest(".d-page-action-button");
      if (sec === "access") return go("/admin/config/mcp/access/new");
      if (sec === "clients") return go("/admin/config/mcp/clients/new");
      if (sec === "client-detail") {
        var buttons = label.parentElement.querySelectorAll(".d-page-action-button");
        if (label === buttons[0]) toggleClientBlock(label);
        else refreshClient(label);
        return;
      }
    }
    if (target.closest(".admin-config-area-empty-list__cta-button")) {
      return go("/admin/config/mcp/clients/new");
    }
    var edit = target.closest(".admin-mcp__edit-access-rule");
    if (edit) {
      return go("/admin/config/mcp/access/" + edit.closest("tr").dataset.groupId + "/edit");
    }
    var del = target.closest(".admin-mcp__delete-access-rule");
    if (del) {
      deleteAccessRule(del.closest("tr"));
      return;
    }
    var group = target.closest(".admin-mcp__primitive-group");
    if (group) {
      selectedGroup = group.dataset.primitiveGroupId;
      drawGroups();
      applyPrimitiveFilters();
      return;
    }
    var visibleButton = target.closest(".admin-mcp__primitive-group-actions button");
    if (visibleButton) {
      var on = visibleButton === visibleButton.parentElement.firstElementChild;
      primitives().forEach(function (p) {
        if (!matches(p)) return;
        var box = (articleOf(p) && articleOf(p).querySelector(".form-kit__control-checkbox"));
        if (box) box.checked = on;
      });
      markDirty();
      drawGroups();
      return;
    }
    var block = target.closest(".admin-mcp__primitive-actions button");
    if (block) {
      togglePrimitiveBlock(block.closest("article"));
      return;
    }
    var toggleClient = target.closest(".admin-mcp__toggle-client-block");
    if (toggleClient) {
      toggleClientBlock(toggleClient);
      return;
    }
    var refresh = target.closest(".admin-mcp__refresh-client");
    if (refresh) {
      refreshClient(refresh);
      return;
    }
    var preset = target.closest("[data-client-preset-id] button");
    if (preset) {
      selectPreset(preset.closest("[data-client-preset-id]").dataset.clientPresetId);
      return;
    }
    if (target.closest(".admin-mcp__change-client-preset")) {
      if (presetsHtml) root().innerHTML = presetsHtml;
      presetsHtml = null;
      return;
    }
    var revoke = sec === "authorizations" && target.closest("td.--controls .btn-danger");
    if (revoke) {
      revokeAuthorization(revoke);
      return;
    }
    var range = target.closest(".admin-mcp .db-date-range__trigger");
    if (range && Discourse.dateRangePicker) {
      Discourse.float.menu(range, {
        identifier: "db-date-range-menu",
        contentClass: "db-date-range__popover",
        maxWidth: 800,
        modalForMobile: true,
        placement: "bottom-end",
        content: Discourse.dateRangePicker(range, {
          t: t,
          escapeHtml: esc,
          icon: ui().icon,
          close: function () {
            Discourse.float.close("db-date-range-menu");
          },
          onPreset: function (id) {
            setRange({ range: id });
          },
          onRange: function (start, end) {
            setRange({ range: "custom", start_date: start, end_date: end });
          },
        }),
      });
      return;
    }
    // GroupChooser's rows.
    var row = target.closest(".admin-mcp details.group-chooser .select-kit-row[data-value]");
    if (row) {
      var details = row.closest("details");
      details.dataset.value = row.dataset.value;
      details.querySelector(".formatted-selection").textContent = row.dataset.name;
      details.querySelector("summary").dataset.name = row.dataset.name;
      details.classList.add("has-selection");
      details.open = false;
    }
  });

  // The initial scope's preventRemoval: its chip stays.
  document.addEventListener(
    "click",
    function (event) {
      var chip = root() && event.target.closest(".admin-mcp__scope-select .d-multi-select-trigger__selected-item");
      var initial = data.model && data.model.initial_scope;
      if (chip && initial && chip.title === initial) {
        event.stopImmediatePropagation();
        event.preventDefault();
      }
    },
    true
  );

  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches(".admin-mcp details.group-chooser") || !details.open) {
        return;
      }
      details.querySelector(".select-kit-body").innerHTML =
        '<div class="select-kit-filter multi-select-filter is-expanded"><input autocapitalize="off" autocomplete="off" autocorrect="off" class="filter-input" placeholder="' +
        esc(t("select_kit.filter_placeholder")) +
        '" type="text"></div><ul class="select-kit-collection" role="menu"></ul>';
      groupRows(details, "");
      details.querySelector(".filter-input").focus();
    },
    true
  );

  document.addEventListener("input", function (event) {
    if (!root()) return;
    var chooser = event.target.closest(".admin-mcp details.group-chooser .filter-input");
    if (chooser) {
      groupRows(chooser.closest("details"), chooser.value);
      return;
    }
    var name = event.target.name;
    if (name === "primitiveFilter") {
      selectedGroup = "all";
      drawGroups();
      applyPrimitiveFilters();
    } else if (name === "clientFilter" || name === "authorizationFilter" || name === "activityFilter") {
      clearTimeout(timer);
      timer = setTimeout(reloadList, INPUT_DELAY);
    }
  });

  document.addEventListener("change", function (event) {
    if (!root()) return;
    var name = event.target.name;
    if (name === "primitiveGroupBy") {
      groupBy = event.target.value;
      selectedGroup = "all";
      drawGroups();
      applyPrimitiveFilters();
    } else if (name === "primitiveRisk" || name === "primitiveState") {
      applyPrimitiveFilters();
    } else if (name === "activityOutcome") {
      reloadList();
    } else if (event.target.closest(".admin-mcp__primitive .form-kit__control-checkbox")) {
      markDirty();
      drawGroups();
    }
  });

  document.addEventListener(
    "submit",
    function (event) {
      var form = event.target;
      if (!root() || !root().contains(form)) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      if (form.classList.contains("admin-mcp__access-form")) {
        saveAccessRule(form);
      } else if (form.classList.contains("admin-mcp__primitive-selection-form")) {
        savePrimitives(form);
      } else if (form.closest(".admin-mcp__client-form")) {
        createClient(form);
      }
    },
    true
  );

  Discourse.onPage(function () {
    if (!root()) return;
    ui().strings("admin-mcp-strings");
    load();
    groupBy = "scope";
    selectedGroup = "all";
    presetsHtml = null;
    nextCursor = (data.meta && data.meta.next_cursor) || data.next_cursor || null;
    rowIds = (data.authorizations || []).map(function (a) {
      return a.id;
    });
    articles = {};
    Array.prototype.forEach.call(root().querySelectorAll(".admin-mcp__primitive[data-primitive-id]"), function (a) {
      articles[a.dataset.primitiveId] = a;
    });
    if (section() === "primitives") {
      // The enabled primitives' boxes, checked as FormKit sets them.
      primitives().forEach(function (p) {
        var box = (articleOf(p) && articleOf(p).querySelector(".form-kit__control-checkbox"));
        if (box) box.checked = Boolean(p.enabled);
      });
      positionPrimitiveActions();
    }
    if (section() === "access-new" || section() === "access-edit") {
      ui().multiSelect("admin-mcp__scope-select", function (term) {
        return scopeOptions(term).map(function (o) {
          return {
            value: o.value,
            name: o.name,
            resultHtml:
              '<span class="admin-mcp__scope-option"><span>' +
              esc(o.name) +
              "</span> <small>" +
              esc(count("admin.config.mcp.access.scope_primitives", o.count)) +
              "</small></span>",
          };
        });
      });
    }
    watch();
  });
})();
