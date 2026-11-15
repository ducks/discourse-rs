// UserFieldsValidationHelper (lib/user-fields-validation-helper.js) over
// the UserFields the server drew in a form's .user-fields, for the pages
// that ask for them (login.js's signup form, invite.js's invite form; the
// manifest puts this file ahead of each).
//
//   var helper = Discourse.userFields(form, fields, options)
//     fields              site.user_fields shown on signup, by position
//     options.password()  getAccountPassword
//     options.t(key, args)  the reasons' strings, where they show
//     options.tips        the components are passed their validation and
//                         draw its tip (the signup form's are; the invite
//                         form's only keep its button disabled)
//     options.changed()   after a field's value changes
//   helper.fields         TrackedUserFields: field, control, component, element
//   helper.visible        validationVisible
//   helper.validation()   userFieldsValidation: the first field that fails
//   helper.draw()         value-entered, and the tips
//   helper.values()       each field's value by id, null as jQuery sends
//                         it, empty
(function () {
  "use strict";

  if (!Discourse.once("user-fields")) {
    return;
  }

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML;
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  // DInputTip without an id.
  function tip(validation, hidden) {
    var div = document.createElement("div");
    div.className = "tip " + (validation.failed ? "bad" : "good") + (hidden ? " hidden" : "");
    div.innerHTML = validation.reason ? icon(validation.failed ? "xmark" : "check") + escape(validation.reason) : "";
    return div;
  }

  Discourse.userFields = function (form, fields, options) {
    var helper = { visible: false };

    // `element` is UserFieldBase's field.element: the dropdown's header,
    // else the component's input.
    helper.fields = (fields || [])
      .map(function (field) {
        var control = form.querySelector('[data-user-field="' + field.id + '"]');
        if (!control) {
          return null;
        }
        var component = control.closest(".user-field");
        return {
          field: field,
          control: control,
          component: component,
          element: field.field_type === "dropdown" ? control.querySelector(".select-kit-header") : component.querySelector("input"),
          instructions: component.querySelector(".instructions"),
          tip: null,
        };
      })
      .filter(Boolean);

    // The component's value: null until a multiselect is touched, as the
    // helper starts every field.
    function value(f) {
      var control = f.control;
      switch (f.field.field_type) {
        case "confirm":
          return control.checked;
        case "dropdown":
          return Discourse.comboBox.value(control);
        case "multiselect":
          var items = JSON.parse(control.dataset.items || "[]");
          if (!items.length && control.dataset.dirty !== "true") {
            return null;
          }
          return items.map(function (item) {
            return item[0];
          });
        default:
          return control.value;
      }
    }

    function reason(key, args) {
      return options.t ? options.t(key, args) : undefined;
    }

    // TrackedUserField#validation
    function validation(f) {
      if (!helper.visible) {
        return { ok: true };
      }
      var v = value(f);
      var type = f.field.field_type;
      if (f.field.required && (!v || v.length === 0)) {
        var key =
          type === "confirm"
            ? "user_fields.required_checkbox"
            : type === "text" || type === "date"
              ? "user_fields.required"
              : "user_fields.required_select";
        return { failed: true, element: f.element, reason: reason(key, { name: f.field.name }) };
      }
      var password = options.password() || "";
      if (password && type === "text" && v && v.toLowerCase().indexOf(password.toLowerCase()) !== -1) {
        return { failed: true, element: f.element, reason: reason("user_fields.same_as_password") };
      }
      return { ok: true };
    }

    helper.validation = function () {
      for (var i = 0; i < helper.fields.length; i++) {
        var v = validation(helper.fields[i]);
        if (v.failed) {
          return v;
        }
      }
      return { ok: true };
    };

    // Each UserField: value-entered, and with tips its validation's. The
    // confirm's is always there, hidden without a reason; the others'
    // takes the instructions' place while the field fails.
    helper.draw = function () {
      helper.fields.forEach(function (f) {
        var v = value(f);
        f.component.classList.toggle("value-entered", !!v && v.length > 0);
        if (!options.tips) {
          return;
        }
        var result = validation(f);
        if (f.field.field_type === "confirm") {
          var old = f.component.querySelector(".controls > .tip");
          if (old) {
            old.replaceWith(tip(result, !result.reason));
          }
          return;
        }
        if (f.tip) {
          f.tip.remove();
          f.tip = null;
        }
        var controls = f.component.querySelector(".controls");
        if (result.failed) {
          f.tip = tip(result, false);
          f.instructions.remove();
          controls.appendChild(f.tip);
        } else if (!f.instructions.parentNode) {
          controls.appendChild(f.instructions);
        }
      });
    };

    helper.values = function () {
      var out = {};
      helper.fields.forEach(function (f) {
        var v = value(f);
        out[f.field.id] = v === null ? "" : v;
      });
      return out;
    };

    // A field's value changing: its inputs, a dropdown's row
    // (combo-box.js's prefs-change), a multiselect's choices.
    ["input", "change", "prefs-change"].forEach(function (type) {
      form.addEventListener(type, function (event) {
        if (event.target.closest && event.target.closest(".user-fields") && !event.target.closest(".select-kit-body")) {
          helper.draw();
          if (options.changed) {
            options.changed();
          }
        }
      });
    });
    // The dropdowns: ComboBox's events over combo-box.js.
    if (form.querySelector(".user-fields details.prefs-combo")) {
      form.addEventListener(
        "toggle",
        function (event) {
          var chooser = event.target;
          if (!chooser.matches || !chooser.matches("details.prefs-combo")) {
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
        var row = event.target.closest(".user-fields details.prefs-combo .select-kit-row");
        form.querySelectorAll(".user-fields details.prefs-combo[open]").forEach(function (chooser) {
          if (!chooser.contains(event.target) || row) {
            chooser.open = false;
          }
        });
        if (row) {
          Discourse.comboBox.select(row.closest("details"), row);
        }
      });
    }

    return helper;
  };
})();
