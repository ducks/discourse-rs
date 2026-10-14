// The admin panel (AdminSidebarPanel) beyond what sidebar.js does for
// every sidebar: the header's ToggleAllSections, which opens every section
// or, when all are open, closes them, its icon and title following; and
// the moderators' Filter, which shows only the links whose text holds what
// is typed, every section open while it does (the admin search the
// admins' search box opens is not ported).
(function () {
  "use strict";

  if (!Discourse.once("admin-panel")) {
    return;
  }

  function collapsable(root) {
    return Array.prototype.filter.call(
      root.querySelectorAll(".sidebar-section[data-section-name]"),
      function (section) {
        return section.querySelector(".sidebar-section-header-collapsable");
      }
    );
  }

  function store(name, collapsed) {
    try {
      window.localStorage.setItem(
        "discourse_sidebar-section-" + name + "-collapsed",
        collapsed ? "true" : "false"
      );
    } catch (e) {
      // Storage blocked: the state lasts for this page only.
    }
  }

  function syncToggle(panel) {
    var button = panel.querySelector(".sidebar-toggle-all-sections");
    if (!button) {
      return;
    }
    var all = collapsable(panel).every(function (section) {
      return section.classList.contains("sidebar-section--expanded");
    });
    var icon = all ? "angles-up" : "angles-down";
    button.title = all ? button.dataset.collapseTitle : button.dataset.expandTitle;
    var svg = button.querySelector("svg");
    svg.setAttribute(
      "class",
      svg.getAttribute("class").replace(/d-icon-angles-(up|down)/, "d-icon-" + icon)
    );
    svg.querySelector("use").setAttribute("href", "#" + icon);
  }

  // DShortcut: the search's keycaps where there is a keyboard to press them
  // (a fine pointer), the Command glyph on Apple platforms.
  function shortcutHint(panel) {
    var template = panel.querySelector("template.sidebar-search__shortcut");
    if (!template || !window.matchMedia("(pointer: fine)").matches) {
      return;
    }
    var kbd = template.content.firstElementChild.cloneNode(true);
    if (/Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent)) {
      var key = kbd.querySelector(".d-shortcut__key");
      key.classList.add("d-shortcut__key--glyph");
      key.innerHTML =
        '<span aria-hidden="true">⌘</span><span class="sr-only"></span>';
      key.querySelector(".sr-only").textContent = kbd.dataset.command;
    }
    kbd.removeAttribute("data-command");
    template.replaceWith(kbd);
  }

  Discourse.onPage(function () {
    document.querySelectorAll(".admin-panel").forEach(function (panel) {
      shortcutHint(panel);
      syncToggle(panel);
    });
  });

  document.addEventListener("click", function (event) {
    var panel = event.target.closest(".admin-panel");
    if (!panel) {
      return;
    }
    if (event.target.closest(".sidebar-toggle-all-sections")) {
      var sections = collapsable(panel);
      var collapse = sections.every(function (section) {
        return section.classList.contains("sidebar-section--expanded");
      });
      sections.forEach(function (section) {
        Discourse.setSidebarSectionExpanded(section, !collapse);
        store(section.getAttribute("data-section-name"), collapse);
      });
      syncToggle(panel);
      return;
    }
    var missing = event.target.closest(
      ".sidebar-search__input, .sidebar-search__icon, [data-sidebar-action-id='design_wizard']"
    );
    if (missing) {
      var error = document.getElementById("page-error");
      if (error) {
        error.textContent = missing.closest(".sidebar-search")
          ? "Not ported yet: the admin search."
          : "Not ported yet: the design wizard.";
        error.hidden = false;
      }
      return;
    }
    if (event.target.closest(".sidebar-section-header-collapsable")) {
      // sidebar.js toggled the section first.
      syncToggle(panel);
    }
  });

  document.addEventListener("input", function (event) {
    var input = event.target.closest(".admin-panel .sidebar-filter__input");
    if (!input) {
      return;
    }
    var panel = input.closest(".admin-panel");
    var filter = input.value.trim().toLowerCase();
    panel.querySelectorAll(".sidebar-section[data-section-name]").forEach(function (section) {
      var shown = 0;
      section.querySelectorAll(".sidebar-section-link-wrapper").forEach(function (li) {
        var text = li.querySelector(".sidebar-section-link-content-text");
        var match = !filter || (text && text.textContent.toLowerCase().indexOf(filter) !== -1);
        li.hidden = !match;
        if (match) {
          shown += 1;
        }
      });
      section.hidden = filter !== "" && shown === 0;
      var content = section.querySelector(".sidebar-section-content");
      if (content) {
        // displaySectionContent: every section is open while filtering.
        content.hidden = filter === "" ? section.classList.contains("sidebar-section--collapsed") : false;
      }
    });
  });
})();
