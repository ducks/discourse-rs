// The admin email pages in the browser (src/admin_email_view.rs draws
// them): the server settings' test email, the summary preview (the date
// picker, the user, Refresh, the html and text toggle, sending it), the
// advanced test's run, and the templates' filter and overridden toggle.
(function () {
  "use strict";

  if (!Discourse.once("admin-email")) {
    return;
  }

  var ui = Discourse.adminUi;
  var t = ui.t;
  var escapeHtml = ui.escapeHtml;

  function page(tab) {
    return document.querySelector(".admin-contents.admin-email" + (tab ? "." + tab : ""));
  }

  // DConditionalLoadingSpinner: the spinner while loading, else the
  // content as it stands.
  function loading(container, on) {
    if (on) {
      container.dataset.content = container.innerHTML;
      container.innerHTML = '<div class="spinner"></div>';
    } else if (container.dataset.content !== undefined) {
      container.innerHTML = container.dataset.content;
      delete container.dataset.content;
    }
  }

  // --- Server settings: the test email.

  function sendTestEmail(form) {
    var inline = form.querySelector(".inline-form");
    var input = inline.querySelector("input");
    var address = input.value;
    var kept = Array.prototype.slice.call(inline.childNodes);
    inline.textContent = t("admin.email.sending_test");
    function restore(message) {
      inline.textContent = "";
      kept.forEach(function (node) {
        if (!(node.classList && node.classList.contains("result-message"))) {
          inline.appendChild(node);
        }
      });
      if (message) {
        var span = document.createElement("span");
        span.className = "result-message";
        span.textContent = message;
        inline.appendChild(span);
      }
    }
    ui.ajax("POST", "/admin/email/test", { email_address: address })
      .then(function (result) {
        restore(result && result.sent_test_email_message);
      })
      .catch(function (error) {
        restore(null);
        var errors = error && error.json && error.json.errors;
        Discourse.dialog.alert({
          message: errors
            ? t("admin.email.error", { server_error: escapeHtml(errors[0]) })
            : t("admin.email.test_error"),
        });
      });
  }

  document.addEventListener(
    "submit",
    function (event) {
      var form = event.target.closest(".admin-email.server-settings form");
      if (!form) {
        return;
      }
      event.preventDefault();
      event.stopImmediatePropagation();
      var button = form.querySelector("button[type=submit]");
      if (button && !button.disabled) {
        sendTestEmail(form);
      }
    },
    true
  );

  // --- Summary preview.

  var preview = { showHtml: true, html: null, text: null };

  function previewState(root) {
    if (preview.html === null) {
      var iframe = root.querySelector(".preview-output iframe");
      var text = root.querySelector("template.email-preview-text");
      preview.html = iframe ? iframe.getAttribute("srcdoc") : "";
      preview.text = text ? text.content.textContent : "";
    }
    return preview;
  }

  function drawPreview(root) {
    var p = previewState(root);
    var toggle = root.querySelector(".email-preview .toggle");
    var html = escapeHtml(t("admin.email.html"));
    var text = escapeHtml(t("admin.email.text"));
    toggle.innerHTML =
      "<label>" +
      escapeHtml(t_format()) +
      "</label>" +
      (p.showHtml
        ? "<span>" + html + '</span> | <a class="show-text-link" href>' + text + "</a>"
        : '<a class="show-html-link" href>' + html + "</a> | <span>" + text + "</span>");
    var digest = root.querySelector(".email-preview-digest");
    var send = digest.querySelector(":scope > .controls");
    if (p.html && !send) {
      digest.insertAdjacentHTML("afterbegin", sendFormHtml(root));
    } else if (!p.html && send) {
      send.remove();
    }
    var output = digest.querySelector(".preview-output");
    if (!p.showHtml) {
      output.innerHTML = "<pre>" + escapeHtml(p.text) + "</pre>";
    } else if (!p.html) {
      output.innerHTML = "<p>" + escapeHtml(t("admin.email.no_result")) + "</p>";
    } else {
      output.innerHTML = '<iframe title="' + escapeHtml(t("admin.email.html_preview")) + '"></iframe>';
      output.querySelector("iframe").setAttribute("srcdoc", p.html);
    }
  }

  var formatLabel = null;
  function t_format() {
    return formatLabel;
  }

  var sendForm = null;
  function sendFormHtml(root) {
    if (sendForm === null) {
      var existing = root.querySelector(".email-preview-digest > .controls");
      sendForm = existing ? existing.outerHTML : "";
    }
    return sendForm;
  }

  function lastSeen(root) {
    var input = root.querySelector(".email-preview .date-picker");
    return input.dataset.value || "";
  }

  function username(root) {
    return root.querySelector(".email-preview .email-group-user-chooser").dataset.value || "";
  }

  function refresh(root) {
    var container = root.querySelector(".admin-container > .loading-container");
    var p = previewState(root);
    // No one chosen: the viewer, as the controller sets.
    var chooser = root.querySelector(".email-preview .email-group-user-chooser");
    if (!chooser.dataset.value) {
      chooser.dataset.value = chooser.dataset.current;
      chooser.classList.add("has-selection");
      chooser.querySelector(".formatted-selection").textContent = chooser.dataset.current;
      chooser.querySelector("summary").dataset.name = chooser.dataset.current;
    }
    loading(container, true);
    ui.ajax(
      "GET",
      "/admin/email/preview-digest.json?last_seen_at=" +
        encodeURIComponent(lastSeen(root)) +
        "&username=" +
        encodeURIComponent(username(root))
    )
      .then(function (result) {
        p.html = (result && result.html_content) || "";
        p.text = (result && result.text_content) || "";
      })
      .catch(function () {})
      .then(function () {
        loading(container, false);
        var sent = container.querySelector(".result-message");
        if (sent) {
          sent.remove();
        }
        drawPreview(root);
      });
  }

  function sendDigest(root, button) {
    var inline = button.closest(".inline-form");
    var input = inline.querySelector("input");
    var kept = Array.prototype.slice.call(inline.childNodes).filter(function (node) {
      return !(node.classList && node.classList.contains("result-message"));
    });
    inline.textContent = t("admin.email.sending_test");
    function restore(sent) {
      inline.textContent = "";
      kept.forEach(function (node) {
        inline.appendChild(node);
      });
      if (sent) {
        var span = document.createElement("span");
        span.className = "result-message";
        span.textContent = t("admin.email.sent_test");
        inline.appendChild(span);
      }
    }
    ui.ajax("POST", "/admin/email/send-digest.json", {
      last_seen_at: lastSeen(root),
      username: username(root),
      email: input.value,
    })
      .then(function (result) {
        if (result && result.errors) {
          restore(false);
          Discourse.dialog.alert(String(result.errors));
        } else {
          restore(true);
        }
      })
      .catch(function (error) {
        restore(false);
        ui.failed(error);
      });
  }

  // DatePickerPast: Pikaday on the desktop, as DDatePicker loads it.
  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = src;
      s.onload = resolve;
      s.onerror = reject;
      document.head.appendChild(s);
    });
  }

  function setUpDatePicker(root) {
    var input = root.querySelector(".email-preview .date-picker");
    if (!input) {
      return;
    }
    if (input.type === "date") {
      input.value = input.dataset.value;
      input.addEventListener("change", function () {
        input.dataset.value = input.value;
      });
      return;
    }
    // local-dates.js (deferred) loads moment.
    var domReady = new Promise(function (resolve) {
      if (document.readyState === "loading") {
        document.addEventListener("DOMContentLoaded", resolve);
      } else {
        resolve();
      }
    });
    domReady
      .then(function () {
        return Discourse.localDates.loadMoment();
      })
      .then(function () {
        return window.Pikaday ? null : loadScript(input.dataset.pikaday);
      })
      .then(function () {
        var m = window.moment;
        var picker = new window.Pikaday({
          field: input,
          container: null,
          bound: true,
          format: "YYYY-MM-DD",
          firstDay: 1,
          maxDate: new Date(),
          defaultDate: m(input.dataset.value, "YYYY-MM-DD").toDate(),
          setDefaultDate: true,
          i18n: {
            previousMonth: t("dates.previous_month"),
            nextMonth: t("dates.next_month"),
            months: m.months(),
            weekdays: m.weekdays(),
            weekdaysShort: m.weekdaysMin(),
          },
          onSelect: function (date) {
            input.dataset.value = m(date).format("YYYY-MM-DD");
          },
        });
        picker.setDate(m(input.dataset.value).toDate(), true);
      })
      .catch(function () {});
  }

  // --- Advanced test.

  function runAdvancedTest(root) {
    var container = root.querySelector(".admin-container > .loading-container");
    var email = root.querySelector(".email-advanced-test .email-body").value;
    container.innerHTML = '<div class="spinner"></div>';
    ui.ajax("POST", "/admin/email/advanced-test", { email: email })
      .then(function (data) {
        container.innerHTML = data && data.format
          ? "<hr><div class=\"text\"><h3>" +
            escapeHtml(t("admin.email.advanced_test.text")) +
            '</h3><pre class="full-reason">' +
            (data.text || "") +
            '</pre></div><hr><div class="elided"><h3>' +
            escapeHtml(t("admin.email.advanced_test.elided")) +
            '</h3><pre class="full-reason">' +
            (data.elided || "") +
            "</pre></div>"
          : "";
      })
      .catch(function (error) {
        container.innerHTML = "";
        ui.failed(error);
      });
  }

  // --- Templates: DFilterControls' text filter (title and id, comma
  // separated terms) and the overridden toggle, both kept in the URL.

  function filterTemplates(root) {
    var input = root.querySelector(".d-filter-controls__input");
    var onlyOverridden = root.querySelector("#toggle-overridden").checked;
    var terms = input.value
      .split(",")
      .map(function (term) {
        return term.trim().toLowerCase();
      })
      .filter(Boolean);
    var shown = 0;
    root.querySelectorAll(".email-templates-list__row").forEach(function (row) {
      var title = row.querySelector(".admin-email-templates__name").textContent.trim().toLowerCase();
      var id = row.dataset.templateId.toLowerCase();
      var match =
        (!onlyOverridden || row.classList.contains("overridden")) &&
        (terms.length === 0 ||
          terms.some(function (term) {
            return title.indexOf(term) !== -1 || id.indexOf(term) !== -1;
          }));
      row.hidden = !match;
      if (match) {
        shown += 1;
      }
    });
    var table = root.querySelector(".email-templates-list");
    if (table) {
      table.hidden = shown === 0;
    }
    var reset = root.querySelector(".d-filter-controls .d-filter-controls__reset");
    reset.disabled = input.value.length === 0;
    var none = root.querySelector(".d-filter-controls__no-results");
    if (shown === 0 && input.value.length > 0) {
      if (!none) {
        root.querySelector(".checkbox-label").insertAdjacentHTML(
          "afterend",
          '<div class="d-filter-controls__no-results"><p>' +
            escapeHtml(t("admin.customize.email_templates.no_templates_found")) +
            "</p>" +
            reset.outerHTML
              .replace(" disabled=\"\"", "")
              .replace(/<span aria-hidden="true">[^<]*<\/span>/, "")
              .replace(" no-text", "")
              .replace(" btn-icon ", " btn-icon-text ")
              .replace("</svg>", '</svg><span class="d-button-label">' + escapeHtml(t("filter_controls.reset")) + "</span>") +
            "</div>"
        );
      }
    } else if (none) {
      none.remove();
    }
    var url = new URL(window.location.href);
    if (input.value) {
      url.searchParams.set("filter", input.value);
    } else {
      url.searchParams.delete("filter");
    }
    if (onlyOverridden) {
      url.searchParams.set("overridden", "true");
    } else {
      url.searchParams.delete("overridden");
    }
    history.replaceState(history.state, "", url.pathname + url.search + url.hash);
  }

  var filterTimer = null;
  document.addEventListener("input", function (event) {
    var root = page("admin-email-templates");
    if (root && event.target.closest(".d-filter-controls__input")) {
      clearTimeout(filterTimer);
      filterTimer = setTimeout(function () {
        filterTemplates(root);
      }, 250);
    }
  });

  document.addEventListener("input", function (event) {
    var field = event.target.closest(".admin-email.server-settings .inline-form input, .admin-email.preview-digest .email-preview-digest .inline-form input");
    if (!field) {
      return;
    }
    var inline = field.closest(".inline-form");
    var button = inline.querySelector("button");
    button.disabled = field.value.trim() === "";
    var sent = inline.querySelector(".result-message");
    if (sent && inline.closest(".server-settings")) {
      sent.remove();
    }
  });

  document.addEventListener("click", function (event) {
    var root = page();
    if (!root) {
      return;
    }
    if (root.classList.contains("preview-digest")) {
      if (event.target.closest(".show-text-link, .show-html-link")) {
        event.preventDefault();
        preview.showHtml = !preview.showHtml;
        drawPreview(root);
      } else if (event.target.closest(".digest-refresh-button")) {
        refresh(root);
      } else if (event.target.closest(".email-preview-digest .inline-form button")) {
        sendDigest(root, event.target.closest("button"));
      }
    } else if (root.classList.contains("advanced-test")) {
      if (event.target.closest(".email-advanced-test button")) {
        runAdvancedTest(root);
      }
    } else if (root.classList.contains("admin-email-templates")) {
      if (event.target.closest("#toggle-overridden")) {
        filterTemplates(root);
      } else if (event.target.closest(".d-filter-controls__reset")) {
        root.querySelector(".d-filter-controls__input").value = "";
        filterTemplates(root);
      }
    }
  });

  Discourse.onPage(function () {
    if (!page()) {
      return;
    }
    ui.strings("admin-email-strings");
    preview = { showHtml: true, html: null, text: null };
    sendForm = null;
    var root = page("preview-digest");
    if (root) {
      var label = root.querySelector(".email-preview .toggle label");
      formatLabel = label ? label.textContent : "";
      previewState(root);
      sendFormHtml(root);
      setUpDatePicker(root);
    }
  });
})();
