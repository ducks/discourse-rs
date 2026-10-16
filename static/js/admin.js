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

  // HorizontalScrollSyncWrapper of the admin tables: a scrollbar above the
  // table, as wide as it, while its bottom is out of view; the two scroll
  // together.
  function scrollSync(container) {
    var content = container.querySelector(".horizontal-scroll-sync__content");
    var top = container.querySelector(".horizontal-scroll-sync__top-scroll");
    var fake = container.querySelector(".horizontal-scroll-sync__fake-content");
    if (!content || !top || !fake) {
      return null;
    }
    function syncWidth() {
      fake.style.width = content.scrollWidth + "px";
      fake.style.height = "12px";
    }
    content.addEventListener("scroll", function () {
      top.scrollLeft = content.scrollLeft;
    }, { passive: true });
    top.addEventListener("scroll", function () {
      content.scrollLeft = top.scrollLeft;
    }, { passive: true });
    // A list whose rows the client loads after the wrapper is drawn (the
    // email logs) resizes once more, which shows the scrollbar again.
    var loadsLater = container.hasAttribute("data-loads-rows");
    var observer = new IntersectionObserver(function (entries) {
      var entry = entries[0];
      if (entry.boundingClientRect.bottom < entry.rootBounds.height) {
        fake.style.height = "0";
      } else {
        syncWidth();
      }
      if (loadsLater) {
        loadsLater = false;
        syncWidth();
      }
    }, { root: null, threshold: 0 });
    observer.observe(content);
    syncWidth();
    // dOnResize: a later change of the content's size syncs the width
    // again (its first report comes before the intersection's).
    var first = true;
    var resize = new ResizeObserver(function () {
      if (first) {
        first = false;
        return;
      }
      syncWidth();
    });
    resize.observe(content);
    return {
      disconnect: function () {
        observer.disconnect();
        resize.disconnect();
      },
    };
  }

  Discourse.onPage(function () {
    document.querySelectorAll(".admin-panel").forEach(function (panel) {
      shortcutHint(panel);
      syncToggle(panel);
    });
    var observers = [];
    document
      .querySelectorAll(".admin-contents .horizontal-scroll-sync__container")
      .forEach(function (container) {
        var observer = scrollSync(container);
        if (observer) {
          observers.push(observer);
        }
      });
    return function () {
      observers.forEach(function (o) {
        o.disconnect();
      });
    };
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

  // DPageHeader's and DPageSubheader's More options in the mobile view:
  // the actions as a menu, each item clicking the button it stands for.
  document.addEventListener("click", function (event) {
    var trigger = event.target.closest(
      ".d-page-header-mobile-actions-trigger, .d-page-subheader-mobile-actions-trigger"
    );
    if (trigger) {
      var template = trigger.parentElement.querySelector("template.d-page-mobile-actions");
      var hidden = trigger.parentElement.querySelector("div[hidden]");
      if (!template || !Discourse.float) {
        return;
      }
      var content = template.content.firstElementChild.cloneNode(true);
      var menu = Discourse.float.menu(trigger, {
        identifier: trigger.dataset.identifier,
        content: content,
      });
      content.addEventListener("click", function (e) {
        var item = e.target.closest("[data-page-action]");
        if (!item) {
          return;
        }
        if (menu && menu.close) {
          menu.close();
        }
        var cls = item.dataset.pageAction.split(" ")[0];
        var button = hidden && hidden.querySelector("." + cls);
        if (button) {
          button.click();
        }
      });
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
