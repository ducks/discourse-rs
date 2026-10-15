// The preferences tabs (controllers/preferences/*.js) on the
// server-rendered form: its ComboBoxes, Save Changes (User#save: the
// tab's saveAttrNames as the model holds them, put to /u/:username.json
// form-encoded as jQuery encodes them), and each tab's own actions:
// UsernamePreference, the email list's menu, deleting one's account, the
// bio's toolbar and preview, the timezone list, the image uploaders,
// PushNotificationSelect, UserNotificationSchedule and the text size.
// What another item ports is refused naming it (data-unported).
(function () {
  "use strict";

  if (!Discourse.once("preferences")) {
    return;
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function escape(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML;
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function csrf() {
    return (document.querySelector('meta[name="csrf-token"]') || {}).content || "";
  }

  function form() {
    return document.querySelector(".user-preferences form.form-vertical");
  }

  function labels() {
    var f = form();
    try {
      return JSON.parse((f && f.dataset.dialogLabels) || "{}");
    } catch (e) {
      return {};
    }
  }

  // jQuery.param: undefined is no param, null an empty one, a list each
  // item under key[], an object each key under key[k].
  function encode(prefix, value, out) {
    if (value === undefined) {
      return;
    }
    if (Array.isArray(value)) {
      value.forEach(function (item) {
        encode(prefix + "[]", item, out);
      });
    } else if (value !== null && typeof value === "object") {
      Object.keys(value).forEach(function (k) {
        encode(prefix + "[" + k + "]", value[k], out);
      });
    } else {
      out.push(encodeURIComponent(prefix) + "=" + encodeURIComponent(value === null ? "" : value));
    }
  }

  function param(data) {
    var out = [];
    Object.keys(data).forEach(function (k) {
      encode(k, data[k], out);
    });
    return out.join("&").replace(/%20/g, "+");
  }

  function ajax(method, url, data) {
    return fetch(base() + url, {
      method: method,
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json, text/javascript, */*; q=0.01",
      },
      body: data ? param(data) : undefined,
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (e) {
          json = null;
        }
        if (!response.ok) {
          var error = new Error(
            (json && json.errors && json.errors.join("\n")) ||
              (json && json.error) ||
              labels().generic_error ||
              "Sorry, an error has occurred."
          );
          error.json = json;
          throw error;
        }
        return json;
      });
    });
  }

  function popupAjaxError(e) {
    Discourse.dialog.popupAjaxError(e.message);
  }

  function unported(what) {
    Discourse.dialog.alert(escape("Not ported yet: " + what + "."));
  }

  // ComboBox and DropdownSelectBox: the body floats under the header as
  // select-kit's does.
  function floatBody(chooser) {
    var header = chooser.querySelector("summary");
    var body = chooser.querySelector(".select-kit-body");
    if (!body || !window.FloatingUIDOM) {
      return;
    }
    var ui = window.FloatingUIDOM;
    var small = window.innerWidth <= 450;
    body.style.position = "fixed";
    body.style.top = "0";
    body.style.left = "0";
    body.style.minWidth = small ? "" : Math.max(Math.round(header.getBoundingClientRect().width), 220) + "px";
    body.style.width = small ? window.innerWidth - 20 + "px" : "";
    ui.computePosition(header, body, {
      placement: "bottom-start",
      strategy: "fixed",
      middleware: [ui.flip({ padding: 3 }), ui.shift({ limiter: ui.limitShift() }), ui.offset(3)],
    }).then(function (result) {
      var x = small ? 10 : Math.round(result.x);
      body.style.transform = "translate(" + x + "px," + Math.round(result.y) + "px)";
    });
  }

  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(".user-preferences details.prefs-combo")) {
        return;
      }
      var open = chooser.open;
      chooser.classList.toggle("is-expanded", open);
      var caret = chooser.querySelector(".angle-icon");
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : "angle-down", "angle-icon");
      }
      if (!open) {
        return;
      }
      if (chooser.classList.contains("timezone-input")) {
        loadTimezones(chooser);
      }
      floatBody(chooser);
      var filter = chooser.querySelector(".filter-input");
      if (filter) {
        filter.value = "";
        filterRows(chooser, "");
        filter.focus();
      }
    },
    true
  );

  function filterRows(chooser, text) {
    var needle = text.toLowerCase();
    chooser.querySelectorAll(".select-kit-row").forEach(function (row) {
      row.hidden = needle !== "" && (row.dataset.name || "").toLowerCase().indexOf(needle) === -1;
    });
  }

  document.addEventListener("input", function (event) {
    var filter = event.target.closest(".user-preferences details.prefs-combo .filter-input");
    if (filter) {
      filterRows(filter.closest("details"), filter.value);
    }
  });

  function rowHtml(index, value, name, selected) {
    var n = escape(name);
    return (
      '<li aria-checked="' +
      selected +
      '" role="menuitemradio" data-index="' +
      index +
      '" data-name="' +
      n +
      '" data-value="' +
      escape(value) +
      '" title="' +
      n +
      '" tabindex="0" class="select-kit-row' +
      (selected ? " is-highlighted is-selected" : "") +
      '"><span class="name">' +
      n +
      "</span></li>"
    );
  }

  // Selecting a row: the header shows it, the rows mark it.
  function select(chooser, row) {
    var value = row.classList.contains("none") ? null : row.dataset.value;
    var name = row.dataset.name;
    var summary = chooser.querySelector("summary");
    var choice = chooser.querySelector(".select-kit-selected-name");
    [summary, choice].forEach(function (el) {
      el.dataset.name = name;
      if (value === null) {
        delete el.dataset.value;
      } else {
        el.dataset.value = value;
      }
    });
    choice.title = name;
    choice.querySelector(".name").textContent = name;
    chooser.classList.toggle("has-selection", value !== null);
    chooser.querySelectorAll(".select-kit-row").forEach(function (r) {
      var is = r === row;
      r.classList.toggle("is-selected", is && value !== null);
      r.classList.toggle("is-highlighted", is);
      r.setAttribute("aria-checked", is ? "true" : "false");
    });
    chooser.dataset.dirty = "true";
    chooser.dispatchEvent(new CustomEvent("prefs-change", { bubbles: true, detail: { value: value } }));
  }

  function setValue(chooser, value) {
    var row = Array.prototype.find.call(chooser.querySelectorAll(".select-kit-row"), function (r) {
      return r.dataset.value === String(value);
    });
    if (row) {
      select(chooser, row);
    }
  }

  function comboValue(chooser) {
    var summary = chooser.querySelector("summary");
    return summary.dataset.value === undefined ? null : summary.dataset.value;
  }

  document.addEventListener("click", function (event) {
    var row = event.target.closest(".user-preferences details.prefs-combo .select-kit-row");
    document.querySelectorAll(".user-preferences details.prefs-combo[open]").forEach(function (chooser) {
      if (!chooser.contains(event.target) || row) {
        chooser.open = false;
      }
    });
    if (row) {
      var chooser = row.closest("details");
      if (chooser.classList.contains("email-dropdown")) {
        emailAction(chooser, row.dataset.value);
      } else {
        select(chooser, row);
      }
      return;
    }
    var unportedEl = event.target.closest(".user-preferences [data-unported]:not(.push-notification-config)");
    if (unportedEl) {
      event.preventDefault();
      unported(unportedEl.dataset.unported);
      return;
    }
    if (event.target.closest(".user-preferences .save-changes")) {
      save();
      return;
    }
    if (event.target.closest(".username-preference__edit-username")) {
      editUsername();
      return;
    }
    if (event.target.closest(".delete-account-button")) {
      deleteAccount(event.target.closest(".delete-account-button"));
      return;
    }
    var resend = event.target.closest(".pref-email .resend-email-confirmation");
    if (resend) {
      resendConfirmation(resend);
      return;
    }
    if (event.target.closest(".pref-email .check-email")) {
      checkEmail(event.target.closest(".pref-email"));
      return;
    }
    if (event.target.closest(".use-current-timezone")) {
      useCurrentTimezone();
      return;
    }
    if (event.target.closest(".pref-reset-seen-user-tips")) {
      ajax("PUT", form().dataset.saveUrl.replace(base(), ""), {
        skip_new_user_tips: false,
        seen_popups: null,
      }).catch(popupAjaxError);
      return;
    }
    var tool = event.target.closest(".pref-bio .toolbar__button");
    if (tool) {
      bioAction(tool.dataset.action);
      return;
    }
    var del = event.target.closest(".file-uploader .file-uploader__delete");
    if (del) {
      setUpload(del.closest(".file-uploader"), null);
      return;
    }
    var expand = event.target.closest(".file-uploader .image-uploader-lightbox-btn");
    if (expand) {
      var link = expand.closest(".file-uploader").querySelector("a.lightbox");
      if (link) {
        window.open(link.href, "_blank", "noopener");
      }
    }
  });

  // Save Changes: the tab's attributes, the controls' values over them.
  function setPath(data, key, value) {
    var parts = key.split(".");
    if (parts.length === 1) {
      data[key] = value;
      return;
    }
    if (!data[parts[0]] || typeof data[parts[0]] !== "object") {
      data[parts[0]] = {};
    }
    data[parts[0]][parts[1]] = value;
  }

  function controlValue(el) {
    // A multi-select (prefs-choosers.js): User#save's categories as ids,
    // an empty list as [-1]; users and tags comma-joined.
    if (el.dataset.chooser) {
      var items = JSON.parse(el.dataset.items || "[]");
      if (el.dataset.chooser === "category") {
        return items.length
          ? items.map(function (i) {
              return parseInt(i[0], 10);
            })
          : [-1];
      }
      return items
        .map(function (i) {
          return i[1];
        })
        .join(",");
    }
    if (el.tagName === "DETAILS") {
      return comboValue(el);
    }
    if (el.type === "checkbox") {
      return el.checked;
    }
    if (el.classList.contains("file-uploader")) {
      return el.dataset.url === "" ? null : el.dataset.url;
    }
    return el.value;
  }

  // The model holds what the controls started with; the ones the user
  // changed set theirs.
  function collect(f) {
    var data = JSON.parse(f.dataset.saveAttrs || "{}");
    f.querySelectorAll("[data-pref]").forEach(function (el) {
      if (el.dataset.dirty === "true") {
        setPath(data, el.dataset.pref, controlValue(el));
      }
    });
    // _updateUserFields: when the model has fields, each one shown,
    // empty as null.
    if (data.user_fields && typeof data.user_fields === "object") {
      f.querySelectorAll("[data-user-field]").forEach(function (el) {
        var v = controlValue(el);
        data.user_fields[el.dataset.userField] = v === "" || v === null ? null : v;
      });
    }
    // The interface's text size: the default on every device, or this
    // browser's cookie.
    var size = f.querySelector("#text-size-selector");
    if (size) {
      var makeDefault = f.querySelector(".text-size-default input");
      if (makeDefault && makeDefault.checked) {
        data.text_size = comboValue(size);
      }
    }
    return data;
  }

  function saved(f, button, ok) {
    button.disabled = false;
    button.querySelector(".d-button-label").textContent = f.dataset.saveLabel;
    var area = button.closest(".save-button");
    var mark = area.querySelector(".saved");
    if (ok && !mark) {
      mark = document.createElement("span");
      mark.className = "saved";
      mark.textContent = f.dataset.savedLabel;
      area.appendChild(mark);
    }
  }

  function save() {
    var f = form();
    if (!f) {
      return;
    }
    var button = f.querySelector(".save-changes");
    var mark = f.querySelector(".save-button .saved");
    if (mark) {
      mark.remove();
    }
    var data = collect(f);
    button.disabled = true;
    button.querySelector(".d-button-label").textContent = f.dataset.savingLabel;
    ajax("PUT", f.dataset.saveUrl.replace(base(), ""), data)
      .then(function (result) {
        f.dataset.saveAttrs = JSON.stringify(data);
        f.querySelectorAll("[data-dirty]").forEach(function (el) {
          delete el.dataset.dirty;
        });
        afterSave(f, result);
        saved(f, button, true);
      })
      .catch(function (e) {
        saved(f, button, false);
        popupAjaxError(e);
      });
  }

  function afterSave(f, result) {
    var size = f.querySelector("#text-size-selector");
    if (size && f.querySelector(".text-size-default")) {
      var makeDefault = f.querySelector(".text-size-default input").checked;
      if (makeDefault) {
        document.cookie = "text_size=; path=/; expires=Thu, 01 Jan 1970 00:00:00 GMT";
      } else {
        var seq = result && result.user && result.user.user_option ? result.user.user_option.text_size_seq : 0;
        document.cookie =
          "text_size=" + encodeURIComponent(comboValue(size) + "|" + seq) + "; path=/; max-age=" + 365 * 86400;
      }
    }
    if (result && result.user && result.user.bio_cooked !== undefined) {
      var preview = f.querySelector(".pref-bio .d-editor-preview");
      if (preview && result.user.bio_cooked !== null) {
        preview.innerHTML = result.user.bio_cooked;
      }
    }
  }

  // Changes the user makes mark a control as set.
  document.addEventListener("change", function (event) {
    var el = event.target.closest(".user-preferences [data-pref]");
    if (el) {
      el.dataset.dirty = "true";
    }
    var schedule = event.target.closest('.user-preferences [data-pref="user_notification_schedule.enabled"]');
    if (schedule) {
      var body = document.querySelector(".notification-schedule-body");
      if (body) {
        body.hidden = !schedule.checked;
      }
    }
  });
  document.addEventListener("input", function (event) {
    var el = event.target.closest(".user-preferences [data-pref]");
    if (el) {
      el.dataset.dirty = "true";
    }
    if (event.target.closest(".pref-bio .d-editor-input")) {
      bioPreview();
    }
    if (event.target.closest(".username-preference__input")) {
      usernameInput(event.target);
    }
  });

  document.addEventListener("prefs-change", function (event) {
    var chooser = event.target;
    var value = event.detail.value;
    // UserNotificationScheduleDay: a new start moves the end after it.
    var startKey = (chooser.dataset.pref || "").match(/^user_notification_schedule\.day_(\d)_start_time$/);
    if (startKey) {
      scheduleStartChanged(chooser, parseInt(startKey[1], 10), parseInt(value, 10));
    }
    if (chooser.id === "text-size-selector") {
      textSizeChanged(value);
    }
    if (chooser.classList.contains("push-notification-select")) {
      pushLevelChanged(chooser, value);
    }
    if (chooser.classList.contains("chat-sound")) {
      var controls = chooser.closest(".chat-sound-controls");
      var preview = controls.querySelector(".chat-sound-preview");
      if (value && !preview) {
        controls.insertAdjacentHTML(
          "beforeend",
          '<button class="btn no-text btn-icon btn-flat chat-sound-preview" data-unported="chat\'s notification sounds (162)" type="button">' +
            icon("play") +
            '<span aria-hidden="true">&#8203;</span></button>'
        );
      } else if (!value && preview) {
        preview.remove();
      }
    }
  });

  // UserNotificationSchedule#buildTimeOptions, moment's LT in English.
  function timeLabel(minutes) {
    var hours = Math.floor(minutes / 60);
    var twelve = hours % 12 === 0 ? 12 : hours % 12;
    var mm = minutes % 60;
    return twelve + ":" + (mm < 10 ? "0" + mm : mm) + " " + (hours < 12 ? "AM" : "PM");
  }

  function scheduleStartChanged(chooser, day, start) {
    var row = chooser.closest("tr");
    var endCell = row.querySelector(".ends-at");
    var toCell = row.querySelector(".to");
    if (start === -1) {
      if (endCell) {
        endCell.remove();
      }
      if (toCell) {
        toCell.remove();
      }
      return;
    }
    var key = "user_notification_schedule.day_" + day + "_end_time";
    var current = null;
    if (endCell) {
      current = comboValue(endCell.querySelector("details"));
    }
    var end = current === null ? null : parseInt(current, 10);
    if (end === null || end <= start) {
      end = start + 30;
    }
    var rows = "";
    var index = 0;
    for (var m = start + 30; m <= 1440; m += 30) {
      var name = m === 1440 ? row.closest(".notification-schedule").dataset.midnight || "Midnight" : timeLabel(m);
      rows += rowHtml(index++, String(m), name, m === end);
    }
    var selectedName = end === 1440 ? row.closest(".notification-schedule").dataset.midnight || "Midnight" : timeLabel(end);
    var html =
      '<details class="select-kit single-select combobox combo-box has-selection prefs-combo" data-pref="' +
      key +
      '" data-dirty="true"><summary data-name="' +
      escape(selectedName) +
      '" data-value="' +
      end +
      '" tabindex="0" class="select-kit-header single-select-header combo-box-header"><div class="select-kit-header-wrapper"><div class="select-kit-selected-name selected-name choice" data-name="' +
      escape(selectedName) +
      '" data-value="' +
      end +
      '" title="' +
      escape(selectedName) +
      '"><span class="name">' +
      escape(selectedName) +
      "</span></div>" +
      icon("angle-down", "angle-icon") +
      '</div></summary><div class="select-kit-body"><ul aria-live="polite" class="select-kit-collection" role="menu">' +
      rows +
      "</ul></div></details>";
    if (endCell) {
      endCell.innerHTML = html;
    } else {
      var to = row.closest(".notification-schedule").dataset.to || "to";
      row.insertAdjacentHTML("beforeend", '<td class="to">' + escape(to) + '</td><td class="ends-at">' + html + "</td>");
    }
  }

  // selectTextSize: one's own page shows the size at once; the default
  // checkbox shows while it differs from the saved one.
  function textSizeChanged(size) {
    var f = form();
    var own = f.querySelector(".text-size-default");
    var classes = document.documentElement.classList;
    ["smallest", "smaller", "normal", "larger", "largest"].forEach(function (name) {
      classes.toggle("text-size-" + name, name === size);
    });
    if (own) {
      var attrs = JSON.parse(f.dataset.saveAttrs || "{}");
      own.hidden = attrs.text_size === size;
    }
  }

  // PushNotificationSelect: DesktopNotifications' support and permission.
  function pushSetup() {
    var config = document.querySelector(".push-notification-config");
    if (!config) {
      return;
    }
    var l = JSON.parse(config.dataset.labels || "{}");
    var supported = "Notification" in window && "serviceWorker" in navigator;
    if (!supported) {
      config.innerHTML =
        '<button class="btn btn-icon-text btn-default" disabled type="button">' +
        icon("bell-slash") +
        '<span class="d-button-label">' +
        escape(l.not_supported) +
        "</span></button>";
    } else if (Notification.permission === "denied") {
      config.innerHTML =
        '<button class="btn btn-icon-text btn-default" disabled type="button">' +
        icon("bell-slash") +
        '<span class="d-button-label">' +
        escape(l.perm_denied_btn) +
        "</span></button> <span>" +
        escape(l.perm_denied_expl) +
        "</span>";
    }
    // The control group's children as the component leaves them.
    while (config.firstChild) {
      config.parentNode.insertBefore(config.firstChild, config);
    }
    config.remove();
  }

  function pushLevelChanged(chooser, value) {
    if (value === "none") {
      ajax("PUT", form().dataset.saveUrl.replace(base(), ""), { push_notification_level: "none" }).catch(
        popupAjaxError
      );
      return;
    }
    setValue(chooser, "none");
    unported("subscribing to push notifications (186)");
  }

  // UsernamePreference
  var usernameTimer = null;

  function usernameRoot() {
    return document.querySelector(".username-preference");
  }

  function usernameLabels() {
    return JSON.parse(usernameRoot().dataset.labels || "{}");
  }

  function editUsername() {
    var root = usernameRoot();
    var l = usernameLabels();
    root.dataset.original = root.innerHTML;
    root.innerHTML =
      '<div class="form-horizontal"><div class="control-group"><input class="ember-text-field input-xxlarge username-preference__input" maxlength="' +
      escape(root.dataset.maxLength) +
      '" type="text" value="' +
      escape(root.dataset.username) +
      '"><div class="instructions"><p><span></span></p></div></div><div class="control-group">' +
      '<button class="btn btn-text btn-primary username-preference__submit" disabled type="submit"><span class="d-button-label">' +
      escape(l.change) +
      '</span></button><a class="cancel" href>' +
      escape(l.cancel) +
      "</a></div></div>";
    root.querySelector(".cancel").addEventListener("click", function (event) {
      event.preventDefault();
      root.innerHTML = root.dataset.original;
    });
    root.querySelector(".username-preference__submit").addEventListener("click", changeUsername);
    root.querySelector("input").focus();
  }

  function usernameState(error, taken) {
    var root = usernameRoot();
    var l = usernameLabels();
    var input = root.querySelector("input");
    var p = root.querySelector(".instructions p");
    p.innerHTML = (taken ? escape(l.taken) : "") + "<span>" + escape(error || "") + "</span>";
    var value = input.value;
    root.querySelector(".username-preference__submit").disabled =
      !value || taken || !!error || value === root.dataset.username;
  }

  function usernameInput(input) {
    var root = usernameRoot();
    var l = usernameLabels();
    var value = input.value;
    clearTimeout(usernameTimer);
    usernameState(null, false);
    if (!value || value === root.dataset.username) {
      return;
    }
    if (value.length < parseInt(root.dataset.minLength, 10)) {
      usernameState(l.too_short, false);
      return;
    }
    usernameTimer = setTimeout(function () {
      var userId = form().dataset.userId;
      var url =
        "/u/check_username.json?username=" +
        encodeURIComponent(value) +
        (userId ? "&for_user_id=" + encodeURIComponent(userId) : "");
      fetch(base() + url, {
        credentials: "same-origin",
        headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
      })
        .then(function (r) {
          return r.json();
        })
        .then(function (result) {
          if (input.value !== value) {
            return;
          }
          if (result.errors) {
            usernameState(result.errors.join(" "), false);
          } else if (result.available === false) {
            usernameState(null, true);
          }
        })
        .catch(function () {});
    }, 500);
  }

  function changeUsername(event) {
    event.preventDefault();
    var root = usernameRoot();
    var l = usernameLabels();
    var value = root.querySelector("input").value;
    Discourse.dialog.yesNoConfirm({
      title: l.confirm,
      didConfirm: function () {
        var button = root.querySelector(".username-preference__submit");
        button.disabled = true;
        button.querySelector(".d-button-label").textContent = l.saving;
        ajax("PUT", "/u/" + encodeURIComponent(root.dataset.username.toLowerCase()) + "/preferences/username", {
          new_username: value,
        })
          .then(function () {
            window.location.href = base() + "/u/" + encodeURIComponent(value.toLowerCase()) + "/preferences";
          })
          .catch(function (e) {
            button.querySelector(".d-button-label").textContent = l.change;
            button.disabled = false;
            popupAjaxError(e);
          });
      },
    });
  }

  // EmailDropdown
  function emailAction(chooser, action) {
    if (action === "updateEmail") {
      window.location.href = chooser.dataset.emailHref;
    } else {
      var email = chooser.closest(".row.email").querySelector(".email-first").textContent;
      var username = form().dataset.username;
      var request =
        action === "setPrimaryEmail"
          ? ajax("PUT", "/u/" + encodeURIComponent(username) + "/preferences/primary-email.json", { email: email })
          : ajax("DELETE", "/u/" + encodeURIComponent(username) + "/preferences/email.json", { email: email });
      request
        .then(redrawEmails)
        .catch(action === "setPrimaryEmail" ? popupAjaxError : function () {});
    }
  }

  // The model's emails changed: the list drawn again from the page.
  function redrawEmails() {
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var fresh = new DOMParser().parseFromString(html, "text/html").querySelector(".pref-email .emails");
        var current = document.querySelector(".pref-email .emails");
        if (fresh && current) {
          current.replaceWith(document.adoptNode(fresh));
        }
      });
  }

  // resendConfirmationEmail: User#addEmail again.
  function resendConfirmation(button) {
    var group = button.closest(".pref-email");
    var l = {};
    try {
      l = JSON.parse(group.dataset.labels || "{}");
    } catch (e) {
      l = {};
    }
    var email = button.closest(".row.email").querySelector(".email-first").textContent;
    var label = document.createElement("span");
    label.textContent = l.resending;
    button.replaceWith(label);
    ajax("POST", "/u/" + encodeURIComponent(form().dataset.username) + "/preferences/email", { email: email })
      .then(function () {
        label.textContent = l.resent;
      })
      .catch(function () {
        label.replaceWith(button);
      });
  }

  // admin.users.check_email: the addresses, as the list shows them.
  function checkEmail(group) {
    var username = form().dataset.username;
    ajax("GET", "/u/" + encodeURIComponent(username) + "/emails.json?context=" + encodeURIComponent(location.pathname))
      .then(function (result) {
        var controls = group.querySelector(".controls");
        var html = '<div class="emails">';
        var emails = [{ email: result.email, primary: true, confirmed: true }]
          .concat(
            (result.secondary_emails || []).map(function (e) {
              return { email: e, confirmed: true };
            })
          )
          .concat(
            (result.unconfirmed_emails || []).map(function (e) {
              return { email: e };
            })
          )
          .sort(function (a, b) {
            return a.email.localeCompare(b.email);
          });
        var l = JSON.parse(group.dataset.labels || "{}");
        emails.forEach(function (e) {
          html +=
            '<div class="row email"><div class="email-first">' +
            escape(e.email) +
            '</div><div class="email-second">' +
            (e.primary ? '<span class="primary">' + escape(l.primary) + "</span>" : "") +
            (e.confirmed ? "" : '<span class="unconfirmed">' + escape(l.unconfirmed) + "</span>") +
            "</div></div>";
        });
        html += '</div><div class="instructions">' + escape(l.instructions) + "</div>";
        controls.outerHTML = html;
      })
      .catch(popupAjaxError);
  }

  // The account's Delete My Account.
  function deleteAccount(button) {
    var d = button.dataset;
    Discourse.dialog.alert({
      message: escape(d.confirm),
      buttons: [
        {
          icon: "triangle-exclamation",
          label: button.querySelector(".d-button-label").textContent,
          class: "btn-danger",
          action: function () {
            ajax("DELETE", "/u/" + encodeURIComponent(form().dataset.username) + ".json", {})
              .then(function () {
                Discourse.dialog.alert({
                  message: escape(d.deleted),
                  didConfirm: function () {
                    window.location.href = base() + "/";
                  },
                  didCancel: function () {
                    window.location.href = base() + "/";
                  },
                });
              })
              .catch(function () {
                Discourse.dialog.alert(escape(d.notAllowed));
              });
          },
        },
        { label: d.cancel },
      ],
    });
  }

  // The bio: the composer's markdown around the selection, and its
  // preview from the composer's renderer.
  function bioArea() {
    return document.querySelector(".pref-bio .d-editor-input");
  }

  function surround(before, after, example) {
    var textarea = bioArea();
    var start = textarea.selectionStart;
    var end = textarea.selectionEnd;
    var selected = textarea.value.slice(start, end) || example;
    textarea.setRangeText(before + selected + after, start, end, "end");
    textarea.selectionStart = start + before.length;
    textarea.selectionEnd = start + before.length + selected.length;
    textarea.focus();
  }

  function prefixLines(prefix, example) {
    var textarea = bioArea();
    var start = textarea.selectionStart;
    var end = textarea.selectionEnd;
    var lineStart = textarea.value.lastIndexOf("\n", start - 1) + 1;
    var selected = textarea.value.slice(lineStart, end) || example;
    var replaced = selected
      .split("\n")
      .map(function (line) {
        return prefix + line;
      })
      .join("\n");
    textarea.setRangeText(replaced, lineStart, Math.max(end, lineStart), "end");
    textarea.focus();
  }

  function bioAction(action) {
    var textarea = bioArea();
    var data = (document.getElementById("reply-control") || {}).dataset || {};
    var actions = {
      bold: function () {
        surround("**", "**", data.textBold || "strong text");
      },
      italic: function () {
        surround("*", "*", data.textItalic || "emphasized text");
      },
      link: function () {
        surround("[", "](https://)", data.textLink || "link");
      },
      blockquote: function () {
        prefixLines("> ", data.textBlockquote || "Blockquote");
      },
      code: function () {
        var selected = textarea.value.slice(textarea.selectionStart, textarea.selectionEnd);
        if (selected.indexOf("\n") >= 0 || selected === "") {
          surround("```\n", "\n```", data.textCode || "code");
        } else {
          surround("`", "`", "");
        }
      },
      list: function () {
        prefixLines("- ", data.textList || "List item");
      },
    };
    if (actions[action]) {
      actions[action]();
      textarea.dataset.dirty = "true";
      bioPreview();
    }
  }

  var previewTimer = null;

  function bioPreview() {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(function () {
      var textarea = bioArea();
      var preview = document.querySelector(".pref-bio .d-editor-preview");
      if (!textarea || !preview || !Discourse.composerCook) {
        return;
      }
      Discourse.composerCook(textarea.value)
        .then(function (html) {
          preview.innerHTML = html;
        })
        .catch(function () {});
    }, 200);
  }

  // TimezoneInput: moment.tz.names(), loaded the first time it opens.
  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var s = document.createElement("script");
      s.src = src;
      s.onload = resolve;
      s.onerror = reject;
      document.head.appendChild(s);
    });
  }

  function moment() {
    var group = document.querySelector(".pref-timezone");
    var ready = window.moment && window.moment.tz ? Promise.resolve() : Promise.resolve();
    if (!window.moment) {
      ready = loadScript(group.dataset.moment);
    }
    return ready.then(function () {
      if (!window.moment.tz) {
        return loadScript(group.dataset.momentTimezone);
      }
    });
  }

  function loadTimezones(chooser) {
    if (chooser.dataset.loaded) {
      return;
    }
    chooser.dataset.loaded = "true";
    moment().then(function () {
      var current = comboValue(chooser);
      var html = "";
      window.moment.tz.names().forEach(function (name, i) {
        html += rowHtml(i, name, name, name === current);
      });
      chooser.querySelector(".select-kit-collection").innerHTML = html;
      floatBody(chooser);
    });
  }

  function useCurrentTimezone() {
    var chooser = document.querySelector(".pref-timezone details");
    moment().then(function () {
      var zone = window.moment.tz.guess(true);
      if (!chooser.querySelector('.select-kit-row[data-value="' + zone + '"]')) {
        chooser
          .querySelector(".select-kit-collection")
          .insertAdjacentHTML("beforeend", rowHtml(0, zone, zone, false));
      }
      setValue(chooser, zone);
    });
  }

  // UppyImageUploader: the file to /uploads.json, its url to the model.
  function setUpload(uploader, upload) {
    uploader.dataset.url = upload ? upload.url : "";
    uploader.dataset.dirty = "true";
    var id = uploader.id;
    var l = JSON.parse(uploader.dataset.labels || form().dataset.uploadLabels || "{}");
    var picker =
      icon("upload") +
      '<div class="pick-files-button"><input accept="image/*" id="' +
      id +
      '__input" type="file"></div>';
    uploader.classList.toggle("has-image", !!upload);
    uploader.classList.toggle("no-image", !upload);
    if (upload) {
      uploader.innerHTML =
        '<div class="file-uploader__preview input-xxlarge" style="background-image: url(' +
        escape(upload.url) +
        ')"><a class="lightbox" href="' +
        escape(upload.url) +
        '" rel="nofollow ugc noopener" title="' +
        escape(upload.original_filename) +
        '"><div class="meta"><span class="informations">' +
        escape(upload.width) +
        "x" +
        escape(upload.height) +
        " " +
        escape(upload.human_filesize) +
        '</span></div></a><div class="expand-overlay"><button class="btn no-text btn-icon btn-default btn-small image-uploader-lightbox-btn" type="button">' +
        icon("discourse-expand") +
        '<span aria-hidden="true">&#8203;</span></button></div></div><div class="file-uploader__controls"><label class="btn btn-default btn-small" for="' +
        id +
        '__input" tabindex="0">' +
        picker +
        " " +
        escape(l.change || "Change") +
        '</label><button class="btn btn-icon-text btn-danger btn-small file-uploader__delete" type="button">' +
        icon("trash-can") +
        '<span class="d-button-label">' +
        escape(l.delete || "Delete") +
        "</span></button></div>";
    } else {
      uploader.innerHTML =
        '<div class="file-uploader__preview input-xxlarge" style="background-image: url()"><div class="file-uploader__controls"><label class="btn btn-transparent" for="' +
        id +
        '__input" tabindex="0">' +
        picker +
        " " +
        escape(l.select_file || "Click to upload or drag & drop file") +
        '</label><div class="file-uploader__progress-status hidden"><div class="progress-bar-container" role="progressbar"><div class="progress-bar" style="width: 0%"></div></div><span></span></div></div></div>';
    }
  }

  document.addEventListener("change", function (event) {
    var input = event.target.closest(".user-preferences .file-uploader input[type=file]");
    if (!input || !input.files.length) {
      return;
    }
    var uploader = input.closest(".file-uploader");
    var file = input.files[0];
    var body = new FormData();
    body.append("upload_type", uploader.dataset.uploadType);
    body.append("file", file, file.name);
    var status = uploader.querySelector(".file-uploader__progress-status");
    var bar = uploader.querySelector(".progress-bar");
    var xhr = new XMLHttpRequest();
    xhr.open("POST", base() + "/uploads.json?client_id=" + Date.now());
    xhr.setRequestHeader("X-CSRF-Token", csrf());
    xhr.setRequestHeader("X-Requested-With", "XMLHttpRequest");
    xhr.upload.addEventListener("progress", function (e) {
      if (e.lengthComputable && status && bar) {
        status.classList.remove("hidden");
        bar.style.width = Math.round((e.loaded / e.total) * 100) + "%";
      }
    });
    xhr.addEventListener("load", function () {
      var json = null;
      try {
        json = JSON.parse(xhr.responseText);
      } catch (e) {
        json = null;
      }
      if (xhr.status >= 200 && xhr.status < 300 && json && json.url) {
        setUpload(uploader, json);
      } else {
        if (status) {
          status.classList.add("hidden");
        }
        Discourse.dialog.popupAjaxError(
          (json && json.errors && json.errors.join(" ")) || labels().generic_error || "Sorry, an error has occurred."
        );
      }
    });
    xhr.send(body);
  });

  // EmailController: the address validated as it is typed, then changed
  // (users_email#update) or added (#create).
  var EMAIL_RE =
    /^[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+(?:\.[a-zA-Z0-9!#$%&'*+/=?^_`{|}~-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9-]*[a-zA-Z0-9])?$/;

  function emailPage() {
    return document.querySelector(".solo-preference[data-email-labels]");
  }

  function emailLabels(page) {
    try {
      return JSON.parse(page.dataset.emailLabels);
    } catch (e) {
      return {};
    }
  }

  function syncEmail(page) {
    var l = emailLabels(page);
    var input = page.querySelector("#change-email");
    var value = input.value;
    var invalid = !EMAIL_RE.test(value);
    var unchanged = value.toLowerCase().trim() === page.dataset.emailOld;
    var button = page.querySelector(".save-button .btn-primary");
    button.disabled = page.dataset.saving === "true" || value === "" || unchanged || invalid;
    var tip = page.querySelector(".tip");
    var failed = invalid && (page.dataset.emailOld !== "" || value !== "");
    tip.className = "tip " + (failed ? "bad" : "good");
    tip.innerHTML = failed ? l.x_icon + " " + escape(l.invalid) : "";
  }

  function saveEmail(page) {
    var l = emailLabels(page);
    var input = page.querySelector("#change-email");
    var button = page.querySelector(".save-button .btn-primary");
    page.dataset.saving = "true";
    button.disabled = true;
    button.querySelector(".d-button-label").textContent = l.saving;
    var username = form().dataset.username;
    ajax(
      page.dataset.emailNew === "true" ? "POST" : "PUT",
      "/u/" + encodeURIComponent(username) + "/preferences/email",
      { email: input.value }
    )
      .then(function () {
        page.innerHTML =
          '<div class="alert alert-success">' +
          escape(l.success) +
          '</div><a class="success-back" href="' +
          escape(base() + "/u/" + encodeURIComponent(username) + "/preferences/account") +
          '">' +
          l.back_icon +
          " " +
          escape(l.back) +
          "</a>";
      })
      .catch(function (e) {
        page.dataset.saving = "false";
        button.querySelector(".d-button-label").textContent = l.save;
        var alert = page.querySelector(".alert-error");
        if (!alert) {
          alert = document.createElement("div");
          alert.className = "alert alert-error";
          page.insertBefore(alert, page.querySelector(".control-group"));
        }
        alert.textContent = (e.json && e.json.errors && e.json.errors[0]) || l.error;
        syncEmail(page);
      });
  }

  document.addEventListener("input", function (event) {
    var page = emailPage();
    if (page && event.target.id === "change-email") {
      syncEmail(page);
    }
  });
  document.addEventListener("click", function (event) {
    var page = emailPage();
    var button = event.target.closest(".solo-preference .save-button .btn-primary");
    if (page && button) {
      event.preventDefault();
      if (!button.disabled) {
        saveEmail(page);
      }
    }
  });
  document.addEventListener("submit", function (event) {
    var page = emailPage();
    if (page && page.contains(event.target)) {
      event.preventDefault();
      var button = page.querySelector(".save-button .btn-primary");
      if (button && !button.disabled) {
        saveEmail(page);
      }
    }
  });

  // What the templates show while a control is checked or has a value
  // (data-show-pref, data-show-value), and disable while one is checked
  // (data-disabled-by).
  function syncShown() {
    var f = form();
    if (!f) {
      return;
    }
    var value = function (key) {
      var control = f.querySelector('[data-pref="' + key + '"]');
      return control ? controlValue(control) : null;
    };
    f.querySelectorAll("[data-show-pref]").forEach(function (el) {
      var v = value(el.dataset.showPref);
      var shown = el.dataset.showValue !== undefined ? String(v) === el.dataset.showValue : !!v;
      if (el.dataset.showAlso) {
        shown = shown && !!value(el.dataset.showAlso);
      }
      el.style.display = shown ? "" : "none";
    });
    f.querySelectorAll("[data-disabled-by]").forEach(function (el) {
      el.disabled = !!value(el.dataset.disabledBy);
    });
    f.querySelectorAll("[data-disabled-unless]").forEach(function (el) {
      el.disabled = !value(el.dataset.disabledUnless);
    });
  }
  document.addEventListener("change", syncShown);
  document.addEventListener("prefs-change", syncShown);

  Discourse.onPage(function () {
    pushSetup();
    // send_shortcut's meta key: Cmd on a Mac (formatShortcut("mod")).
    if (/Mac|iPhone|iPad/.test(navigator.platform)) {
      document.querySelectorAll('#user-send-shortcut [data-value="meta_enter"]').forEach(function (el) {
        el.dataset.name = el.dataset.name.replace("Ctrl", "⌘");
        el.title = el.dataset.name;
        var name = el.querySelector(".name");
        if (name) {
          name.textContent = el.dataset.name;
        }
      });
    }
  });
})();
