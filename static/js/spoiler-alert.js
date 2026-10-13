// spoiler-alert's decorator (initializers/spoiler-alert.js, when
// spoiler_enabled): each div.spoiler the server cooked becomes .spoiled
// and is blurred until clicked, as lib/apply-spoiler.js does.
(function () {
  "use strict";

  var Discourse = window.Discourse;
  if (!Discourse || !Discourse.decorateCooked || Discourse.spoilerAlert) {
    return;
  }
  Discourse.spoilerAlert = true;

  var INTERACTIVE_SELECTOR = [
    "a",
    "area",
    "audio",
    "button",
    "details",
    "embed",
    "iframe",
    "img.animated",
    "input",
    "map",
    "object",
    "option",
    "portal",
    "select",
    "textarea",
    "track",
    "video",
    ".lightbox",
  ].join(", ");

  function isInteractive(event, element) {
    if (event.defaultPrevented) {
      return true;
    }
    var interactive = event.target.closest(INTERACTIVE_SELECTOR);
    return interactive && element.contains(interactive);
  }

  function noTextSelected() {
    return window.getSelection() + "" === "";
  }

  // null removes the attribute.
  function setAttributes(element, attributes) {
    Object.keys(attributes).forEach(function (key) {
      var value = attributes[key];
      if (value === null) {
        element.removeAttribute(key);
      } else {
        element.setAttribute(key, value);
      }
    });
  }

  function setSpoilerHidden(element) {
    setAttributes(element, {
      role: "button",
      tabindex: "0",
      "data-spoiler-state": "blurred",
      "aria-expanded": false,
      "aria-label": Discourse.cooked.t("spoiler.label.show"),
      "aria-live": "polite",
    });
    element.classList.add("spoiler-blurred");
    Array.prototype.forEach.call(element.children, function (e) {
      e.setAttribute("aria-hidden", true);
    });
  }

  function setSpoilerVisible(element) {
    setAttributes(element, {
      "data-spoiler-state": "revealed",
      "aria-expanded": true,
      "aria-label": null,
      role: null,
    });
    element.classList.remove("spoiler-blurred");
    Array.prototype.forEach.call(element.children, function (e) {
      e.removeAttribute("aria-hidden");
    });
  }

  // Revealed by any click or Enter; blurred again only by one that isn't
  // on something interactive in it and doesn't end a text selection.
  function toggleSpoiler(event, element) {
    if (element.getAttribute("data-spoiler-state") === "blurred") {
      setSpoilerVisible(element);
      event.preventDefault();
    } else if (!isInteractive(event, element) && noTextSelected()) {
      setSpoilerHidden(element);
    }
  }

  function applySpoiler(element) {
    setSpoilerHidden(element);
    element.addEventListener("click", function (event) {
      toggleSpoiler(event, element);
    });
    element.addEventListener("keydown", function (event) {
      if (event.key === "Enter") {
        toggleSpoiler(event, element);
      }
    });
  }

  if (!Discourse.cooked.setting("spoiler_enabled")) {
    return;
  }

  Discourse.decorateCooked("spoiler-alert", function (el) {
    el.querySelectorAll(".spoiler").forEach(function (spoiler) {
      spoiler.classList.remove("spoiler");
      spoiler.classList.add("spoiled");
      applySpoiler(spoiler);
    });
  });
})();
