// The messages pages' inboxes menu (MessagesDropdown, a DMenu): moved to
// the body and floated under its trigger (bottom-start, 10px below as
// float-kit's offset puts it); a row goes to its inbox.
(function () {
  "use strict";

  if (!Discourse.once("messages-dropdown")) {
    return;
  }

  var menu = null;
  var trigger = null;
  Discourse.onPage(function () {
    menu = document.querySelector(".messages-dropdown-content");
    trigger = null;
    return function () {
      // The menu lives on the body; it goes with the page.
      if (menu && menu.parentElement === document.body) {
        menu.remove();
      }
      menu = null;
    };
  });

  function close() {
    if (!trigger) {
      return;
    }
    menu.hidden = true;
    menu.style.display = "none";
    menu.classList.remove("-expanded");
    trigger.setAttribute("aria-expanded", "false");
    trigger = null;
  }

  function open(button) {
    if (menu.parentElement !== document.body) {
      document.body.appendChild(menu);
    }
    var rect = button.getBoundingClientRect();
    menu.style.position = "absolute";
    menu.style.maxWidth = "min(400px, -20px + 100dvw)";
    menu.style.left = rect.left + window.scrollX + "px";
    menu.style.top = rect.bottom + window.scrollY + 10 + "px";
    menu.hidden = false;
    menu.style.display = "";
    menu.classList.add("-expanded");
    button.setAttribute("aria-expanded", "true");
    trigger = button;
  }

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      close();
    }
  });

  document.addEventListener("click", function (event) {
    if (!menu) {
      return;
    }
    var button = event.target.closest(".messages-dropdown-trigger");
    if (button) {
      var reopen = button !== trigger;
      close();
      if (reopen) {
        open(button);
      }
      return;
    }
    var row = event.target.closest(".messages-dropdown-content [data-href]");
    if (row) {
      close();
      window.location.assign(row.dataset.href);
      return;
    }
    if (!event.target.closest(".messages-dropdown-content")) {
      close();
    }
  });
})();
