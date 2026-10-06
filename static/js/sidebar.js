// The sidebar's client state, as Ember keeps it: the header button hides
// the sidebar (discourse_sidebar-hidden), a section header collapses its
// section (discourse_sidebar-section-<name>-collapsed), and More opens the
// secondary community links. Runs right after the sidebar is parsed, so the
// stored state applies before the page paints.
(function () {
  "use strict";

  var PREFIX = "discourse_";

  function getItem(key) {
    try {
      return window.localStorage.getItem(PREFIX + key);
    } catch (e) {
      return null;
    }
  }

  function setItem(key, value) {
    try {
      window.localStorage.setItem(PREFIX + key, value);
    } catch (e) {
      // Storage blocked: the state lasts for this page only.
    }
  }

  function removeItem(key) {
    try {
      window.localStorage.removeItem(PREFIX + key);
    } catch (e) {
      // As above.
    }
  }

  function collapsedKey(name) {
    return "sidebar-section-" + name + "-collapsed";
  }

  function setSidebarShown(shown) {
    var nav = document.getElementById("d-sidebar");
    var button = document.querySelector(".btn-sidebar-toggle");
    if (!nav) {
      return;
    }
    nav.hidden = !shown;
    document.body.classList.toggle("has-sidebar-page", shown);
    if (button) {
      button.setAttribute("aria-expanded", shown ? "true" : "false");
    }
  }

  function setSectionExpanded(section, expanded) {
    var button = section.querySelector(".sidebar-section-header-collapsable");
    var content = section.querySelector(".sidebar-section-content");
    var caret = section.querySelector(".sidebar-section-header-caret svg");
    var icon = expanded ? "angle-down" : "angle-right";
    section.classList.toggle("sidebar-section--expanded", expanded);
    section.classList.toggle("sidebar-section--collapsed", !expanded);
    if (button) {
      button.setAttribute("aria-expanded", expanded ? "true" : "false");
    }
    if (content) {
      content.hidden = !expanded;
    }
    if (caret) {
      caret.setAttribute(
        "class",
        caret
          .getAttribute("class")
          .replace(/d-icon-angle-(down|right)/, "d-icon-" + icon)
      );
      caret.querySelector("use").setAttribute("href", "#" + icon);
    }
  }

  function setMenuOpen(trigger, open) {
    var menu = trigger.parentElement.querySelector(".fk-d-menu");
    trigger.setAttribute("aria-expanded", open ? "true" : "false");
    trigger.classList.toggle("-expanded", open);
    if (menu) {
      menu.hidden = !open;
      menu.classList.toggle("-expanded", open);
    }
  }

  function closeMenus(except) {
    document
      .querySelectorAll(".sidebar-more-section-trigger[aria-expanded='true']")
      .forEach(function (trigger) {
        if (trigger !== except) {
          setMenuOpen(trigger, false);
        }
      });
  }

  // Stored state. A section holding the current page stays open
  // (expandWhenActive).
  if (getItem("sidebar-hidden") === "true") {
    setSidebarShown(false);
  }
  document
    .querySelectorAll(".sidebar-section[data-section-name]")
    .forEach(function (section) {
      var name = section.getAttribute("data-section-name");
      if (
        section.querySelector(".sidebar-section-header-collapsable") &&
        getItem(collapsedKey(name)) === "true" &&
        !section.querySelector(".sidebar-section-link.active")
      ) {
        setSectionExpanded(section, false);
      }
    });

  document.addEventListener("click", function (event) {
    var toggle = event.target.closest(".btn-sidebar-toggle");
    if (toggle) {
      var nav = document.getElementById("d-sidebar");
      var shown = nav && nav.hidden;
      setSidebarShown(shown);
      if (shown) {
        removeItem("sidebar-hidden");
      } else {
        setItem("sidebar-hidden", "true");
      }
      return;
    }

    var header = event.target.closest(".sidebar-section-header-collapsable");
    if (header) {
      var section = header.closest(".sidebar-section");
      var expanded = !section.classList.contains("sidebar-section--expanded");
      setSectionExpanded(section, expanded);
      setItem(
        collapsedKey(section.getAttribute("data-section-name")),
        expanded ? "false" : "true"
      );
      header.blur();
      return;
    }

    var trigger = event.target.closest(".sidebar-more-section-trigger");
    if (trigger) {
      var open = trigger.getAttribute("aria-expanded") !== "true";
      closeMenus(trigger);
      setMenuOpen(trigger, open);
      return;
    }

    if (!event.target.closest(".sidebar-more-section-content")) {
      closeMenus(null);
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeMenus(null);
    }
  });
})();
