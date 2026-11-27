// The setup wizard's step in the browser (static/wizard's WizardStep and
// its fields): the URL becomes the step's (wizard.index's replaceWith),
// a radio choice marks itself selected, the dropdown is combo-box.js's
// ComboBox, and Jump in validates the required fields, PUTs the step's
// fields to /wizard/steps/:id and goes to the home page, or shows each
// field's error from the response.
(function () {
  "use strict";

  if (!Discourse.once("wizard")) {
    return;
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    return (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
  }

  function stepButton() {
    return document.querySelector("#wizard-main .jump-in[data-step-id]");
  }

  Discourse.onPage(function () {
    var button = stepButton();
    if (!button) {
      return;
    }
    var path = base() + "/wizard/steps/" + button.dataset.stepId;
    if (location.pathname !== path) {
      history.replaceState(history.state, "", path + location.search + location.hash);
    }
  });

  // Radio: the chosen choice is --selected.
  document.addEventListener("change", function (event) {
    var input = event.target.closest("#wizard-main .wizard-container__radio");
    if (!input) {
      return;
    }
    var choices = input.closest(".wizard-container__radio-choices");
    choices.querySelectorAll(".wizard-container__radio-choice").forEach(function (choice) {
      choice.classList.toggle("--selected", choice.dataset.choiceId === input.value && input.checked);
    });
  });

  // Dropdown: the ComboBox opens under its header, a row selects, and a
  // click elsewhere closes it.
  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches("#wizard-main details.wizard-container__dropdown")) {
        return;
      }
      Discourse.comboBox.expanded(chooser);
      if (chooser.open) {
        Discourse.comboBox.float(chooser);
      }
    },
    true
  );

  document.addEventListener("click", function (event) {
    var row = event.target.closest("#wizard-main details.wizard-container__dropdown .select-kit-row");
    document.querySelectorAll("#wizard-main details.wizard-container__dropdown[open]").forEach(function (chooser) {
      if (!chooser.contains(event.target) || row) {
        chooser.open = false;
      }
    });
    if (row) {
      Discourse.comboBox.select(row.closest("details"), row);
      return;
    }
    if (event.target.closest("#wizard-main .jump-in")) {
      jumpIn();
    }
  });

  function fields() {
    return Array.prototype.slice.call(document.querySelectorAll("#wizard-main .wizard-container__field[data-field-id]"));
  }

  // Field#value as the step serializes it.
  function value(field) {
    switch (field.dataset.fieldType) {
      case "text":
        return field.querySelector(".wizard-container__text-input").value;
      case "radio": {
        var checked = field.querySelector(".wizard-container__radio:checked");
        return checked ? checked.value : null;
      }
      default:
        return Discourse.comboBox.value(field.querySelector("details.wizard-container__dropdown"));
    }
  }

  // Field#errorDescription: below the input, the field invalid.
  function setError(field, description) {
    field.classList.add("invalid");
    var error = field.querySelector(".wizard-container__description.error");
    if (!error) {
      error = document.createElement("div");
      error.className = "wizard-container__description error";
      field.querySelector(".wizard-container__input").insertAdjacentElement("afterend", error);
    }
    error.innerHTML = description;
  }

  // Step#validate: a required field needs a value; validating clears the
  // errors a save gave.
  function validate() {
    var valid = true;
    fields().forEach(function (field) {
      var ok = !("required" in field.dataset) || (value(field) || "").length > 0;
      field.classList.toggle("invalid", !ok);
      var error = field.querySelector(".wizard-container__description.error");
      if (error) {
        error.remove();
      }
      valid = valid && ok;
    });
    return valid;
  }

  var saving = false;

  function jumpIn() {
    if (saving || !validate()) {
      return;
    }
    var button = stepButton();
    var body = new URLSearchParams();
    fields().forEach(function (field) {
      var v = value(field);
      body.append("fields[" + field.dataset.fieldId + "]", v === null ? "" : v);
    });
    saving = true;
    fetch(base() + "/wizard/steps/" + encodeURIComponent(button.dataset.stepId), {
      method: "PUT",
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json, text/javascript, */*; q=0.01",
      },
      body: body.toString(),
    })
      .then(function (response) {
        return response.json().then(
          function (json) {
            return { ok: response.ok, json: json };
          },
          function () {
            return { ok: response.ok, json: null };
          }
        );
      })
      .then(function (result) {
        saving = false;
        if (result.ok && result.json && result.json.success) {
          // A full load, as Ember's, for the language and title.
          document.location = base() + "/";
          return;
        }
        var errors = (result.json && result.json.errors) || [];
        errors.forEach(function (err) {
          var field = fields().find(function (f) {
            return f.dataset.fieldId === err.field;
          });
          if (field) {
            setError(field, err.description);
          }
        });
        if (!errors.length) {
          throw new Error("the wizard step's save failed without field errors");
        }
      })
      .catch(function (error) {
        saving = false;
        throw error;
      });
  }
})();
