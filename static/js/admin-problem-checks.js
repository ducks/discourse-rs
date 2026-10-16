// The admin problem checks page in the browser: ProblemCheckItem's
// Ignore and Watch, then the page again (router.refresh).
(function () {
  "use strict";

  if (!Discourse.once("admin-problem-checks")) {
    return;
  }

  var ui = Discourse.adminUi;

  document.addEventListener("click", function (event) {
    var button = event.target.closest(".admin-problem-checks__items button[data-problem-check-id]");
    if (!button) {
      return;
    }
    ui.ajax(
      "PUT",
      "/admin/problem_checks/" + encodeURIComponent(button.dataset.problemCheckId) + "/" + button.dataset.action + ".json"
    )
      .then(function () {
        location.reload();
      })
      .catch(ui.failed);
  });
})();
