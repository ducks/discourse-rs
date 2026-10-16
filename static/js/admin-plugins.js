// The installed plugins list in the browser (src/admin_plugins_view.rs
// draws it): DFilterControls' text filter (name, author, about; kept in
// the URL's filter) and status dropdown, and a plugin's enabled switch,
// which saves its enabled setting as SiteSetting.update does.
(function () {
  "use strict";

  if (!Discourse.once("admin-plugins")) {
    return;
  }

  var ui = Discourse.adminUi;
  var t = ui.t;
  var escapeHtml = ui.escapeHtml;

  function page() {
    return document.querySelector(".admin-contents.admin-plugins .admin-plugins-list-container");
  }

  function resetHtml(label) {
    return (
      '<button aria-label="' +
      escapeHtml(t("filter_controls.reset")) +
      '" class="btn ' +
      (label ? "btn-icon-text" : "no-text btn-icon") +
      ' btn-default d-filter-controls__reset" title="' +
      escapeHtml(t("filter_controls.reset")) +
      '" type="button">' +
      ui.icon("arrow-rotate-left") +
      (label
        ? '<span class="d-button-label">' + escapeHtml(t("filter_controls.reset")) + "</span>"
        : '<span aria-hidden="true">&#8203;</span>') +
      "</button>"
    );
  }

  function apply(root) {
    var input = root.querySelector(".d-filter-controls__input");
    var select = root.querySelector(".d-filter-controls__dropdown");
    var status = select.value;
    var terms = input.value
      .split(",")
      .map(function (term) {
        return term.trim().toLowerCase();
      })
      .filter(Boolean);
    var shown = 0;
    root.querySelectorAll(".admin-plugins-list__row").forEach(function (row) {
      // nameTitleized, author and about (the about's text, not its link).
      var about = row.querySelector(".admin-plugins-list__about").firstChild;
      var text = [
        row.querySelector(".admin-plugins-list__name").textContent,
        row.querySelector(".admin-plugins-list__author").textContent,
        about && about.nodeType === 3 ? about.textContent : "",
      ].map(function (s) {
        return s.trim().toLowerCase();
      });
      var matchesText =
        terms.length === 0 ||
        terms.some(function (term) {
          return text.some(function (s) {
            return s.indexOf(term) !== -1;
          });
        });
      var enabled = row.dataset.enabled === "true";
      var matchesStatus =
        status === "all" ||
        (status === "enabled" && enabled) ||
        (status === "disabled" && !enabled) ||
        (status === "preinstalled" && row.dataset.preinstalled === "true");
      var match = matchesText && matchesStatus;
      row.hidden = !match;
      if (match) {
        shown += 1;
      }
    });
    var active = input.value.length > 0 || status !== "all";
    select.classList.toggle("--active", status !== "all");
    select.setAttribute("aria-label", select.options[select.selectedIndex].text);
    root.querySelector(".d-filter-controls > .d-filter-controls__reset").disabled = !active;
    var table = root.querySelector(".admin-plugins-list");
    table.hidden = shown === 0;
    var none = root.querySelector(".d-filter-controls__no-results");
    if (shown === 0 && active) {
      if (!none) {
        table.insertAdjacentHTML(
          "beforebegin",
          '<div class="d-filter-controls__no-results"><p>' +
            escapeHtml(t("admin.plugins.filters.no_results")) +
            "</p>" +
            resetHtml(true) +
            "</div>"
        );
      }
    } else if (none) {
      none.remove();
    }
    var url = new URL(location.href);
    if (input.value) {
      url.searchParams.set("filter", input.value);
    } else {
      url.searchParams.delete("filter");
    }
    history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  }

  var timer = null;
  document.addEventListener("input", function (event) {
    var root = page();
    if (root && event.target.closest(".d-filter-controls__input")) {
      clearTimeout(timer);
      timer = setTimeout(function () {
        apply(root);
      }, 250);
    }
  });

  document.addEventListener("change", function (event) {
    var root = page();
    if (root && event.target.closest(".d-filter-controls__dropdown")) {
      apply(root);
    }
  });

  function setSwitch(button, on) {
    button.setAttribute("aria-checked", on ? "true" : "false");
    button.parentNode.querySelector(".d-toggle-switch__checkbox-slider").innerHTML = on ? ui.icon("check") : "";
    button.closest(".admin-plugins-list__row").dataset.enabled = on ? "true" : "false";
  }

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root || !root.contains(event.target)) {
      return;
    }
    if (event.target.closest(".d-filter-controls__reset")) {
      root.querySelector(".d-filter-controls__input").value = "";
      root.querySelector(".d-filter-controls__dropdown").value = "all";
      apply(root);
      return;
    }
    var button = event.target.closest(".d-toggle-switch__checkbox[data-enabled-setting]");
    if (!button) {
      return;
    }
    var old = button.getAttribute("aria-checked") === "true";
    setSwitch(button, !old);
    var setting = button.dataset.enabledSetting;
    var data = {};
    data[setting] = !old;
    ui.ajax("PUT", "/admin/site_settings/" + encodeURIComponent(setting), data).catch(function (error) {
      setSwitch(button, old);
      ui.failed(error);
    });
  });

  Discourse.onPage(function () {
    var root = page();
    if (!root) {
      return;
    }
    ui.strings("admin-plugins-strings");
    if (root.querySelector(".d-filter-controls__input") && root.querySelector(".d-filter-controls__input").value) {
      apply(root);
    }
  });
})();
