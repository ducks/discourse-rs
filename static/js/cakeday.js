// discourse-cakeday's UserDateOfBirthInput: the month and day combo
// boxes set the date of birth the profile saves, "1904-M-D" once both are
// chosen, blank otherwise (static/js/preferences.js collects it).
(function () {
  "use strict";

  function value(input, id) {
    var summary = input.querySelector("#" + id + " summary");
    return summary && summary.dataset.value !== undefined ? summary.dataset.value : null;
  }

  document.addEventListener("prefs-change", function (event) {
    var input = event.target.closest(".user-date-of-birth-input");
    if (!input) {
      return;
    }
    var month = value(input, "cakeday-date-of-birth-month");
    var day = value(input, "cakeday-date-of-birth-day");
    var field = input.querySelector('input[data-pref="date_of_birth"]');
    field.value = month && day ? "1904-" + month + "-" + day : "";
    field.dataset.dirty = "true";
  });
})();
