// The flag modal (components/modal/flag.gjs with FlagSelection and
// FlagActionType) as the topic route's showFlags and showFlagTopic open
// it: from a post's flag button (lib/flag-targets/post-flag.js) and from
// the topic footer's (lib/flag-targets/topic-flag.js).
//
// The flags the viewer may use (Site#flagTypes the post's actions summary
// can act on, notify user first; the topic flag types for a topic), each
// with its description, the message a flag that needs one takes (its
// length counted against min_personal_message_post_length and 500), the
// illegal flag's confirmation, then Flag Post or Message. For staff on a
// post: Take Action (hide, or suspend or silence through PenalizeUser in
// static/js/penalize-user.js with the flag made first), Official Warning
// for a message, and Delete Spammer for a spam flag on a deletable user.
// The flag is POST /post_actions (ActionSummary#act), the modal closed
// first as Flag#create does; the post is drawn again from the server after,
// as updateActionsSummary redraws it (its flag button gone, or hidden when
// staff took action). A post's count of flags for staff opens its
// reviewable.
//
// The post's model is fetched when its flag is opened (posts#by_number,
// render_post_json); the flag types, the topic's actions summary and the
// strings are the page's #flag-modal-config. Not ported, and saying so:
// official warnings, queueing for review and flags in messages.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (!Discourse.once || !Discourse.once("flag-modal")) {
    return;
  }

  var MAX_MESSAGE_LENGTH = 500;

  // Read again each time: a page swap brings another topic's block.
  function config() {
    var el = document.getElementById("flag-modal-config");
    try {
      return el ? JSON.parse(el.textContent) : null;
    } catch (e) {
      return null;
    }
  }

  var strings = {};
  function t(key, args) {
    var s = strings[key];
    if (s === undefined) {
      return "[en." + key + "]";
    }
    Object.keys(args || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(String(args[name]));
    });
    return s;
  }

  function tCount(key, count, args) {
    return t(key + (count === 1 ? ".one" : ".other"), Object.assign({ count: count }, args || {}));
  }

  // I18n.messageFormat for delete_confirm_MF's plurals and names.
  function messageFormat(key, values) {
    var src = t(key);
    return src
      .replace(/\{\s*(\w+),\s*plural,((?:[^{}]|\{[^{}]*\})*)\}/g, function (all, name, body) {
        var n = Number(values[name]) || 0;
        var forms = {};
        body.replace(/(=\d+|zero|one|two|few|many|other)\s*\{([^{}]*)\}/g, function (m, form, text) {
          forms[form] = text;
        });
        var form = forms["=" + n] !== undefined ? forms["=" + n] : n === 1 && forms.one !== undefined ? forms.one : forms.other;
        return (form || "").replace(/#/g, String(n));
      })
      .replace(/\{(\w+)\}/g, function (all, name) {
        return values[name] === undefined ? "" : escapeHtml(values[name]);
      })
      .trim();
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name, extra) {
    if (Discourse.cooked && Discourse.cooked.icon) {
      return Discourse.cooked.icon(name, extra);
    }
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon svg-string" +
      (extra ? " " + extra : "") +
      '" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function base() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrf() {
    var meta = document.querySelector("meta[name='csrf-token']");
    if (meta && meta.content) {
      return meta.content;
    }
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}")["X-CSRF-Token"] || "";
    } catch (e) {
      return "";
    }
  }

  // jQuery's param: undefined and null left out.
  function encode(data) {
    var params = new URLSearchParams();
    Object.keys(data).forEach(function (key) {
      if (data[key] !== undefined && data[key] !== null) {
        params.append(key, data[key]);
      }
    });
    return params.toString();
  }

  function extractError(json, response) {
    if (json) {
      if (json.errors && json.errors.length > 1) {
        return t("multiple_errors", {
          errors: json.errors
            .map(function (e, i) {
              return i + 1 + ") " + e;
            })
            .join(" "),
        });
      }
      if (json.errors && json.errors.length) {
        return t("generic_error_with_reason", { error: json.errors[0] });
      }
      if (json.error) {
        return json.error;
      }
      if (json.message) {
        return json.message;
      }
    }
    if (response && response.status >= 400) {
      return response.status + " " + (response.statusText || "");
    }
    return t("generic_error");
  }

  function ajax(method, path, data) {
    var options = {
      method: method,
      credentials: "same-origin",
      headers: {
        Accept: "application/json",
        "X-Requested-With": "XMLHttpRequest",
        "X-CSRF-Token": csrf(),
      },
    };
    if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = encode(data);
    }
    return fetch(base() + path, options).then(
      function (response) {
        return response.text().then(function (text) {
          var json = null;
          try {
            json = text ? JSON.parse(text) : null;
          } catch (e) {
            // Not JSON: the status decides.
          }
          if (!response.ok) {
            throw new Error(extractError(json, response));
          }
          return json;
        });
      },
      function () {
        throw new Error(t("generic_error"));
      }
    );
  }

  function popupAjaxError(error) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(error && error.message ? error.message : t("generic_error"));
    }
  }

  function notPorted(what) {
    if (Discourse.dialog) {
      Discourse.dialog.alert("Not ported yet: " + what + ".");
    }
  }

  function button(className, iconName, label, opts) {
    opts = opts || {};
    return (
      '<button class="btn ' +
      (iconName && label ? "btn-icon-text " : "") +
      className +
      '"' +
      (opts.disabled ? ' disabled=""' : "") +
      (opts.title ? ' title="' + escapeHtml(opts.title) + '"' : "") +
      ' type="button">' +
      (iconName ? icon(iconName) + " " : "") +
      '<span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  // postStream.refreshPost: the post drawn again from the live stream.
  function refreshPost(postId) {
    if (window.htmx && postId) {
      window.htmx.ajax("GET", base() + "/live/post/" + postId, { swap: "none" });
    }
  }

  // A post's model: render_post_json from posts#by_number.
  function loadPost(article) {
    var topic = article.closest("[data-topic-id]");
    var number = article.dataset.postNumber || (article.id || "").replace("post_", "");
    return ajax("GET", "/posts/by_number/" + topic.dataset.topicId + "/" + number + ".json");
  }

  // Site#flagTypes and the post's actionByName: the flags the post's
  // actions summary can act on.
  function postFlags(c, post) {
    var byId = {};
    (post.actions_summary || []).forEach(function (a) {
      byId[a.id] = a;
    });
    var flags = c.post_action_types.filter(function (type) {
      return type.is_flag && byId[type.id] && byId[type.id].can_act;
    });
    // PostFlag#flagsAvailable: the ones for posts, notify user first.
    flags = flags.filter(function (flag) {
      return (flag.applies_to || []).indexOf("Post") !== -1;
    });
    var notifyUser = flags.findIndex(function (flag) {
      return flag.name_key === "notify_user";
    });
    if (notifyUser !== -1) {
      flags.unshift(flags.splice(notifyUser, 1)[0]);
    }
    return flags;
  }

  // TopicFlag#flagsAvailable
  function topicFlags(c) {
    var summary = c.topic.actions_summary || [];
    return c.topic_flag_types.filter(function (item) {
      return summary.some(function (a) {
        return a.id === item.id && a.can_act;
      });
    });
  }

  // The flag targets (lib/flag-targets).
  var PostFlag = {
    targetsTopic: false,
    title: "flagging.title",
    submitLabel: "flagging.action",
    customSubmitLabel: "flagging.notify_action",
  };
  var TopicFlag = {
    targetsTopic: true,
    title: "flagging_topic.title",
    submitLabel: "flagging_topic.action",
    customSubmitLabel: "flagging_topic.notify_action",
  };
  // chat's lib/chat-message-flag.js: POST .../messages/:id/flags.
  var ChatMessageFlag = {
    targetsTopic: false,
    chat: true,
    title: "flagging.title",
    submitLabel: "chat.flagging.action",
    customSubmitLabel: "flagging.notify_action",
  };

  // ChatMessageFlag#flagsAvailable: the site's flag types the message
  // offers (its available_flags) that apply to chat messages, notify user
  // first, each described as chat describes it.
  function chatFlags(c, available) {
    var flags = c.post_action_types.filter(function (flag) {
      return available.indexOf(flag.name_key) !== -1 && (flag.applies_to || []).indexOf("Chat::Message") !== -1;
    });
    var notifyUser = flags.findIndex(function (flag) {
      return flag.name_key === "notify_user";
    });
    if (notifyUser !== -1) {
      flags.unshift(flags.splice(notifyUser, 1)[0]);
    }
    return flags.map(function (flag) {
      var key = "chat.flags." + flag.name_key;
      var copy = Object.assign({}, flag);
      if (c.strings[key]) {
        copy.description = c.strings[key].replace("%{basePath}", base());
      }
      return copy;
    });
  }

  function show(target, flagModel, flags) {
    var c = config();
    var staff = c.staff;
    var mobile = c.mobile;
    var state = {
      selected: null,
      message: undefined,
      isConfirmed: false,
      spammerDetails: null,
    };
    flags = flags.filter(function (flag) {
      return flag.enabled;
    });
    if (flags.length === 1) {
      state.selected = flags[0];
    }
    var staffFlagsAvailable = flags.length > 1;
    var footer = null;
    var form = null;
    var close = null;

    function submitEnabled() {
      var selected = state.selected;
      if (!selected) {
        return false;
      }
      if (!selected.require_message) {
        return true;
      }
      if (selected.name_key === "illegal" && !state.isConfirmed) {
        return false;
      }
      var len = (state.message || "").length;
      return len >= c.min_personal_message_post_length && len <= MAX_MESSAGE_LENGTH;
    }

    function canSendWarning() {
      return !target.targetsTopic && staff && state.selected && state.selected.name_key === "notify_user";
    }

    function canTakeAction() {
      return !target.targetsTopic && !(state.selected && state.selected.require_message) && staff;
    }

    function showDeleteSpammer() {
      return state.spammerDetails && state.spammerDetails.canDelete && state.selected && state.selected.name_key === "spam";
    }

    // FlagActionType#formattedName
    function formattedName(flag) {
      if (flag.name_key === "notify_user" || flag.name_key === "notify_moderators") {
        return String(flag.name || "").replace(/{{username}}|%{username}/, flagModel.username);
      }
      var formatted = strings["flagging.formatted_name." + flag.name_key];
      return formatted !== undefined ? formatted : flag.name;
    }

    function description(flag) {
      return mobile ? flag.short_description : flag.description;
    }

    function customPlaceholder(flag) {
      var own = strings["flagging.custom_placeholder_" + flag.name_key];
      return own !== undefined ? own : t("flagging.custom_placeholder_notify_moderators");
    }

    function lengthText() {
      var len = (state.message || "").length;
      var min = c.min_personal_message_post_length;
      if (len === 0) {
        return tCount("flagging.custom_message.at_least", min);
      }
      if (len < min) {
        return tCount("flagging.custom_message.more", min - len);
      }
      return tCount("flagging.custom_message.left", MAX_MESSAGE_LENGTH - len);
    }

    function lengthClass() {
      // `undefined < min` is false: no message yet reads as "ok".
      return state.message !== undefined && state.message.length < c.min_personal_message_post_length ? "too-short" : "ok";
    }

    function flagHtml(flag) {
      var input =
        '<input id="radio_' +
        escapeHtml(flag.name_key) +
        '" name="post_action_type_index" type="radio">';
      if (flag.name_key === "notify_user") {
        return (
          '<div class="flag-action-type notify_user"><h3>' +
          escapeHtml(formattedName(flag)) +
          '</h3><div class="controls"><label class="radio checkbox-label">' +
          input +
          '<div class="flag-action-type-details"><span class="description">' +
          (description(flag) || "") +
          "</span></div></label></div>" +
          (staffFlagsAvailable ? "<hr><h3>" + escapeHtml(t("flagging.notify_staff")) + "</h3>" : "") +
          "</div>"
        );
      }
      return (
        '<div class="flag-action-type ' +
        escapeHtml(flag.name_key) +
        '"><div class="controls ' +
        escapeHtml(flag.name_key) +
        '"><label class="radio checkbox-label">' +
        input +
        '<div class="flag-action-type-details"><strong class="flag-name">' +
        escapeHtml(formattedName(flag)) +
        '</strong><div class="description">' +
        (description(flag) || "") +
        "</div></div></label></div></div>"
      );
    }

    // FlagActionType's showMessageInput and showConfirmation for the
    // selected flag; the others lose theirs.
    function drawSelection() {
      form.querySelectorAll(".flag-message, .custom-message-length, .flag-confirmation").forEach(function (el) {
        el.remove();
      });
      form.querySelectorAll("input[type='radio']").forEach(function (radio) {
        radio.checked = !!state.selected && radio.id === "radio_" + state.selected.name_key;
      });
      var flag = state.selected;
      if (!flag) {
        return;
      }
      var wrapper = form.querySelector(".flag-action-type." + CSS.escape(flag.name_key));
      if (!wrapper) {
        return;
      }
      if (flag.require_message) {
        var details = wrapper.querySelector(".flag-action-type-details");
        var textarea = document.createElement("textarea");
        textarea.className = "flag-message";
        textarea.setAttribute(
          "aria-label",
          t(flag.name_key === "notify_user" ? "flagging.notify_user_textarea_label" : "flagging.notify_moderators_textarea_label")
        );
        textarea.name = "message";
        textarea.placeholder = customPlaceholder(flag);
        textarea.value = state.message || "";
        details.appendChild(textarea);
        var length = document.createElement("div");
        length.className = "custom-message-length " + lengthClass();
        length.textContent = lengthText();
        details.appendChild(length);
        textarea.addEventListener("input", function () {
          state.message = textarea.value;
          length.className = "custom-message-length " + lengthClass();
          length.textContent = lengthText();
          drawFooter();
        });
      }
      if (flag.name_key === "illegal") {
        var confirmation = document.createElement("label");
        confirmation.className = "checkbox-label flag-confirmation";
        confirmation.innerHTML =
          '<input name="confirmation" type="checkbox"><span>' + escapeHtml(t("flagging.confirmation_illegal")) + "</span>";
        var box = confirmation.querySelector("input");
        box.checked = state.isConfirmed;
        box.addEventListener("change", function () {
          state.isConfirmed = box.checked;
          drawFooter();
        });
        wrapper.querySelector(".controls").appendChild(confirmation);
      }
    }

    // ReviewableBundledAction for the take action bundle.
    function takeActionHtml(disabled) {
      var label = t("flagging.take_action");
      var filterBy = escapeHtml(t("select_kit.filter_by", { name: label }));
      var name = escapeHtml(label);
      return (
        '<details class="select-kit single-select dropdown-select-box' +
        (disabled ? " is-disabled" : "") +
        ' reviewable-action-dropdown btn-icon-text agree-and-hide"><summary aria-label="' +
        filterBy +
        '" name="' +
        filterBy +
        '" data-name="' +
        name +
        '" tabindex="0" class="select-kit-header single-select-header dropdown-select-box-header btn btn-icon-text btn-default"><div class="select-kit-header-wrapper"><div class="select-kit-selected-name selected-name choice" data-name="' +
        name +
        '" title="' +
        name +
        '"><span class="name">' +
        name +
        "</span></div>" +
        icon("angle-down", "angle-icon") +
        ' &#8203;</div></summary><div class="select-kit-body"></div></details>'
      );
    }

    var TAKE_ACTIONS = [
      { action_name: "agree_and_hide", option: "default" },
      { action_name: "agree_and_suspend", option: "suspend", client_action: "suspend" },
      { action_name: "agree_and_silence", option: "silence", client_action: "silence" },
    ];

    function takeActionRows() {
      return (
        '<div class="select-kit-filter"></div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
        TAKE_ACTIONS.map(function (a, index) {
          var label = escapeHtml(t("flagging.take_action_options." + a.option + ".title"));
          return (
            '<li aria-checked="false" role="menuitemradio" data-index="' +
            index +
            '" data-name="' +
            label +
            '" data-value="' +
            a.action_name +
            '" title="' +
            label +
            '" tabindex="0" class="select-kit-row dropdown-select-box-row"><div class="icons"><span class="selection-indicator"></span></div><div class="texts"><span class="name">' +
            label +
            '</span><span class="desc">' +
            escapeHtml(t("flagging.take_action_options." + a.option + ".details")) +
            "</span></div></li>"
          );
        }).join("") +
        "</ul>"
      );
    }

    function drawFooter() {
      if (!footer) {
        return;
      }
      var enabled = submitEnabled();
      var requireMessage = state.selected && state.selected.require_message;
      var html = button(
        "btn-primary flag-modal__create-flag",
        requireMessage ? "envelope" : "flag",
        t(requireMessage ? target.customSubmitLabel : target.submitLabel),
        { disabled: !enabled, title: t("flagging.submit_tooltip") }
      );
      if (canSendWarning()) {
        html += button("btn-danger flag-modal__send-warning", "triangle-exclamation", t("flagging.official_warning"), {
          disabled: !enabled,
        });
      }
      if (canTakeAction()) {
        html += takeActionHtml(!enabled);
        html += button("btn-danger flag-modal__flag-for-review", "triangle-exclamation", t("flagging.flag_for_review"), {
          disabled: !enabled,
        });
      }
      if (showDeleteSpammer()) {
        html += button("btn-danger delete-spammer flag-modal__delete-spammer", "triangle-exclamation", t("flagging.delete_spammer"), {
          disabled: !enabled,
        });
      }
      footer.innerHTML = html;
      var dropdown = footer.querySelector(".reviewable-action-dropdown");
      if (dropdown) {
        dropdown.addEventListener("toggle", function () {
          if (dropdown.classList.contains("is-disabled")) {
            dropdown.open = false;
            return;
          }
          dropdown.classList.toggle("is-expanded", dropdown.open);
          var body = dropdown.querySelector(".select-kit-body");
          if (!dropdown.open) {
            body.innerHTML = "";
            body.removeAttribute("style");
            return;
          }
          body.innerHTML = takeActionRows();
          floatBody(dropdown);
        });
      }
    }

    function floatBody(details) {
      var body = details.querySelector(".select-kit-body");
      var header = details.querySelector("summary");
      var fui = window.FloatingUIDOM;
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = Math.round(header.getBoundingClientRect().width) + "px";
      if (!fui) {
        return;
      }
      fui
        .computePosition(header, body, {
          placement: "bottom-start",
          strategy: "fixed",
          middleware: [fui.flip({ padding: 3 }), fui.shift({ limiter: fui.limitShift() }), fui.offset(3)],
        })
        .then(function (result) {
          body.style.transform = "translate(" + Math.round(result.x) + "px, " + Math.round(result.y) + "px)";
          body.style.visibility = "visible";
          body.style.pointerEvents = "auto";
        });
    }

    // Flag#create with ActionSummary#act: the modal closes, then the flag.
    // Resolves false when it wasn't created.
    function createFlag(opts) {
      opts = opts || {};
      if (state.selected.require_message) {
        opts.message = state.message;
      }
      var selected = state.selected;
      close();
      if (target.chat) {
        // The bus's self_flagged draws the flag mark.
        return ajax("POST", "/chat/api/channels/" + flagModel.channel_id + "/messages/" + flagModel.id + "/flags", {
          flag_type_id: selected.id,
          message: opts.message,
          is_warning: opts.isWarning,
          take_action: opts.takeAction,
          queue_for_review: opts.queue_for_review,
        }).then(
          function () {
            return true;
          },
          function (error) {
            popupAjaxError(error);
            return false;
          }
        );
      }
      if (c.topic.archetype === "private_message") {
        notPorted("flags in messages");
        return Promise.resolve(false);
      }
      return ajax("POST", "/post_actions", {
        id: flagModel.id,
        post_action_type_id: selected.id,
        message: opts.message,
        is_warning: opts.isWarning,
        take_action: opts.takeAction,
        queue_for_review: opts.queue_for_review,
        flag_topic: target.targetsTopic,
      }).then(
        function () {
          if (!target.targetsTopic) {
            refreshPost(flagModel.id);
          }
          return true;
        },
        function (error) {
          popupAjaxError(error);
          return false;
        }
      );
    }

    function createFlagAndHide(opts) {
      return createFlag(opts);
    }

    // Flag#penalize through adminTools: the author's AdminUser, the post
    // to act on, and the flag made before the penalty.
    function penalize(type) {
      if (!Discourse.penalizeUser) {
        notPorted("the " + type + " modal");
        return;
      }
      Discourse.penalizeUser
        .find(flagModel.user_id)
        .then(function (user) {
          var opts = {
            penaltyType: type,
            user: user,
            before: function () {
              return createFlag({ takeAction: true, skipClose: true });
            },
          };
          if (!target.targetsTopic) {
            opts.postId = flagModel.id;
            opts.postEdit = flagModel.cooked;
          }
          return Discourse.penalizeUser.show(opts);
        })
        .catch(popupAjaxError);
    }

    function takeAction(actionName) {
      var actionable = TAKE_ACTIONS.find(function (a) {
        return a.action_name === actionName;
      });
      if (!actionable) {
        return;
      }
      if (actionable.client_action) {
        penalize(actionable.client_action);
      } else {
        createFlagAndHide({ takeAction: true });
      }
    }

    // adminTools#_deleteSpammer
    function deleteSpammer() {
      var user = state.spammerDetails && state.spammerDetails.user;
      if (!user) {
        return;
      }
      if (c.moderators_view_emails) {
        notPorted("revealing the email");
        return;
      }
      Discourse.dialog.dialog({
        class: "flagging-delete-spammer",
        message: messageFormat("flagging.delete_confirm_MF", {
          POSTS: user.post_count,
          TOPICS: user.topic_count,
          email: user.email || t("flagging.hidden_email_address"),
          ip_address: user.ip_address || t("flagging.ip_address_missing"),
        }),
        buttons: [
          {
            label: t("flagging.yes_delete_spammer"),
            class: "btn-danger",
            icon: "triangle-exclamation",
            action: function () {
              ajax("DELETE", "/admin/users/" + user.id + ".json", {
                delete_posts: true,
                block_email: true,
                block_urls: true,
                block_ip: true,
                delete_as_spammer: true,
                context: location.pathname,
              })
                .then(function (result) {
                  if (result && result.deleted) {
                    location.reload();
                  } else {
                    throw new Error("failed to delete");
                  }
                })
                .catch(function () {
                  Discourse.dialog.alert(t("admin.user.delete_failed"));
                });
            },
          },
          { label: t("cancel"), class: "btn-default", action: "cancel" },
        ],
      });
    }

    close = Discourse.modal.show({
      className: "flag-modal",
      title: t(target.title),
      body: function (body) {
        body.classList.add("flag-modal-body");
        body.innerHTML = "<p>" + t("flagging.review_process_description") + "</p><form><div></div></form>";
        form = body.querySelector("form > div");
        form.innerHTML = flags.length ? flags.map(flagHtml).join("") : escapeHtml(t("flagging.cant"));
        form.addEventListener("click", function (event) {
          var radio = event.target.closest("input[name='post_action_type_index']");
          if (!radio) {
            return;
          }
          var flag = flags.find(function (f) {
            return radio.id === "radio_" + f.name_key;
          });
          if (flag && flag !== state.selected) {
            state.selected = flag;
            drawSelection();
            drawFooter();
          }
        });
        form.closest("form").addEventListener("submit", function (event) {
          event.preventDefault();
        });
        drawSelection();
      },
      footer: function (el) {
        footer = el;
        drawFooter();
        footer.addEventListener("click", function (event) {
          var row = event.target.closest(".reviewable-action-dropdown .select-kit-row");
          if (row) {
            row.closest("details").open = false;
            takeAction(row.dataset.value);
            return;
          }
          var btn = event.target.closest("button");
          if (!btn || btn.disabled || !submitEnabled()) {
            return;
          }
          if (btn.classList.contains("flag-modal__create-flag")) {
            createFlag();
          } else if (btn.classList.contains("flag-modal__send-warning")) {
            notPorted("official warnings");
          } else if (btn.classList.contains("flag-modal__flag-for-review")) {
            notPorted("queueing a post for review");
          } else if (btn.classList.contains("flag-modal__delete-spammer")) {
            deleteSpammer();
          }
        });
      },
    });

    // Flag#onKeydown: Ctrl or Cmd and Enter submits.
    var modal = footer && footer.closest(".d-modal");
    if (modal) {
      modal.addEventListener("keydown", function (event) {
        if (submitEnabled() && event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
          event.preventDefault();
          createFlag();
        }
      });
    }

    // adminTools.checkSpammer, for staff.
    if (staff && flagModel.user_id && Discourse.penalizeUser) {
      Discourse.penalizeUser
        .find(flagModel.user_id)
        .then(function (user) {
          state.spammerDetails = { user: user, canDelete: !!(user.can_be_deleted && user.can_delete_all_posts) };
          drawFooter();
        })
        .catch(function () {});
    }
  }

  document.addEventListener("click", function (event) {
    var c = config();
    if (!c || !Discourse.modal) {
      return;
    }
    strings = c.strings;
    // A post's reviewable, from its count of flags.
    var count = event.target.closest(".post-controls .button-count[data-reviewable-id]");
    if (count) {
      location.assign(base() + "/review/" + count.dataset.reviewableId);
      return;
    }
    var flag = event.target.closest(".post-controls .post-action-menu__flag");
    if (flag) {
      var article = flag.closest("article[data-post-id]");
      if (!article) {
        return;
      }
      flag.disabled = true;
      loadPost(article)
        .then(function (post) {
          show(PostFlag, post, postFlags(c, post));
        })
        .catch(popupAjaxError)
        .then(function () {
          flag.disabled = false;
        });
      return;
    }
    var topicFlag = event.target.closest(".flag-topic");
    if (topicFlag && c.topic) {
      show(TopicFlag, c.topic, topicFlags(c));
    }
  });

  // ChatMessageInteractor#flag: the modal over a chat message, its model
  // { id, channel_id, user_id, available_flags }.
  Discourse.flagModal = {
    showChat: function (message) {
      var c = config();
      if (!c || !Discourse.modal) {
        return;
      }
      strings = c.strings;
      show(ChatMessageFlag, message, chatFlags(c, message.available_flags || []));
    },
  };
})();
