// components/wrap-attributes-modal.gjs over Discourse.modal (modal.js): a
// [wrap]'s name and its key/value attribute rows as FormKit draws them,
// Apply handing back the attributes string (serializeFromForm), and Unwrap
// where the model has onRemove.
//
//   showWrapAttributesModal({ initialAttributes, onApply, onRemove }, i18n)
import { iconHtml } from "discourse/rich-editor/toolbar";
import { parseAttributesString, serializeFromForm } from "discourse/rich-editor/wrap-utils";

function escapeHtml(s) {
  return String(s === null || s === undefined ? "" : s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

var ids = 0;

function field(name, title, value, opts) {
  var id = "wrap-modal-field-" + ++ids;
  return (
    '<div class="form-kit__container form-kit__field form-kit__field-input' +
    (opts && opts.required ? " --required" : "") +
    '" data-name="' +
    escapeHtml(name) +
    '"><label class="form-kit__container-title" for="' +
    id +
    '"><span>' +
    escapeHtml(title) +
    '</span></label><div class="form-kit__container-content"><div class="form-kit__input-wrapper">' +
    '<input type="text" class="form-kit__control-input" id="' +
    id +
    '" name="' +
    escapeHtml(name) +
    '" value="' +
    escapeHtml(value) +
    '"' +
    (opts && opts.autocomplete ? ' autocomplete="' + opts.autocomplete + '"' : "") +
    '></div><p class="form-kit__errors" hidden></p></div></div>'
  );
}

export function showWrapAttributesModal(model, i18n) {
  var Discourse = window.Discourse || {};
  if (!Discourse.modal) {
    return null;
  }
  var t = i18n || function (key) {
    return key;
  };
  var parsed = parseAttributesString(model.initialAttributes || "");
  var rows = Object.keys(parsed)
    .filter(function (key) {
      return key !== "wrap";
    })
    .map(function (key) {
      return { key: key, value: parsed[key] };
    });

  var form = document.createElement("form");
  form.className = "form-kit";
  form.noValidate = true;

  function rowHtml(row, index) {
    return (
      '<div class="form-kit__object"><div class="wrap-modal__attribute-row" data-index="' +
      index +
      '">' +
      field("attributes." + index + ".key", t("composer.wrap_modal.attribute_key"), row.key, { required: true }) +
      field("attributes." + index + ".value", t("composer.wrap_modal.attribute_value"), row.value, { required: true }) +
      '<button class="btn btn-icon no-text btn-default btn-small" type="button" title="' +
      escapeHtml(t("composer.wrap_modal.remove_attribute")) +
      '" data-remove="' +
      index +
      '">' +
      iconHtml("trash-can") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>'
    );
  }

  // What is typed, before a redraw.
  function readRows() {
    form.querySelectorAll(".wrap-modal__attribute-row").forEach(function (el) {
      var index = parseInt(el.dataset.index, 10);
      rows[index].key = el.querySelector('input[name$=".key"]').value;
      rows[index].value = el.querySelector('input[name$=".value"]').value;
    });
  }

  function draw() {
    var name = form.querySelector('input[name="name"]');
    var nameValue = name ? name.value : parsed.wrap || "";
    form.innerHTML =
      field("name", t("composer.wrap_modal.name_label"), nameValue, { autocomplete: "off" }) +
      '<div class="form-kit__section"><h2 class="form-kit__section-title">' +
      escapeHtml(t("composer.wrap_modal.attributes_label")) +
      "</h2>" +
      (rows.length ? "" : "<div>" + escapeHtml(t("composer.wrap_modal.no_attributes")) + "</div>") +
      '<div class="form-kit__collection">' +
      rows.map(rowHtml).join("") +
      '</div><button class="btn btn-text btn-default btn-small form-kit__button wrap-modal__add-attribute" type="button">' +
      '<span class="d-button-label">' +
      escapeHtml(t("composer.wrap_modal.add_attribute")) +
      "</span></button></div>";
  }

  // @validation="required" on each row's key and value.
  function validate() {
    var ok = true;
    form.querySelectorAll(".wrap-modal__attribute-row .form-kit__field").forEach(function (el) {
      var input = el.querySelector("input");
      var errors = el.querySelector(".form-kit__errors");
      var missing = input.value === "";
      el.classList.toggle("has-error", missing);
      errors.hidden = !missing;
      errors.textContent = missing ? t("form_kit.errors.required") : "";
      ok = ok && !missing;
    });
    return ok;
  }

  var close;

  function submit() {
    readRows();
    if (!validate()) {
      return;
    }
    model.onApply(serializeFromForm(form.querySelector('input[name="name"]').value, rows));
    close();
  }

  form.addEventListener("click", function (event) {
    var remove = event.target.closest("[data-remove]");
    if (remove) {
      readRows();
      rows.splice(parseInt(remove.dataset.remove, 10), 1);
      draw();
      return;
    }
    if (event.target.closest(".wrap-modal__add-attribute")) {
      readRows();
      rows.push({ key: "", value: "" });
      draw();
    }
  });
  form.addEventListener("submit", function (event) {
    event.preventDefault();
    submit();
  });

  draw();

  close = Discourse.modal.show({
    className: "wrap-attributes-modal",
    title: t("composer.wrap_modal.title"),
    body: function (el) {
      el.appendChild(form);
    },
    footer: function (el) {
      el.innerHTML =
        '<button class="btn btn-text btn-primary" type="button"><span class="d-button-label">' +
        escapeHtml(t("composer.wrap_modal.apply")) +
        "</span></button>" +
        '<button class="btn btn-text btn-default" type="button"><span class="d-button-label">' +
        escapeHtml(t("cancel")) +
        "</span></button>" +
        (model.onRemove
          ? '<button class="btn btn-text btn-danger wrap-attributes-modal__unwrap" type="button"><span class="d-button-label">' +
            escapeHtml(t("composer.wrap_modal.unwrap")) +
            "</span></button>"
          : "");
      el.querySelector(".btn-primary").addEventListener("click", submit);
      el.querySelector(".btn-default").addEventListener("click", function () {
        close();
      });
      var unwrap = el.querySelector(".wrap-attributes-modal__unwrap");
      if (unwrap) {
        unwrap.addEventListener("click", function () {
          model.onRemove();
          close();
        });
      }
    },
  });
  return close;
}
