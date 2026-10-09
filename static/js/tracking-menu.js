// The notification level menu (DMenu) a topic's and a category's
// notification buttons open: one for the page, moved to the body and
// floated under the trigger that opened it (bottom-start, or bottom-end
// as the menu's data-placement says), 10px below as float-kit's offset
// puts it; a level saved through htmx updates every trigger.
(function () {
  "use strict";

  if (!Discourse.once("tracking-menu")) {
    return;
  }

  // The page's menu, found again on each page swapped in (page.js).
  var trackingMenu = null;
  var trackingTrigger = null;
  Discourse.onPage(function () {
    trackingMenu = document.querySelector(".notifications-tracking-content");
    trackingTrigger = null;
  });

  function closeTrackingMenu() {
    if (!trackingTrigger) {
      return;
    }
    trackingMenu.hidden = true;
    trackingMenu.classList.remove("-expanded");
    trackingMenu.classList.remove("topic-timeline-notifications-tracking-content");
    trackingTrigger.setAttribute("aria-expanded", "false");
    trackingTrigger = null;
  }

  function openTrackingMenu(trigger) {
    if (trackingMenu.parentElement !== document.body) {
      document.body.appendChild(trackingMenu);
    }
    var rect = trigger.getBoundingClientRect();
    trackingMenu.style.position = "absolute";
    trackingMenu.style.maxWidth = "min(400px, -20px + 100dvw)";
    trackingMenu.style.left = rect.left + window.scrollX + "px";
    trackingMenu.style.top = rect.bottom + window.scrollY + 10 + "px";
    trackingMenu.classList.toggle(
      "topic-timeline-notifications-tracking-content",
      !!trigger.closest(".timeline-footer-controls")
    );
    trackingMenu.hidden = false;
    if (trackingMenu.dataset.placement === "bottom-end") {
      trackingMenu.style.left =
        rect.right + window.scrollX - trackingMenu.offsetWidth + "px";
    }
    trackingMenu.classList.add("-expanded");
    trigger.setAttribute("aria-expanded", "true");
    trackingTrigger = trigger;
  }

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeTrackingMenu();
    }
  });

  // A level saved (TopicDetails#updateNotifications, which clears the
  // reason, or the category's): every trigger and the footer's reason
  // follow.
  document.addEventListener("htmx:afterRequest", function (event) {
    var option = event.target.closest(".notifications-tracking-btn");
    if (!option || !event.detail.successful) {
      return;
    }
    var icon = option.querySelector(".notifications-tracking-btn__icons svg");
    document
      .querySelectorAll(".notifications-tracking-trigger")
      .forEach(function (trigger) {
        trigger.dataset.levelId = option.dataset.levelId;
        trigger.dataset.levelName = option.dataset.levelName;
        trigger.title = option.dataset.tooltip;
        trigger.querySelector("svg").replaceWith(icon.cloneNode(true));
        var label = trigger.querySelector(".d-button-label");
        if (label) {
          label.textContent = option.dataset.title + " ";
        }
      });
    document
      .querySelectorAll(".notifications-button-footer .text")
      .forEach(function (text) {
        text.innerHTML = option.dataset.reason;
      });
    trackingMenu
      .querySelectorAll(".notifications-tracking-btn")
      .forEach(function (b) {
        b.classList.toggle("-selected", b === option);
      });
    closeTrackingMenu();
  });

  document.addEventListener("click", function (event) {
    if (trackingMenu) {
      var trigger = event.target.closest(".notifications-tracking-trigger");
      if (trigger) {
        var reopen = trigger !== trackingTrigger;
        closeTrackingMenu();
        if (reopen) {
          openTrackingMenu(trigger);
        }
        return;
      }
      if (!event.target.closest(".notifications-tracking-content")) {
        closeTrackingMenu();
      }
    }
  });
})();
