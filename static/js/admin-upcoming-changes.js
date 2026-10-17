// The upcoming changes page in the browser (src/admin_upcoming_changes_
// view.rs draws it): DFilterControls over the list (the text filter, the
// drawer's dropdowns, reset, the URL's query params) and each
// UpcomingChangeItem's "Enabled for..." select, which toggles the change
// and saves its opt-in groups.
(function () {
  "use strict";

  if (!Discourse.once("admin-upcoming-changes")) {
    return;
  }

  var STAFF = "staff";
  var SAVING_DELAY = 2000;
  var INPUT_DELAY = 250;

  function ui() {
    return Discourse.adminUi;
  }

  function root() {
    return document.querySelector(".admin-contents.upcoming-changes");
  }

  var data = null;
  var urlTimer = null;
  // The table and every row as drawn, kept while filters take them out.
  var table = null;
  var allRows = [];

  function load() {
    var el = document.getElementById("upcoming-changes-data");
    try {
      data = el ? JSON.parse(el.textContent) : null;
    } catch (_) {
      data = null;
    }
  }

  function change(name) {
    return ((data && data.changes) || []).find(function (c) {
      return c.setting === name;
    });
  }

  function controls() {
    return root().querySelector(".d-filter-controls");
  }

  function textInput() {
    return controls().querySelector(".d-filter-controls__input");
  }

  function dropdownValue(key) {
    var select = controls().querySelector('.d-filter-controls__dropdown[data-dropdown-key="' + key + '"]');
    if (select) {
      return select.value;
    }
    return new URLSearchParams(location.search).get(key) || "all";
  }

  var FILTERS = {
    status: function (c, v) {
      return c.upcoming_change.status === v;
    },
    type: function (c, v) {
      return c.upcoming_change.impact_type === v;
    },
    impactRole: function (c, v) {
      return c.upcoming_change.impact_role === v;
    },
    enabled: function (c, v) {
      var enabledFor = c.upcoming_change.enabled_for;
      switch (v) {
        case "enabled":
          return enabledFor === "everyone";
        case "enabled_for_staff":
          return enabledFor === "staff";
        case "enabled_for_specific_groups":
          return enabledFor === "groups";
        case "disabled":
          return !c.value;
      }
      return true;
    },
  };

  function activeDropdowns() {
    return ((data && data.dropdowns) || []).filter(function (d) {
      return dropdownValue(d.key) !== "all";
    });
  }

  function hasActiveFilters() {
    return textInput().value.length > 0 || activeDropdowns().length > 0;
  }

  // filteredData: any of the comma separated terms in a searchable prop,
  // then each dropdown's filter.
  function matches(c) {
    var terms = textInput()
      .value.split(",")
      .map(function (t) {
        return t.trim().toLowerCase();
      })
      .filter(Boolean);
    if (terms.length) {
      var props = [c.humanized_name, c.description, c.plugin_identifier, c.setting];
      var hit = terms.some(function (term) {
        return props.some(function (p) {
          return p && String(p).toLowerCase().indexOf(term) !== -1;
        });
      });
      if (!hit) {
        return false;
      }
    }
    return activeDropdowns().every(function (d) {
      return FILTERS[d.key](c, dropdownValue(d.key));
    });
  }

  function noResults() {
    var reset = ui().escapeHtml(ui().t("filter_controls.reset"));
    return (
      '<div class="d-filter-controls__no-results"><p>' +
      ui().escapeHtml(ui().t("admin.upcoming_changes.filter.search_placeholder")) +
      '</p><button aria-label="' +
      reset +
      '" class="btn btn-icon-text btn-default d-filter-controls__reset" title="' +
      reset +
      '" type="button">' +
      ui().icon("arrow-rotate-left") +
      '<span class="d-button-label">' +
      reset +
      "</span></button></div>"
    );
  }

  function apply() {
    var el = root();
    if (!el || !data) {
      return;
    }
    // DFilterControls yields only the filtered rows, and no table at all
    // when none match.
    var visible = allRows.filter(function (row) {
      var c = change(row.dataset.upcomingChange);
      return !c || matches(c);
    });
    if (table) {
      table.querySelector("tbody").replaceChildren.apply(table.querySelector("tbody"), visible);
    }
    var existing = el.querySelector(".d-filter-controls__no-results");
    if (existing) {
      existing.remove();
    }
    if (table && data.changes.length) {
      if (visible.length) {
        if (!table.isConnected) {
          controls().insertAdjacentElement("afterend", table);
        }
      } else {
        table.remove();
        if (hasActiveFilters()) {
          controls().insertAdjacentHTML("afterend", noResults());
        }
      }
    }
    controls().querySelector(".d-filter-controls__inputs .d-filter-controls__reset").disabled = !hasActiveFilters();
  }

  function setParam(params, name, value) {
    if (value === null || value === "" || value === "all") {
      params.delete(name);
    } else {
      params.set(name, value);
    }
  }

  // DiscourseURL.replaceState with the filters' query params.
  function syncUrl() {
    var params = new URLSearchParams(location.search);
    setParam(params, "changeNamesFilter", textInput().value);
    ((data && data.dropdowns) || []).forEach(function (d) {
      setParam(params, d.key, dropdownValue(d.key));
    });
    var query = params.toString();
    history.replaceState(history.state, "", location.pathname + (query ? "?" + query : ""));
  }

  function dropdownsHtml() {
    return (
      '<div class="d-filter-controls__dropdowns">' +
      data.dropdowns
        .map(function (d) {
          var value = dropdownValue(d.key);
          return (
            '<select aria-label="' +
            ui().escapeHtml(d.options[0][1]) +
            '" class="d-native-select d-filter-controls__dropdown d-filter-controls__dropdown--' +
            d.key +
            (value === "all" ? "" : " --active") +
            '" data-dropdown-key="' +
            d.key +
            '">' +
            d.options
              .map(function (o) {
                var selected = o[0] === value;
                return (
                  '<option class="d-native-select__option' +
                  (selected ? " --selected" : "") +
                  '" value="' +
                  ui().escapeHtml(o[0]) +
                  '"' +
                  ">" +
                  ui().escapeHtml(o[1]) +
                  "</option>"
                );
              })
              .join("") +
            "</select>"
          );
        })
        .join("") +
      "</div>"
    );
  }

  // DNativeSelect's value: the --selected option, as the property (the
  // markup carries no selected attribute).
  function syncSelects(scope) {
    scope.querySelectorAll("select.d-native-select").forEach(function (select) {
      var option = select.querySelector("option.--selected");
      if (option) {
        option.selected = true;
      }
    });
  }

  function setToggleIcon(open) {
    var toggle = controls().querySelector(".d-filter-controls__toggle-filters");
    toggle.setAttribute("aria-expanded", open ? "true" : "false");
    var svg = toggle.querySelector("svg");
    if (svg) {
      svg.outerHTML = ui().icon(open ? "filter-circle-xmark" : "filter");
    }
  }

  function toggleDrawer() {
    var existing = controls().querySelector(".d-filter-controls__dropdowns");
    if (existing) {
      existing.remove();
      setToggleIcon(false);
    } else {
      controls().insertAdjacentHTML("beforeend", dropdownsHtml());
      syncSelects(controls());
      setToggleIcon(true);
    }
  }

  function reset() {
    textInput().value = "";
    controls()
      .querySelectorAll(".d-filter-controls__dropdown")
      .forEach(function (select) {
        select.value = "all";
        select.classList.remove("--active");
        Array.prototype.forEach.call(select.options, function (o) {
          o.classList.toggle("--selected", o.selected);
        });
      });
    var params = new URLSearchParams(location.search);
    ["changeNamesFilter"]
      .concat(
        data.dropdowns.map(function (d) {
          return d.key;
        })
      )
      .forEach(function (name) {
        params.delete(name);
      });
    var query = params.toString();
    history.replaceState(history.state, "", location.pathname + (query ? "?" + query : ""));
    apply();
    textInput().focus();
  }

  // UpcomingChangeItem

  function rowOf(name) {
    return allRows.find(function (row) {
      return row.dataset.upcomingChange === name;
    });
  }

  function enabledForLabel(enabledFor, groups) {
    if (enabledFor === "no_one") {
      return ui().t("admin.upcoming_changes.enabled_for_options.no_one");
    }
    if (enabledFor === "everyone") {
      return ui().t("admin.upcoming_changes.enabled_for_options.everyone");
    }
    if (enabledFor === STAFF) {
      return ui().t("admin.upcoming_changes.enabled_for_options.staff", { staffGroupName: "Staff" });
    }
    var names = (groups || "").split(",");
    var key = names.length === 1 ? "one" : "other";
    return ui().t("admin.upcoming_changes.enabled_for_options.specific_groups_with_group_names." + key, {
      groupNames: names.join(", "),
      count: names.length,
    });
  }

  function toggleChange(c, enabled, enabledFor) {
    return ui()
      .ajax("PUT", "/admin/config/upcoming-changes/toggle", { enabled: enabled, setting_name: c.setting })
      .then(function () {
        c.value = enabled;
        ui().toast(
          enabled
            ? ui().t("admin.upcoming_changes.change_enabled_for_success", {
                enabledFor: enabledForLabel(enabledFor, c._groups).toLowerCase(),
              })
            : ui().t("admin.upcoming_changes.change_disabled")
        );
      });
  }

  function selectedGroups(row) {
    var trigger = row.querySelector(".group-selector");
    if (!trigger) {
      return "";
    }
    return Array.prototype.map
      .call(trigger.querySelectorAll(".d-multi-select-trigger__selected-item"), function (b) {
        return b.title;
      })
      .join(",");
  }

  function saveGroups(c, opts) {
    var row = rowOf(c.setting);
    var enabledFor = c._enabledFor;
    var silence = enabledFor === "groups" || (opts && opts.silenceToast);
    var isGroupsMode = enabledFor === "groups";
    var groupNames;
    var toggleValue;
    if (!c._groups.length && isGroupsMode) {
      groupNames = [];
      enabledFor = "no_one";
      toggleValue = false;
    } else {
      groupNames = c._groups.length ? c._groups.split(",") : [];
      if (isGroupsMode) {
        toggleValue = true;
      }
    }
    return ui()
      .ajax("PUT", "/admin/config/upcoming-changes/groups", { setting: c.setting, group_names: groupNames })
      .then(function () {
        c._groups = groupNames.join(",");
        c.groups = c._groups || null;
        c._dirty = false;
        if (enabledFor !== c._enabledFor) {
          setEnabledFor(c, enabledFor);
        }
        if (!silence) {
          ui().toast(ui().t("admin.upcoming_changes.groups_updated"));
        }
        refreshRow(c);
        if (toggleValue !== undefined) {
          return toggleChange(c, toggleValue, enabledFor);
        }
      })
      .catch(function (error) {
        ui().failed(error);
        if (row) {
          refreshRow(c);
        }
      });
  }

  function groupSelectionHtml(c) {
    var label = ui().t("admin.upcoming_changes.select_groups");
    var names = (c._groups || "").split(",").filter(Boolean);
    var selection = names.length
      ? '<div class="d-multi-select-trigger__selection">' +
        names
          .map(function (n) {
            var e = ui().escapeHtml(n);
            return (
              '<button class="d-multi-select-trigger__selected-item" title="' +
              e +
              '" type="button" data-value="' +
              e +
              '"><span class="d-multi-select-trigger__selection-label">' +
              e +
              "</span>" +
              ui().icon("xmark").replace("svg-icon", "svg-icon d-multi-select-trigger__remove-selection-icon") +
              "</button>"
            );
          })
          .join("") +
        "</div>"
      : '<span class="d-multi-select-trigger__label">' + ui().escapeHtml(label) + "</span>";
    return (
      '<div class="upcoming-change__group-selection-wrapper"><div class="group-selector-wrapper"><div aria-expanded="false" class="fk-d-menu__trigger d-multi-select-trigger' +
      (names.length ? " --has-selection" : "") +
      ' group-selector" data-identifier="d-multi-select" data-trigger="" data-label="' +
      ui().escapeHtml(label) +
      '">' +
      selection +
      '<button class="btn no-text btn-icon btn-transparent d-multi-select-trigger__expand-btn" type="button">' +
      ui().icon("angle-down") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>' +
      saveButtonHtml(c._dirty) +
      "</div>"
    );
  }

  function saveButtonHtml(dirty) {
    var button =
      '<button class="btn btn-icon btn-primary no-text upcoming-change__save-groups"' +
      (dirty ? ' title="' + ui().escapeHtml(ui().t("admin.upcoming_changes.save_groups")) + '"' : ' disabled=""') +
      ' type="button">' +
      ui().icon("check") +
      '<span aria-hidden="true">&#8203;</span></button>';
    if (dirty) {
      return button;
    }
    return (
      '<span class="fk-d-tooltip__trigger" aria-expanded="false" data-trigger="" role="button"><span class="fk-d-tooltip__trigger-container">' +
      button +
      "</span></span>"
    );
  }

  // The DTooltips' content: the status's description, or why the groups
  // can't be saved yet.
  function tooltipText(trigger) {
    var row = trigger.closest(".upcoming-change-row");
    var c = row && change(row.dataset.upcomingChange);
    if (!c) {
      return null;
    }
    if (trigger.querySelector(".upcoming-change__save-groups")) {
      return c._groups
        ? ui().t("admin.upcoming_changes.no_changes_to_save")
        : ui().t("admin.upcoming_changes.add_groups_to_enable");
    }
    return ui().t("admin.upcoming_changes.status_descriptions." + c.upcoming_change.status);
  }

  function showTooltip(trigger) {
    var text = tooltipText(trigger);
    if (text && Discourse.float) {
      Discourse.float.showTooltip(trigger, { content: ui().escapeHtml(text) });
    }
  }

  function relatedLinksHtml(c) {
    var html = "";
    if (c._enabledFor === "no_one") {
      return html;
    }
    var label = ui().escapeHtml(ui().t("admin.upcoming_changes.show_related_settings"));
    if (c.dependents && c.dependents.length) {
      html +=
        '<div class="upcoming-change__dependents"><a class="" href="' +
        ui().base() +
        "/admin/site_settings?dependsOn=" +
        encodeURIComponent(c.setting) +
        '&filter=all_results">' +
        label +
        "</a></div>";
    }
    if (c.overriding_defaults) {
      html +=
        '<div class="upcoming-change__default-override-setting"><a class="" href="' +
        ui().base() +
        "/admin/site_settings?filter=" +
        encodeURIComponent("upcoming_change_default_override:" + c.setting) +
        '">' +
        label +
        "</a></div>";
    }
    return html;
  }

  // The detail cell's links and group selection after the select.
  function refreshRow(c) {
    var row = rowOf(c.setting);
    if (!row) {
      return;
    }
    var cell = row.querySelector(".upcoming-change__toggle-cell");
    var select = cell.querySelector("select.upcoming-change__enabled-for");
    while (select.nextSibling) {
      select.nextSibling.remove();
    }
    var html = relatedLinksHtml(c);
    if (c._enabledFor === "groups") {
      html += groupSelectionHtml(c);
    }
    select.insertAdjacentHTML("afterend", html);
    select.value = c._enabledFor;
    Array.prototype.forEach.call(select.options, function (o) {
      o.classList.toggle("--selected", o.selected);
    });
  }

  function setEnabledFor(c, value) {
    c._enabledFor = value;
    c.upcoming_change.enabled_for = value;
  }

  function enabledForChanged(c, select) {
    var oldValue = c.upcoming_change.enabled_for;
    var newValue = select.value;
    c._enabledFor = newValue;
    if (newValue === "groups") {
      refreshRow(c);
      return;
    }
    select.disabled = true;
    var isEnabled = newValue !== "no_one";
    var work;
    if (newValue === STAFF) {
      c._groups = STAFF;
      c._dirty = true;
      work = saveGroups(c, { silenceToast: true }).then(function () {
        return toggleChange(c, isEnabled, newValue);
      });
    } else {
      work = toggleChange(c, isEnabled, newValue).then(function () {
        c._groups = "";
        c._dirty = true;
        return saveGroups(c, { silenceToast: true });
      });
    }
    work
      .then(function () {
        setEnabledFor(c, newValue);
        refreshRow(c);
      })
      .catch(function (error) {
        setEnabledFor(c, oldValue);
        refreshRow(c);
        ui().failed(error);
      })
      .then(function () {
        setTimeout(function () {
          select.disabled = false;
        }, SAVING_DELAY);
      });
  }

  // GroupSelector's DMultiSelect: Group.findAll by the term.
  function groupSearch(term) {
    var params = new URLSearchParams({ term: term || "", ignore_automatic: "false" });
    return ui()
      .ajax("GET", "/groups/search.json?" + params.toString())
      .then(function (groups) {
        return (groups || []).map(function (g) {
          return { value: g.name, name: g.name };
        });
      });
  }

  document.addEventListener("input", function (event) {
    if (!root() || !event.target.closest(".upcoming-changes .d-filter-controls__input")) {
      return;
    }
    apply();
    clearTimeout(urlTimer);
    urlTimer = setTimeout(syncUrl, INPUT_DELAY);
  });

  document.addEventListener("change", function (event) {
    if (!root()) {
      return;
    }
    var dropdown = event.target.closest(".upcoming-changes .d-filter-controls__dropdown");
    if (dropdown) {
      dropdown.classList.toggle("--active", dropdown.value !== "all");
      syncUrl();
      apply();
      return;
    }
    var select = event.target.closest(".upcoming-changes select.upcoming-change__enabled-for");
    if (select) {
      var c = change(select.closest(".upcoming-change-row").dataset.upcomingChange);
      if (c) {
        enabledForChanged(c, select);
      }
      return;
    }
    var trigger = event.target.closest(".upcoming-changes .group-selector");
    if (trigger) {
      var item = change(trigger.closest(".upcoming-change-row").dataset.upcomingChange);
      if (item) {
        item._groups = selectedGroups(trigger.closest(".upcoming-change-row"));
        item._dirty = true;
        var wrapper = trigger.closest(".upcoming-change__group-selection-wrapper");
        var old = wrapper.lastElementChild;
        old.outerHTML = saveButtonHtml(true);
      }
    }
  });

  document.addEventListener("mouseover", function (event) {
    var trigger = root() && event.target.closest(".upcoming-changes .upcoming-change-row .fk-d-tooltip__trigger");
    if (trigger && !trigger.contains(event.relatedTarget)) {
      showTooltip(trigger);
    }
  });

  document.addEventListener("click", function (event) {
    if (!root()) {
      return;
    }
    var tip = event.target.closest(".upcoming-changes .upcoming-change-row .fk-d-tooltip__trigger");
    if (tip && !event.target.closest(".upcoming-change__save-groups:not([disabled])")) {
      event.preventDefault();
      showTooltip(tip);
      return;
    }
    if (event.target.closest(".upcoming-changes .d-filter-controls__toggle-filters")) {
      toggleDrawer();
      return;
    }
    if (event.target.closest(".upcoming-changes .d-filter-controls__reset")) {
      reset();
      return;
    }
    var save = event.target.closest(".upcoming-changes .upcoming-change__save-groups");
    if (save && !save.disabled) {
      var c = change(save.closest(".upcoming-change-row").dataset.upcomingChange);
      if (c) {
        saveGroups(c).then(function () {
          setEnabledFor(c, c._enabledFor);
          refreshRow(c);
        });
      }
    }
  });

  Discourse.onPage(function () {
    var el = root();
    if (!el) {
      return;
    }
    ui().strings("admin-upcoming-changes-strings");
    load();
    ((data && data.changes) || []).forEach(function (c) {
      c._enabledFor = c.upcoming_change.enabled_for;
      c._groups = c.groups || "";
      c._dirty = false;
    });
    table = el.querySelector(".upcoming-changes-table");
    allRows = table ? Array.prototype.slice.call(table.querySelectorAll("tbody > .upcoming-change-row")) : [];
    // The text filter the URL carries, set as the input's value.
    var initial = new URLSearchParams(location.search).get("changeNamesFilter");
    if (initial) {
      textInput().value = initial;
    }
    ui().multiSelect("group-selector", groupSearch);
    syncSelects(el);
    if (Discourse.lightbox) {
      el.querySelectorAll(".upcoming-change__description-details").forEach(Discourse.lightbox);
    }
    if (textInput().value || activeDropdowns().length) {
      apply();
    }
  });
})();
