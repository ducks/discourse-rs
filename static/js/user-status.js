// User status in the browser: UserStatusModal (modal/user-status.gjs)
// with UserStatusPicker (the emoji picker and the description) and
// DTimeShortcutPicker's tiles for when it ends, the UserStatusService's
// set and clear (PUT and DELETE /user-status, entering or leaving do not
// disturb), the DUserStatusMessage tooltips (the emoji, the description
// and the "until"), and the header bubble's title. The pages draw the
// statuses (crate::users::views::status); the live stream redraws them.
//
//   Discourse.userStatus.open({ status, pauseNotifications,
//                               hidePauseNotifications, saveAction(status,
//                               pause) -> Promise, deleteAction() -> Promise })
//   Discourse.userStatus.set(status, pause) -> Promise
//   Discourse.userStatus.clear() -> Promise
//   Discourse.userStatus.messageHtml(status, showDescription, className)
//                                             DUserStatusMessage -> String
//   Discourse.userStatus.plainHtml(status, showDescription)
//                                             UserStatusMessage -> String
//   Discourse.userStatus.drawBubble(status)   the header avatar's bubble,
//                                             as currentUser.status changes
//
// The strings, the member's timezone and the scripts it loads on demand
// (moment for the shortcuts, the emoji picker) come with the script tag.
// The custom date and time tile is refused, not ported.
(function () {
  "use strict";

  if (!Discourse.once("user-status")) {
    return;
  }

  var script = document.currentScript;
  var s = {};
  var customEmoji = {};
  try {
    s = JSON.parse(script.dataset.strings || "{}");
    customEmoji = JSON.parse(script.dataset.customEmoji || "{}");
  } catch (e) {
    window.console && console.error("user-status strings", e);
  }
  var emojiUrlTemplate = script.dataset.emojiUrl || "";
  // DoNotDisturb.forever
  var FOREVER = "3000-01-01T00:00:00.000Z";

  function escapeHtml(text) {
    var div = document.createElement("div");
    div.textContent = text == null ? "" : String(text);
    return div.innerHTML.replace(/"/g, "&quot;").replace(/'/g, "&#39;");
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector('meta[name="csrf-token"]');
    return meta ? meta.content : "";
  }

  function showError(message) {
    if (Discourse.dialog && Discourse.dialog.popupAjaxError) {
      Discourse.dialog.popupAjaxError(message);
      return;
    }
    var error = document.getElementById("page-error");
    if (error) {
      error.textContent = message || "Something went wrong.";
      error.hidden = false;
    }
  }

  function unported(what) {
    var message = "Not ported yet: " + what + ".";
    if (Discourse.dialog && Discourse.dialog.alert) {
      Discourse.dialog.alert(escapeHtml(message));
      return;
    }
    showError(message);
  }

  // ajax as jQuery sends a form: nested keys as status[emoji].
  function encode(prefix, value, out) {
    if (value === undefined) {
      return;
    }
    if (value !== null && typeof value === "object") {
      Object.keys(value).forEach(function (k) {
        encode(prefix + "[" + k + "]", value[k], out);
      });
    } else {
      out.push(encodeURIComponent(prefix) + "=" + encodeURIComponent(value === null ? "" : value));
    }
  }

  function request(method, path, data) {
    var body = [];
    Object.keys(data || {}).forEach(function (k) {
      encode(k, data[k], body);
    });
    return fetch(basePath() + path, {
      method: method,
      credentials: "same-origin",
      headers: {
        "Content-Type": "application/x-www-form-urlencoded; charset=UTF-8",
        "X-CSRF-Token": csrf(),
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json, text/javascript, */*; q=0.01",
      },
      body: body.length ? body.join("&").replace(/%20/g, "+") : undefined,
    }).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (e) {
          json = null;
        }
        if (!response.ok) {
          throw new Error((json && json.errors && json.errors.join("\n")) || "Sorry, an error has occurred.");
        }
        return json;
      });
    });
  }

  // emojiUnescape(":<emoji>:"): a custom emoji's image, else the set's.
  function emojiUrl(code) {
    var name = code.split(":")[0];
    if (customEmoji[name]) {
      return customEmoji[name];
    }
    if (Discourse.emojiPicker && Discourse.emojiPicker.url) {
      var url = Discourse.emojiPicker.url(code);
      if (url) {
        return url;
      }
    }
    return emojiUrlTemplate.replace("%{name}", code.replace(/:t([1-6])$/, "/$1"));
  }

  // performEmojiUnescape's image: the code (no colons) as title and alt.
  function emojiHtml(code, skipTitle) {
    var title = escapeHtml(code);
    return (
      '<img width="20" height="20" src=\'' +
      escapeHtml(emojiUrl(code)) +
      "' " +
      (skipTitle ? "" : "title='" + title + "' ") +
      "alt='" +
      title +
      "' class='emoji'>"
    );
  }

  // DUserStatusMessage, as crate::users::views::status::message draws it.
  // `className` is the class given it (...attributes), after its own.
  function messageHtml(status, showDescription, className) {
    var inner = emojiHtml(status.emoji, true);
    if (showDescription) {
      inner += '<span class="user-status-message-description">' + escapeHtml(status.description) + "</span>";
    }
    return (
      '<span aria-expanded="false" class="fk-d-tooltip__trigger user-status-message' +
      (className ? " " + escapeHtml(className) : "") +
      '" data-identifier="user-status-message-tooltip" data-trigger="" role="button"' +
      dataAttrs(status) +
      '><span class="fk-d-tooltip__trigger-container">' +
      inner +
      "</span></span>"
    );
  }

  // What the tooltip is drawn from.
  function dataAttrs(status) {
    return (
      ' data-emoji="' +
      escapeHtml(status.emoji) +
      '" data-description="' +
      escapeHtml(status.description) +
      '"' +
      (status.ends_at ? ' data-ends-at="' + new Date(status.ends_at).getTime() + '"' : "")
    );
  }

  // lib/user-status-message's UserStatusMessage (chat's mentions, the
  // autocomplete), as crate::users::views::status::plain_message draws it.
  function plainHtml(status, showDescription) {
    var inner = emojiHtml(status.emoji, true);
    if (showDescription) {
      inner += '<span class="user-status-message-description">' + escapeHtml(status.description) + "</span>";
    }
    return (
      '<span class="user-status-message" data-identifier="user-status-message-tooltip"' + dataAttrs(status) + ">" + inner + "</span>"
    );
  }

  // The member's timezone, else the browser's (moment.tz.guess()).
  function zone() {
    if (s.timezone) {
      return s.timezone;
    }
    try {
      return Intl.DateTimeFormat().resolvedOptions().timeZone || "";
    } catch (e) {
      return "";
    }
  }

  // lib/formatter's until: the time when it ends today, else the date.
  function until(ms) {
    if (!Discourse.dates) {
      return "";
    }
    var z = zone();
    var sameDay = Discourse.dates.format(ms, "YYYY-MM-DD", z) === Discourse.dates.format(Date.now(), "YYYY-MM-DD", z);
    var formatted = Discourse.dates.format(ms, sameDay ? "LT" : s.long_no_year_no_time, z);
    return s.until + " " + formatted;
  }

  // The DUserStatusMessage tooltip's content.
  function tooltipContent(trigger) {
    var ends = parseInt(trigger.dataset.endsAt, 10);
    if (!trigger.classList.contains("fk-d-tooltip__trigger")) {
      // UserStatusMessage#tooltipHtml
      return (
        '<div class="user-status-message-tooltip-content">' +
        emojiHtml(trigger.dataset.emoji || "", true) +
        '<span class="user-status-tooltip-description">' +
        escapeHtml(trigger.dataset.description) +
        "</span>" +
        (isNaN(ends) ? "" : '<div class="user-status-tooltip-until">' + escapeHtml(until(ends)) + "</div>") +
        "</div>"
      );
    }
    var html =
      emojiHtml(trigger.dataset.emoji || "", true) +
      '<div class="user-status-tooltip-wrapper"><span class="user-status-tooltip-description">' +
      escapeHtml(trigger.dataset.description) +
      "</span>";
    var ms = parseInt(trigger.dataset.endsAt, 10);
    if (!isNaN(ms)) {
      html += '<span class="user-status-tooltip-until">' + escapeHtml(until(ms)) + "</span>";
    }
    return html + "</div>";
  }

  function openTooltip(trigger) {
    if (!Discourse.float || trigger.getAttribute("aria-expanded") === "true") {
      return;
    }
    Discourse.float.showTooltip(trigger, {
      identifier: "user-status-message-tooltip",
      content: tooltipContent(trigger),
    });
  }

  document.addEventListener("mouseover", function (event) {
    var trigger = event.target.closest && event.target.closest(".user-status-message[data-identifier]");
    if (trigger) {
      openTooltip(trigger);
    }
  });
  document.addEventListener("click", function (event) {
    var trigger = event.target.closest && event.target.closest(".user-status-message[data-identifier]");
    if (trigger && !trigger.closest("a, button")) {
      openTooltip(trigger);
    }
  });

  // UserStatusBubble's title: the description, and "Until" with the end in
  // the member's timezone (dates.long_date_without_year).
  function titleBubbles(root) {
    (root || document).querySelectorAll(".user-status-background[data-ends-at]").forEach(function (bubble) {
      var img = bubble.querySelector("img");
      var ms = parseInt(bubble.dataset.endsAt, 10);
      if (!img || isNaN(ms) || !Discourse.dates) {
        return;
      }
      var title = bubble.dataset.description + "\n" + s.until + " " + Discourse.dates.format(ms, s.long_date_without_year, zone());
      if (img.title !== title) {
        img.title = title;
      }
    });
  }
  titleBubbles(document);
  new MutationObserver(function () {
    titleBubbles(document.querySelector("#toggle-current-user"));
  }).observe(document.documentElement, { childList: true, subtree: true });

  // UserStatusBubble, as crate::users::views::status::bubble draws it,
  // put on the header avatar ahead of its badge (or taken off).
  function drawBubble(status) {
    var toggle = document.getElementById("toggle-current-user");
    if (!toggle) {
      return;
    }
    var old = toggle.querySelector(".user-status-background");
    if (old) {
      old.remove();
    }
    if (!status) {
      return;
    }
    var ends = status.ends_at ? ' data-ends-at="' + new Date(status.ends_at).getTime() + '"' : "";
    var html =
      '<div class="user-status-background" data-description="' +
      escapeHtml(status.description) +
      '"' +
      ends +
      ">" +
      emojiHtml(status.emoji, true)
        .replace("<img ", '<img title="' + escapeHtml(status.description) + '" ')
        .replace("alt='" + escapeHtml(status.emoji) + "'", "alt=':" + escapeHtml(status.emoji) + ":'") +
      "</div>";
    var avatar = toggle.querySelector("img.avatar");
    if (avatar) {
      avatar.insertAdjacentHTML("afterend", html);
    } else {
      toggle.insertAdjacentHTML("afterbegin", html);
    }
    titleBubbles(toggle);
  }

  // ---- UserStatusService

  // enterDoNotDisturbFor: minutes until the end (or DoNotDisturb.forever).
  function enterDoNotDisturb(endsAt) {
    var minutes = Math.floor((new Date(endsAt || FOREVER).getTime() - Date.now()) / 60000);
    return request("POST", "/do-not-disturb.json", { duration: minutes });
  }

  function isInDoNotDisturb() {
    var dnd = document.querySelector(".do-not-disturb-background");
    return !!dnd;
  }

  function set(status, pauseNotifications) {
    return request("PUT", "/user-status.json", status).then(function () {
      if (pauseNotifications) {
        enterDoNotDisturb(status.ends_at).catch(function (e) {
          showError(e.message);
        });
      } else if (isInDoNotDisturb()) {
        request("DELETE", "/do-not-disturb.json").catch(function (e) {
          showError(e.message);
        });
      }
    });
  }

  function clear() {
    return request("DELETE", "/user-status.json").then(function () {
      if (isInDoNotDisturb()) {
        request("DELETE", "/do-not-disturb.json").catch(function (e) {
          showError(e.message);
        });
      }
    });
  }

  // ---- loading what the modal needs

  function loadScript(src) {
    return new Promise(function (resolve, reject) {
      var tag = document.createElement("script");
      tag.src = src;
      tag.onload = resolve;
      tag.onerror = reject;
      document.head.appendChild(tag);
    });
  }

  var ready = null;
  function load() {
    if (!ready) {
      var moment = window.moment ? Promise.resolve() : loadScript(s.moment);
      ready = Promise.all([
        moment.then(function () {
          return window.moment.tz ? null : loadScript(s.moment_timezone);
        }),
        Discourse.emojiPicker || !s.emoji_picker ? null : loadScript(s.emoji_picker),
      ]);
    }
    return ready;
  }

  // timeShortcuts(timezone): oneHour, twoHours and tomorrow (8am), then
  // specialShortcutOptions' custom and none ("Never"), last custom hidden.
  function shortcuts() {
    var tz = s.timezone || window.moment.tz.guess();
    var now = function () {
      return window.moment.tz(tz);
    };
    return [
      { id: "one_hour", label: s.in_one_hour, time: now().add(1, "hours"), format: s.time },
      { id: "two_hours", label: s.in_two_hours, time: now().add(2, "hours"), format: s.time },
      {
        id: "tomorrow",
        label: s.tomorrow,
        time: now().add(1, "day").hour(8).startOf("hour"),
        format: s.time_short_day,
      },
      { id: "custom", label: s.custom, time: null },
      { id: "none", label: s.never, time: null },
    ];
  }

  // ---- UserStatusModal

  function open(model) {
    if (!Discourse.modal) {
      return;
    }
    load()
      .then(function () {
        show(model);
      })
      .catch(function () {
        showError("Sorry, an error has occurred.");
      });
  }

  function show(model) {
    // status = trackedObject({ ...model.status }): the end is sent only
    // when a tile picks one (status.endsAt).
    var status = {};
    if (model.status) {
      status.description = model.status.description;
      status.emoji = model.status.emoji;
      status.ends_at = model.status.ends_at;
    }
    var endsAt;
    var selected = null;
    // parsePrefilledDatetime: an end already set is the custom tile.
    if (status.ends_at) {
      selected = "custom";
    }
    var options = shortcuts();
    var footerEl = null;

    function pickerHtml() {
      var trigger = status.emoji ? emojiHtml(status.emoji, false) : s.icons["far-face-smile"];
      return (
        '<div class="user-status-picker-wrap"><div class="emoji-picker-anchor user-status-picker">' +
        '<button aria-expanded="false" class="btn no-text fk-d-menu__trigger emoji-picker-trigger btn-emoji" data-identifier="emoji-picker" data-trigger="" type="button">' +
        trigger +
        "&#8203;</button>" +
        '<input class="user-status-description" maxlength="100" placeholder="' +
        escapeHtml(s.what_are_you_doing) +
        '" type="text" autofocus="">' +
        "</div></div>"
      );
    }

    function tilesHtml() {
      return (
        '<div class="tap-tile-grid">' +
        options
          .map(function (o) {
            return (
              '<div id="tap_tile_' +
              o.id +
              '" class="tap-tile' +
              (selected === o.id ? " active" : "") +
              '" role="button" aria-pressed="' +
              (selected === o.id ? "true" : "false") +
              '" tabindex="0" data-tile="' +
              o.id +
              '"><div class="tap-tile-title">' +
              escapeHtml(o.label) +
              '</div><div class="tap-tile-date">' +
              (o.time ? escapeHtml(o.time.format(o.format)) : "") +
              "</div></div>"
            );
          })
          .join("") +
        "</div>"
      );
    }

    function refreshSave() {
      if (!footerEl) {
        return;
      }
      footerEl.querySelector(".btn-primary").disabled = !status.emoji || !status.description;
    }

    function selectTile(body, id) {
      if (id === "custom") {
        unported("the custom date and time of a status's end");
        return;
      }
      var option = options.filter(function (o) {
        return o.id === id;
      })[0];
      selected = id;
      endsAt = option && option.time ? option.time.toDate() : null;
      body.querySelector(".control-group-remove-status .tap-tile-grid").outerHTML = tilesHtml();
    }

    var close = Discourse.modal.show({
      className: "user-status",
      title: s.title,
      body: function (body) {
        body.innerHTML =
          '<div class="control-group">' +
          pickerHtml() +
          "</div>" +
          (model.hidePauseNotifications
            ? ""
            : '<div class="control-group pause-notifications"><label class="checkbox-label"><input type="checkbox"' +
              (model.pauseNotifications ? ' checked=""' : "") +
              "> " +
              escapeHtml(s.pause_notifications) +
              "</label></div>") +
          '<div class="control-group control-group-remove-status"><label class="control-label">' +
          escapeHtml(s.remove_status) +
          "</label>" +
          tilesHtml() +
          "</div>";
        var input = body.querySelector(".user-status-description");
        input.value = status.description || "";
        var picker = body.querySelector(".user-status-picker");
        input.addEventListener("input", function () {
          status.description = input.value;
          // UserStatusPicker#updateDescription
          if (!status.emoji) {
            status.emoji = "speech_balloon";
            body.querySelector(".emoji-picker-trigger").innerHTML = emojiHtml(status.emoji, false) + "&#8203;";
          }
          refreshSave();
        });
        input.addEventListener("focus", function () {
          picker.classList.add("focused");
        });
        input.addEventListener("blur", function () {
          picker.classList.remove("focused");
        });
        body.querySelector(".emoji-picker-trigger").addEventListener("click", function (event) {
          if (!Discourse.emojiPicker) {
            return;
          }
          var trigger = event.currentTarget;
          Discourse.emojiPicker.open(trigger, {
            context: "user-status",
            maxWidth: 405,
            onSelect: function (emoji) {
              status.emoji = emoji;
              trigger.innerHTML = emojiHtml(emoji, false) + "&#8203;";
              refreshSave();
            },
          });
        });
        body.addEventListener("click", function (event) {
          var tile = event.target.closest(".tap-tile");
          if (tile) {
            selectTile(body, tile.dataset.tile);
          }
        });
        body.addEventListener("keydown", function (event) {
          var tile = event.target.closest && event.target.closest(".tap-tile");
          if (tile && event.key === "Enter") {
            event.stopPropagation();
            selectTile(body, tile.dataset.tile);
          }
        });
      },
      footer: function (footer, closeModal) {
        footerEl = footer;
        footer.innerHTML =
          '<button class="btn btn-text btn-primary" type="button"><span class="d-button-label">' +
          escapeHtml(s.save) +
          "</span></button>" +
          '<button class="btn btn-text btn-flat d-modal-cancel" type="button"><span class="d-button-label">' +
          escapeHtml(s.cancel) +
          "</span></button>" +
          (model.status
            ? '<button class="btn no-text btn-icon delete-status btn-danger" type="button">' +
              s.icons["trash-can"] +
              '<span aria-hidden="true">&#8203;</span></button>'
            : "");
        refreshSave();
        footer.querySelector(".d-modal-cancel").addEventListener("click", closeModal);
        footer.querySelector(".btn-primary").addEventListener("click", function () {
          var pause = footer.parentNode.querySelector(".pause-notifications input");
          var newStatus = {
            description: status.description,
            emoji: status.emoji,
            ends_at: endsAt ? endsAt.toISOString() : undefined,
          };
          Promise.resolve(model.saveAction(newStatus, pause ? pause.checked : model.pauseNotifications))
            .then(closeModal)
            .catch(function (e) {
              showError(e && e.message ? e.message : String(e));
            });
        });
        var del = footer.querySelector(".delete-status");
        if (del) {
          del.addEventListener("click", function () {
            Promise.resolve(model.deleteAction())
              .then(closeModal)
              .catch(function (e) {
                showError(e && e.message ? e.message : String(e));
              });
          });
        }
      },
    });
    return close;
  }

  // UserStatusManager's tracked users in chat: a `user-status` event on
  // the page's stream (routes/live.rs) carries each user whose status
  // changed, and what chat shows of them is drawn again: by a message's
  // author (ChatMessageInfo), on mentions in messages
  // (updateUserStatusOnMention), after a one on one DM's title
  // (ChatChannelName), and in the members' (ChatUserInfo) and the message
  // creator's rows, those two with the description.
  function redrawChat(user) {
    var status = user.status;
    var name = String(user.username);
    document.querySelectorAll(".chat-message-info__username__name[data-user-card]").forEach(function (button) {
      if (button.getAttribute("data-user-card") !== name) {
        return;
      }
      var old = button.parentNode.querySelector(".chat-message-info__status");
      if (old) {
        old.remove();
      }
      if (status) {
        button.insertAdjacentHTML(
          "afterend",
          '<span class="chat-message-info__status">' + messageHtml(status, false) + "</span>"
        );
      }
    });
    var href = "/u/" + name.toLowerCase();
    document.querySelectorAll(".chat-message-text a.mention").forEach(function (mention) {
      var path = mention.getAttribute("href") || "";
      if (path.slice(-href.length) !== href) {
        return;
      }
      mention.querySelectorAll(".user-status-message").forEach(function (old) {
        old.remove();
      });
      if (status) {
        mention.insertAdjacentHTML("beforeend", plainHtml(status, false));
      }
    });
    document.querySelectorAll('.chat-channel-name__label[data-status-user-id="' + user.id + '"]').forEach(function (label) {
      label.querySelectorAll(".user-status-message").forEach(function (old) {
        old.remove();
      });
      if (status) {
        label.insertAdjacentHTML("beforeend", messageHtml(status, false, "chat-channel__user-status-message"));
      }
    });
    document
      .querySelectorAll(
        '.c-channel-members__list-item[data-user-id="' + user.id + '"] .user-status, ' +
          '.chat-message-creator__chatable[data-status-user-id="' + user.id + '"] .user-status'
      )
      .forEach(function (slot) {
        slot.innerHTML = status ? messageHtml(status, true) : "";
      });
  }

  document.addEventListener("htmx:sseBeforeMessage", function (event) {
    var message = event.detail;
    if (!message || message.type !== "user-status") {
      return;
    }
    event.preventDefault();
    var users = [];
    try {
      users = JSON.parse(message.data) || [];
    } catch (e) {
      return;
    }
    users.forEach(redrawChat);
  });

  Discourse.userStatus = {
    open: open,
    set: set,
    clear: clear,
    messageHtml: messageHtml,
    plainHtml: plainHtml,
    drawBubble: drawBubble,
  };
})();
