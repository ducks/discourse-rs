// The interface color selector (InterfaceColorSelector and the
// InterfaceColor service): its trigger, in the header or the sidebar
// footer, opens a DMenu with Light, Dark and Auto. A choice writes the
// forced_color_mode cookie for a year (lib/cookie) and flips the color
// definitions stylesheets' media as forceLightMode, forceDarkMode and
// useAutoMode do; the server reads the cookie on the next page
// (crate::color_mode). The labels come from the script tag.
(function () {
  "use strict";

  if (!Discourse.once("interface-color")) {
    return;
  }

  var script = document.currentScript;
  var labels = {
    light: script.dataset.light,
    dark: script.dataset.dark,
    auto: script.dataset.auto,
  };
  var ariaLabel = script.dataset.ariaLabel;
  var ICONS = { light: "sun", dark: "moon", auto: "circle-half-stroke" };
  var menu = null;

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function escape(s) {
    var div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }

  function content() {
    var items = ["light", "dark", "auto"].map(function (mode) {
      return (
        '<li class="dropdown-menu__item"><button class="btn btn-icon-text interface-color-selector__' +
        mode +
        '-option" type="button" data-mode="' +
        mode +
        '">' +
        icon(ICONS[mode]) +
        '<span class="d-button-label">' +
        escape(labels[mode]) +
        "</span></button></li>"
      );
    });
    return '<ul class="dropdown-menu">' + items.join("") + "</ul>";
  }

  function setCookie(mode) {
    var expires = new Date();
    expires.setTime(+expires + 365 * 864e5);
    document.cookie =
      "forced_color_mode=" + mode + "; expires=" + expires.toUTCString() + "; path=/;samesite=Lax";
  }

  function setMedia(light, dark) {
    var lightSheet = document.querySelector("link.light-scheme");
    var darkSheet = document.querySelector("link.dark-scheme");
    if (lightSheet && darkSheet) {
      lightSheet.media = light;
      darkSheet.media = dark;
    }
  }

  // The selector's icon and label follow the mode, light and dark with
  // their own icons, auto with the half circle.
  function updateTriggers(mode) {
    document.querySelectorAll(".interface-color-selector").forEach(function (trigger) {
      trigger.dataset.currentMode = mode;
      trigger.setAttribute("aria-label", ariaLabel.replace("%{mode}", mode));
      var svg = trigger.querySelector("svg");
      if (svg) {
        svg.outerHTML = icon(mode === "auto" ? ICONS.auto : ICONS[mode]);
      }
    });
  }

  function choose(mode) {
    setCookie(mode);
    if (mode === "light") {
      setMedia("all", "none");
    } else if (mode === "dark") {
      setMedia("none", "all");
    } else {
      setMedia("(prefers-color-scheme: light)", "(prefers-color-scheme: dark)");
    }
    updateTriggers(mode);
  }

  document.addEventListener("click", function (event) {
    var option = event.target.closest(".interface-color-selector-content [data-mode]");
    if (option) {
      choose(option.dataset.mode);
      if (menu) {
        menu.close();
        menu = null;
      }
      return;
    }
    var trigger = event.target.closest(".interface-color-selector");
    if (!trigger || !Discourse.float) {
      return;
    }
    if (menu) {
      menu.close();
      return;
    }
    // DMenu's trigger is expanded while its menu is open, -expanded after
    // fk-d-menu__trigger in its classes.
    trigger.className = trigger.className.replace("fk-d-menu__trigger", "fk-d-menu__trigger -expanded");
    trigger.setAttribute("aria-expanded", "true");
    menu = Discourse.float.menu(trigger, {
      identifier: "interface-color-selector",
      mainClass: "interface-color-selector-content",
      ariaLabel: trigger.getAttribute("aria-label"),
      animated: false,
      content: content(),
      onClose: function () {
        trigger.classList.remove("-expanded");
        trigger.setAttribute("aria-expanded", "false");
        menu = null;
      },
    });
  });
})();
