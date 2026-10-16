// The sidebar's client state, as Ember keeps it: the header button hides
// the sidebar (discourse_sidebar-hidden), a section header collapses its
// section (discourse_sidebar-section-<name>-collapsed), and More opens the
// secondary community links. In the mobile view the header's hamburger
// opens the same sections in a slide-in panel (HamburgerDropdownWrapper),
// from the #hamburger-dropdown template. Runs right after the sidebar is
// parsed, and again on each page swapped in (page.js), so the stored state
// applies before the page paints.
(function () {
  "use strict";

  if (!Discourse.once("sidebar")) {
    return;
  }

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

  // For scripts that redraw sections (chat's, after a membership change).
  Discourse.setSidebarSectionExpanded = setSectionExpanded;

  // A section holding the current page stays open (expandWhenActive); one
  // drawn collapsed until opened (the admin panel's, collapsedByDefault)
  // opens when the member left it open.
  function applyCollapsed(root) {
    root
      .querySelectorAll(".sidebar-section[data-section-name]")
      .forEach(function (section) {
        var name = section.getAttribute("data-section-name");
        if (!section.querySelector(".sidebar-section-header-collapsable")) {
          return;
        }
        var stored = getItem(collapsedKey(name));
        if (stored === "true" && !section.querySelector(".sidebar-section-link.active")) {
          setSectionExpanded(section, false);
        } else if (stored === "false" && section.classList.contains("sidebar-section--collapsed")) {
          setSectionExpanded(section, true);
        }
      });
  }

  // Stored state.
  Discourse.onPage(function () {
    if (getItem("sidebar-hidden") === "true") {
      setSidebarShown(false);
    }
    applyCollapsed(document);
    revealActiveLink();
  });

  // SidebarFooter's buttons open modals not ported yet.
  document.addEventListener("click", function (event) {
    var button = event.target.closest && event.target.closest(".sidebar-footer-actions-button");
    if (!button) {
      return;
    }
    var what = button.classList.contains("add-section")
      ? "adding a custom sidebar section"
      : "the keyboard shortcuts help";
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = "Not ported yet: " + what + ".";
      error.hidden = false;
    }
  });

  // SectionLink's revealLink for a panel that scrolls its active link
  // into view (the admin panel's scrollActiveLinkIntoView): the link's
  // item, when the scrolled sections don't show all of it.
  function revealActiveLink() {
    var link = document.querySelector(
      "#d-sidebar .sidebar-sections.admin-panel .sidebar-section-link.active"
    );
    var item = link && link.closest(".sidebar-section-link-wrapper");
    if (!item) {
      return;
    }
    // Ember reveals the link as it is inserted, while every section is
    // still drawn open; the collapsed ones close after, which leaves the
    // scroll where that put it (or at its end).
    var closed = Array.prototype.slice.call(
      document.querySelectorAll("#d-sidebar .sidebar-sections.admin-panel .sidebar-section-content[hidden]")
    );
    closed.forEach(function (content) {
      content.hidden = false;
    });
    if (!fullyVisible(item)) {
      item.scrollIntoView({ block: "nearest", inline: "nearest" });
    }
    closed.forEach(function (content) {
      content.hidden = true;
    });
  }

  // isFullyScrolledIntoView
  function fullyVisible(element) {
    var rect = element.getBoundingClientRect();
    var node = element.parentElement;
    var scrolled = false;
    while (node && node !== document.body) {
      var overflow = getComputedStyle(node).overflowY;
      if ((overflow === "auto" || overflow === "scroll") && node.scrollHeight > node.clientHeight) {
        scrolled = true;
        var bounds = node.getBoundingClientRect();
        if (rect.top < bounds.top || rect.bottom > bounds.bottom) {
          return false;
        }
      }
      node = node.parentElement;
    }
    return scrolled || (rect.top >= 0 && rect.bottom <= window.innerHeight);
  }

  // HamburgerDropdownWrapper's CLOSE_ON_CLICK_SELECTORS: following a link
  // closes the menu.
  var CLOSE_ON_CLICK =
    "a[href], .sidebar-section-header-button, .sidebar-section-link:not(.--link-button)";

  function hamburgerOpen() {
    return !!document.querySelector(".d-header .hamburger-dropdown-wrapper");
  }

  function setHamburgerOpen(open) {
    // The mobile view's hamburger button, or in the narrow desktop view the
    // sidebar toggle (whose aria-expanded stays the hidden sidebar's).
    var toggle = document.querySelector(".hamburger-dropdown");
    var button = document.getElementById("toggle-hamburger-menu");
    var narrow = document.querySelector(".btn-sidebar-toggle.narrow-desktop");
    var template = document.getElementById("hamburger-dropdown");
    if (!template || ((!toggle || !button) && !narrow)) {
      return;
    }
    if (open && !hamburgerOpen()) {
      var icons = document.querySelector(".d-header .d-header-icons");
      icons.after(template.content.cloneNode(true));
      applyCollapsed(document.querySelector(".hamburger-dropdown-wrapper"));
      if (toggle && button) {
        toggle.className = "active " + toggle.className;
        button.setAttribute("aria-expanded", "");
      }
    } else if (!open) {
      document
        .querySelectorAll(
          ".d-header .hamburger-dropdown-wrapper, .d-header .header-cloak"
        )
        .forEach(function (element) {
          element.remove();
        });
      if (toggle && button) {
        toggle.classList.remove("active");
        button.removeAttribute("aria-expanded");
      }
    }
  }

  document.addEventListener("click", function (event) {
    // Header#toggleNavigationMenu: the hamburger in the narrow desktop
    // view, where the sidebar is not shown.
    if (event.target.closest("#toggle-hamburger-menu, .btn-sidebar-toggle.narrow-desktop")) {
      setHamburgerOpen(!hamburgerOpen());
      return;
    }
    if (hamburgerOpen()) {
      if (event.target.closest(".header-cloak")) {
        setHamburgerOpen(false);
        return;
      }
      if (
        event.target.closest(".hamburger-panel") &&
        event.target.closest(CLOSE_ON_CLICK)
      ) {
        setHamburgerOpen(false);
      }
    }

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
    // In the mobile view More is a bottom sheet (its DMenu's
    // modalForMobile), with the links the hidden menu beside it holds.
    if (trigger && document.documentElement.classList.contains("mobile-view")) {
      var links = trigger.parentElement.querySelector(".sidebar-more-section-content .dropdown-menu");
      if (links && Discourse.float) {
        var content = links.cloneNode(true);
        var sheet = Discourse.float.menu(trigger, {
          identifier: "sidebar-more-section",
          modalForMobile: true,
          content: content,
        });
        content.addEventListener("click", function (e) {
          if (e.target.closest("a[href]")) {
            sheet.close();
          }
        });
      }
      return;
    }
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
      setHamburgerOpen(false);
    }
  });
})();
