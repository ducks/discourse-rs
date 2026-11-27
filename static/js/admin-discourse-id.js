// The Discourse ID tab (src/admin/routes/discourse_id.rs over
// templates/admin-config/login/discourse-id.gjs): the switch PUTs its
// `enabled` setting, turned back on a failure, and Regenerate confirms,
// POSTs and toasts its success.
(function () {
  "use strict";

  if (!Discourse.once("admin-discourse-id")) {
    return;
  }

  var ui = Discourse.adminUi;
  var BASE = "/admin/config/login-and-authentication/discourse-id";

  function root() {
    return document.querySelector(".discourse-id-admin");
  }

  function setSwitch(button, on) {
    button.setAttribute("aria-checked", on ? "true" : "false");
    button.parentNode.querySelector(".d-toggle-switch__checkbox-slider").innerHTML = on ? ui.icon("check") : "";
  }

  function toggle(button) {
    var on = button.getAttribute("aria-checked") !== "true";
    setSwitch(button, on);
    ui.ajax("PUT", BASE + "/settings", { enabled: on }).catch(function (error) {
      setSwitch(button, !on);
      ui.failed(error);
    });
  }

  function regenerate(button) {
    ui.strings("discourse-id-strings");
    Discourse.dialog.confirm({
      message: ui.t("admin.config.discourse_id.regenerate_confirm"),
      didConfirm: function () {
        button.disabled = true;
        button.classList.add("is-loading");
        ui.ajax("POST", BASE + "/regenerate")
          .then(function () {
            ui.toast(ui.t("admin.config.discourse_id.regenerate_success"));
          }, ui.failed)
          .then(function () {
            button.disabled = false;
            button.classList.remove("is-loading");
          });
      },
    });
  }

  document.addEventListener("click", function (event) {
    var page = root();
    if (!page || !page.contains(event.target)) {
      return;
    }
    var button = event.target.closest(".discourse-id-toggle");
    if (button) {
      toggle(button);
      return;
    }
    var regen = event.target.closest(".discourse-id-regenerate");
    if (regen && !regen.disabled) {
      regenerate(regen);
    }
  });
})();
