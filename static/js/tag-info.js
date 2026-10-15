// A tag's page (tags/show): the info button shows and hides the TagInfo
// panel above the list; the edit button opens the tag's settings page.
(function () {
  "use strict";

  if (!Discourse.once("tag-info")) {
    return;
  }

  document.addEventListener("click", function (event) {
    var info = event.target.closest("#show-tag-info");
    if (info) {
      var panel = document.querySelector("section.tag-info");
      if (!panel) {
        return;
      }
      var shown = panel.hidden;
      panel.hidden = !shown;
      info.setAttribute("aria-pressed", shown ? "true" : "false");
      return;
    }
    var edit = event.target.closest("#edit-tag");
    if (edit && edit.dataset.href) {
      window.location.href = edit.dataset.href;
    }
  });
})();
