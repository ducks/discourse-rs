// CreateInvite (components/modal/create-invite.gjs), as lib/invite-modal's
// showCreateInviteModal opens it.
//
//   Discourse.createInviteModal.open({ groupIds, onSaved(invite), onClose,
//                                      editing, invite })
//       The simple mode first: "Create link" makes an invite link with
//       the default redemptions and expiry (createLink) and shows it to
//       copy or share, the footer gone. "Edit link options" turns to the
//       form (FormKit's fields): a description, the email or domain it is
//       restricted to, the redemptions allowed (not for an email), when
//       it expires, the topic arrived at (TopicChooser, for staff where
//       members need no approval), the groups joined (staff and group
//       owners), and for
//       an email with allow_email_invites a custom message and "Create
//       link and send email". Saved with POST /invites, then PUT
//       /invites/:id for the invite made (Invite#save); the alert under
//       the header then carries the link. onSaved hears each save
//       (create-invite:saved).
//       With `editing` and the `invite` (InviteSerializer's JSON) the
//       form at once, for that invite: its link in the alert under the
//       header, its values in the fields, "Update" saving them with PUT.
//       An invite that exists has the moment it expires for a field
//       (DFutureDateInput without shortcuts, bookmark-modal.js's
//       Discourse.timeShortcutPicker.futureDateInput) where a new one has
//       the days until then.
//
// A link to /new-invite followed in the page opens the modal where the
// member is (routes/new-invite.js, a transition from another route); the
// page load of it is the server's.
//
// The strings, limits, groups and the link validity line are the script
// tag's (crate::invites::modal::script_attrs). A member who may not
// invite has the script too, with the refusal alone (refused_attrs):
// /new-invite shows it in a dialog.
//
// Not ported: CreateInviteWithRoles (enable_invite_modal_with_roles), and
// in TopicChooser's rows the topic's statuses and category.
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  var script = document.currentScript;
  if (!Discourse.once("create-invite-modal")) {
    return;
  }

  var s = {};
  try {
    s = JSON.parse((script && script.dataset.config) || "{}");
  } catch (e) {
    window.console && console.error("create-invite-modal config", e);
  }
  var strings = s.strings || {};

  var INVITE_DESCRIPTION_MAX_LENGTH = 100;
  var COPIED_MS = 3000;
  // lib/utilities' emailValid and hostnameValid.
  var EMAIL =
    /^[a-zA-Z0-9!#$%&'*+\/=?\^_`{|}~\-]+(?:\.[a-zA-Z0-9!#$%&'\*+\/=?\^_`{|}~\-]+)*@(?:[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?\.)+[a-zA-Z0-9](?:[a-zA-Z0-9\-]*[a-zA-Z0-9])?$/;
  var HOSTNAME =
    /^(([a-zA-Z0-9]|[a-zA-Z0-9][a-zA-Z0-9\-]*[a-zA-Z0-9])\.)+([A-Za-z0-9]|[A-Za-z0-9][A-Za-z0-9\-]*[A-Za-z0-9])$/;

  function t(key, args) {
    var text = strings[key] === undefined ? key : strings[key];
    Object.keys(args || {}).forEach(function (name) {
      text = text.split("%{" + name + "}").join(args[name]);
    });
    return text;
  }

  function escapeHtml(value) {
    return String(value === null || value === undefined ? "" : value)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return (s.icons && s.icons[name]) || "";
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      return {};
    }
  }

  // jQuery's param for what the form sends: a list as `key[]`.
  function encode(data) {
    var params = new URLSearchParams();
    Object.keys(data).forEach(function (key) {
      var value = data[key];
      if (value === undefined || value === null) {
        return;
      }
      if (Array.isArray(value)) {
        value.forEach(function (item) {
          params.append(key + "[]", item);
        });
      } else {
        params.append(key, value);
      }
    });
    return params.toString();
  }

  function request(method, path, data) {
    var headers = memberHeaders();
    headers.Accept = "application/json";
    headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
    return fetch(base() + path, {
      method: method,
      headers: headers,
      credentials: "same-origin",
      body: encode(data),
    }).then(function (r) {
      return r
        .json()
        .catch(function () {
          return {};
        })
        .then(function (body) {
          if (!r.ok) {
            // extractError
            var errors = body && body.errors;
            throw new Error(errors && errors.length ? errors.join("\n") : (body && body.error) || t("generic_error"));
          }
          return body;
        });
    });
  }

  // moment().add(days, "days").format("YYYY-MM-DD HH:mm:ssZ")
  // (DATE_INPUT_FORMAT), in the browser's zone.
  function expiresAt(days) {
    var at = new Date(Date.now() + days * 86400000);
    var pad = function (n) {
      return (n < 10 ? "0" : "") + n;
    };
    var offset = -at.getTimezoneOffset();
    var sign = offset < 0 ? "-" : "+";
    offset = Math.abs(offset);
    return (
      at.getFullYear() +
      "-" +
      pad(at.getMonth() + 1) +
      "-" +
      pad(at.getDate()) +
      " " +
      pad(at.getHours()) +
      ":" +
      pad(at.getMinutes()) +
      ":" +
      pad(at.getSeconds()) +
      sign +
      pad(Math.floor(offset / 60)) +
      ":" +
      pad(offset % 60)
    );
  }

  // pwa-utils' canNativeShare.
  function canNativeShare() {
    return (
      (document.documentElement.classList.contains("ios-device") || /Android|Windows Phone/.test(navigator.userAgent)) &&
      location.protocol === "https:" &&
      typeof navigator.share !== "undefined"
    );
  }

  // FKField's container around a control (src/admin/form_kit.rs).
  function field(name, control, opts, inner) {
    return (
      '<div class="form-kit__container form-kit__field form-kit__field-' +
      control +
      " --" +
      opts.format +
      '" data-name="' +
      name +
      '" id="control-' +
      name +
      '" data-control-type="' +
      control +
      '"><label class="form-kit__container-title" for="control-' +
      name +
      '-input"><span>' +
      escapeHtml(opts.title) +
      "</span></label>" +
      (opts.description
        ? '<p class="form-kit-text form-kit__container-description">' + escapeHtml(opts.description) + "</p>"
        : "") +
      '<div class="form-kit__container-content">' +
      inner +
      "</div></div>"
    );
  }

  function input(name, type, attrs) {
    return (
      '<div class="form-kit__control-input-wrapper"><input class="form-kit__control-input" id="control-' +
      name +
      '-input" name="' +
      name +
      '"' +
      (attrs || "") +
      ' type="' +
      type +
      '"></div>'
    );
  }

  function setError(form, name, message) {
    var container = form.querySelector('.form-kit__field[data-name="' + name + '"]');
    if (!container) {
      return;
    }
    var content = container.querySelector(".form-kit__container-content");
    var meta = content.querySelector(".form-kit__meta");
    if (meta) {
      meta.remove();
    }
    container.classList.toggle("has-error", !!message);
    if (message) {
      content.insertAdjacentHTML(
        "beforeend",
        '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors"><span>' +
          escapeHtml(message) +
          "</span></p></div>"
      );
    }
  }

  // GroupChooser (a select-kit MultiSelect over the groups, by id): the
  // chosen ones as buttons that take them out again, the rest in its
  // rows. { content: [{ id, name }], value: [ids], onChange(ids) }
  function groupChooser(opts) {
    var details = document.createElement("details");
    details.className = "select-kit multi-select group-chooser";
    details.innerHTML =
      '<summary tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"><div class="formatted-selection"></div></div></summary><div class="select-kit-body"></div>';
    var summary = details.querySelector("summary");
    var body = details.querySelector(".select-kit-body");
    var value = (opts.value || []).map(Number);

    function nameOf(id) {
      var group = opts.content.find(function (g) {
        return g.id === id;
      });
      return group ? group.name : String(id);
    }

    function drawHeader() {
      details.classList.toggle("has-selection", value.length > 0);
      summary.querySelector(".formatted-selection").innerHTML = value.length
        ? value
            .map(function (id) {
              return (
                '<button class="btn btn-icon-text btn-default selected-choice" data-value="' +
                id +
                '" data-name="' +
                escapeHtml(nameOf(id)) +
                '" type="button">' +
                icon("xmark") +
                '<span class="d-button-label">' +
                escapeHtml(nameOf(id)) +
                "</span></button>"
              );
            })
            .join("")
        : '<span class="name">' + escapeHtml(t("select_kit.default_header_text")) + "</span>";
    }

    function drawRows() {
      body.innerHTML =
        '<ul aria-live="polite" class="select-kit-collection" role="menu">' +
        opts.content
          .filter(function (g) {
            return value.indexOf(g.id) === -1;
          })
          .map(function (g, index) {
            return (
              '<li tabindex="0" title="' +
              escapeHtml(g.name) +
              '" data-value="' +
              g.id +
              '" data-name="' +
              escapeHtml(g.name) +
              '" data-index="' +
              index +
              '" role="menuitemcheckbox" aria-checked="false" class="select-kit-row"><span class="name">' +
              escapeHtml(g.name) +
              "</span></li>"
            );
          })
          .join("") +
        "</ul>";
    }

    function changed() {
      drawHeader();
      if (details.open) {
        drawRows();
      }
      opts.onChange(value.slice());
    }

    summary.addEventListener("click", function (event) {
      var chosen = event.target.closest(".selected-choice");
      if (chosen) {
        // deselect: the header stays as it is.
        event.preventDefault();
        value = value.filter(function (id) {
          return id !== Number(chosen.dataset.value);
        });
        changed();
      }
    });
    details.addEventListener("toggle", function () {
      details.classList.toggle("is-expanded", details.open);
      if (details.open) {
        drawRows();
      } else {
        body.innerHTML = "";
      }
    });
    var choose = function (event) {
      var row = event.target.closest(".select-kit-row");
      if (!row) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      value.push(Number(row.dataset.value));
      changed();
    };
    body.addEventListener("click", choose);
    body.addEventListener("keydown", function (event) {
      if (event.key === "Enter" || event.key === " ") {
        choose(event);
      }
    });
    drawHeader();
    return details;
  }

  // TopicChooser (a select-kit ComboBox over a search of the topics, with
  // additionalFilters "status:public"): the topic chosen in its header
  // with a button that clears it, a filter and the topics found in its
  // body. { content: [{ id, title }], value: id, onChange(id, topic) }
  function topicChooser(opts) {
    var FILTERS = "status:public";
    var details = document.createElement("details");
    details.className = "select-kit single-select combo-box topic-chooser";
    details.innerHTML =
      '<summary tabindex="0" class="select-kit-header single-select-header combo-box-header"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div>';
    var summary = details.querySelector("summary");
    var wrapper = details.querySelector(".select-kit-header-wrapper");
    var body = details.querySelector(".select-kit-body");
    var value = opts.value === undefined || opts.value === null ? null : Number(opts.value);
    var content = (opts.content || []).slice();
    var timer = null;
    var searches = 0;

    function chosen() {
      return content.find(function (topic) {
        return topic.id === value;
      });
    }

    function drawHeader() {
      var topic = chosen();
      var title = topic ? topic.title : value === null ? t("select_kit.default_header_text") : String(value);
      details.classList.toggle("has-selection", value !== null);
      summary.dataset.value = value === null ? "" : value;
      summary.dataset.name = topic ? topic.title : "";
      wrapper.innerHTML =
        '<div class="select-kit-selected-name selected-name choice" title="' +
        escapeHtml(title) +
        '"><span class="name">' +
        escapeHtml(title) +
        "</span></div>" +
        (value === null
          ? ""
          : '<button class="btn no-text btn-icon btn-clear" type="button">' + icon("xmark") + "</button>") +
        icon(details.open ? "angle-up" : "angle-down");
    }

    function drawRows(topics) {
      var list = body.querySelector(".select-kit-collection");
      if (!list) {
        return;
      }
      list.innerHTML = topics.length
        ? topics
            .map(function (topic, index) {
              return (
                '<li tabindex="0" title="' +
                escapeHtml(topic.title) +
                '" data-value="' +
                topic.id +
                '" data-name="' +
                escapeHtml(topic.fancy_title || topic.title) +
                '" data-index="' +
                index +
                '" role="menuitemradio" aria-checked="' +
                (topic.id === value) +
                '" class="select-kit-row topic-row' +
                (topic.id === value ? " is-selected" : "") +
                '"><div class="topic-title">' +
                escapeHtml(topic.title) +
                "</div></li>"
              );
            })
            .join("")
        : '<li class="no-content" role="alert">' + escapeHtml(t("select_kit.no_content")) + "</li>";
    }

    // search(filter): the public topics for no term; with one, the
    // regular topics by title, url or id.
    function search(filter) {
      var params = new URLSearchParams({ term: filter + " " + FILTERS });
      if (filter) {
        params.set("type_filter", "topic");
        params.set("restrict_to_archetype", "regular");
        params.set("search_for_id", "true");
      }
      var mine = ++searches;
      var headers = memberHeaders();
      headers.Accept = "application/json";
      fetch(base() + "/search/query?" + params.toString(), { credentials: "same-origin", headers: headers })
        .then(function (r) {
          return r.ok ? r.json() : {};
        })
        .catch(function () {
          return {};
        })
        .then(function (results) {
          if (mine !== searches || !details.open) {
            return;
          }
          // results.posts.map((item) => item.topic)
          var topics = (results && results.topics) || [];
          var found = [];
          ((results && results.posts) || []).forEach(function (post) {
            var topic = topics.find(function (candidate) {
              return candidate.id === post.topic_id;
            });
            if (topic && found.indexOf(topic) === -1) {
              found.push(topic);
            }
          });
          details.found = found;
          drawRows(found);
        });
    }

    function change(id, topic) {
      value = id;
      if (topic) {
        // onChangeTopic: the content is the topic chosen.
        content = [topic];
      }
      details.open = false;
      drawHeader();
      opts.onChange(id, topic || null);
    }

    summary.addEventListener("click", function (event) {
      if (event.target.closest(".btn-clear")) {
        event.preventDefault();
        change(null, null);
      }
    });
    details.addEventListener("toggle", function () {
      details.classList.toggle("is-expanded", details.open);
      drawHeader();
      if (!details.open) {
        clearTimeout(timer);
        body.innerHTML = "";
        return;
      }
      body.innerHTML =
        '<div class="select-kit-filter is-expanded"><input class="filter-input" type="search" autocomplete="off" spellcheck="false" placeholder="' +
        escapeHtml(t("choose_topic.title.placeholder")) +
        '"></div><ul aria-live="polite" class="select-kit-collection" role="menu"></ul>';
      var filter = body.querySelector(".filter-input");
      filter.addEventListener("input", function (event) {
        // The form's fields are not this one's.
        event.stopPropagation();
        clearTimeout(timer);
        timer = setTimeout(function () {
          search(filter.value.trim());
        }, 250);
      });
      filter.addEventListener("keydown", function (event) {
        if (event.key === "Enter") {
          event.preventDefault();
        }
      });
      filter.focus();
      search("");
    });
    var choose = function (event) {
      var row = event.target.closest(".select-kit-row");
      if (!row) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      var id = Number(row.dataset.value);
      change(
        id,
        (details.found || []).find(function (topic) {
          return topic.id === id;
        })
      );
    };
    body.addEventListener("click", choose);
    body.addEventListener("keydown", function (event) {
      if ((event.key === "Enter" || event.key === " ") && event.target.closest(".select-kit-row")) {
        choose(event);
      }
    });
    drawHeader();
    return details;
  }

  function open(model) {
    if (s.cannotInvite) {
      // #openInviteModalIfAllowed: who may not invite is told so.
      if (Discourse.dialog) {
        Discourse.dialog.alert(t("user.invited.cannot_invite_to_forum"));
      }
      return;
    }
    if (!Discourse.modal) {
      return;
    }
    model = model || {};
    var invite = model.invite ? Object.assign({}, model.invite) : null;
    var advanced = false;
    var expiresInput = null;
    var saving = false;
    var sendEmail = false;
    var flashText = null;
    var flashClass = "info";
    var bodyEl = null;
    var footerEl = null;
    var close = null;
    // this.data, as the form holds it.
    var had = invite || {};
    var topics = (had.topics || []).slice();
    var data = {
      description: had.description || "",
      // emailOrDomain
      restrictTo: had.email || had.domain || "",
      maxRedemptions:
        had.max_redemptions_allowed === undefined || had.max_redemptions_allowed === null
          ? s.defaultRedemptions
          : had.max_redemptions_allowed,
      // topicId
      inviteToTopic: had.topics && had.topics[0] ? had.topics[0].id : null,
      inviteToGroups: (
        model.groupIds ||
        (had.groups || []).map(function (g) {
          return g.id;
        })
      ).map(Number),
      customMessage: had.custom_message || "",
      expiresAfterDays: s.expiryDays,
      expiresAt: had.expires_at || null,
    };

    function isEmailInvite() {
      return EMAIL.test(data.restrictTo.trim());
    }

    function canSendEmailInvite() {
      return isEmailInvite() && s.allowEmailInvites;
    }

    function created() {
      return !!(invite && invite.id);
    }

    function simpleMode() {
      return !model.editing && !advanced;
    }

    // ShareOrCopyInviteLink
    function linkHtml() {
      return (
        '<div class="link-share-container"><input class="invite-link" name="invite-link" readonly type="text" value="' +
        escapeHtml(invite.link) +
        '">' +
        (canNativeShare()
          ? '<button class="btn btn-icon-text btn-primary share-invite-link" type="button">' +
            icon("share") +
            '<span class="d-button-label">' +
            escapeHtml(t("user.invited.invite.share_link")) +
            "</span></button>"
          : '<button class="btn btn-icon-text copy-button btn-primary" type="button">' +
            icon("copy") +
            '<span class="d-button-label">' +
            escapeHtml(t("user.invited.invite.copy_link")) +
            "</span></button>") +
        "</div>"
      );
    }

    // InviteModalAlert, under the header.
    function drawAlert() {
      var old = bodyEl.parentNode.querySelector("#modal-alert");
      if (old) {
        old.remove();
      }
      if (!flashText && !model.editing) {
        return;
      }
      var alert = document.createElement("div");
      alert.id = "modal-alert";
      alert.className = "alert alert-" + flashClass;
      alert.setAttribute("role", flashClass === "error" ? "alert" : "status");
      alert.innerHTML =
        '<div class="input-group invite-link"><label for="invite-link">' +
        escapeHtml(flashText || t("user.invited.invite.copy_link_and_share_it")) +
        "</label>" +
        (created() && flashClass !== "error" ? linkHtml() : "") +
        "</div>";
      bodyEl.parentNode.insertBefore(alert, bodyEl);
    }

    function formHtml() {
      var options = (s.expireAfterOptions || [])
        .map(function (option) {
          return (
            '<option class="d-native-select__option' +
            (Number(option.value) === Number(data.expiresAfterDays) ? " --selected" : "") +
            '"' +
            (Number(option.value) === Number(data.expiresAfterDays) ? ' selected=""' : "") +
            ' value="' +
            option.value +
            '">' +
            escapeHtml(option.text) +
            "</option>"
          );
        })
        .join("");
      return (
        '<form class="form-kit" novalidate="">' +
        field(
          "description",
          "input",
          {
            format: "full",
            title: t("user.invited.invite.description"),
            description: t("user.invited.invite.description_help"),
          },
          input("description", "text")
        ) +
        field(
          "restrictTo",
          "input",
          {
            format: "full",
            title: t("user.invited.invite.restrict"),
            description: t("user.invited.invite.restrict_help"),
          },
          input(
            "restrictTo",
            "text",
            ' placeholder="' + escapeHtml(t("user.invited.invite.email_or_domain_placeholder")) + '"'
          )
        ) +
        field(
          "maxRedemptions",
          "input-number",
          { format: "small", title: t("user.invited.invite.max_redemptions_allowed") },
          input("maxRedemptions", "number", ' max="' + s.maxRedemptionsLimit + '" min="1"')
        ) +
        (created()
          ? field("expiresAt", "custom", { format: "full", title: t("user.invited.invite.expires_at") }, "")
          : field(
              "expiresAfterDays",
              "select",
              { format: "full", title: t("user.invited.invite.expires_after") },
              '<select class="d-native-select form-kit__control-select" id="control-expiresAfterDays-input" name="expiresAfterDays">' +
                options +
                "</select>"
            )) +
        (s.canArriveAtTopic
          ? field("inviteToTopic", "custom", { format: "full", title: t("user.invited.invite.invite_to_topic") }, "")
          : "") +
        (s.canInviteToGroup
          ? field("inviteToGroups", "custom", { format: "full", title: t("user.invited.invite.add_to_groups") }, "")
          : "") +
        field(
          "customMessage",
          "textarea",
          {
            format: "full",
            title: t("user.invited.invite.custom_message"),
            description: t("user.invited.invite.custom_message_help"),
          },
          '<textarea class="form-kit__control-textarea" id="control-customMessage-input" name="customMessage" style="height: 100px" placeholder="' +
            escapeHtml(t("user.invited.invite.custom_message_placeholder")) +
            '"></textarea>'
        ) +
        "</form>"
      );
    }

    // What shows of the form follows the address typed: no redemptions
    // for an email, the message and the second button with one that may
    // be sent.
    function syncForm() {
      var form = bodyEl.querySelector("form.form-kit");
      if (!form) {
        return;
      }
      // ({{#unless}} and {{#if}} in the form: FormKit's containers are
      // displayed whatever their hidden attribute says.)
      form.querySelector('[data-name="maxRedemptions"]').style.display = isEmailInvite() ? "none" : "";
      form.querySelector('[data-name="customMessage"]').style.display = canSendEmailInvite() ? "" : "none";
      drawFooter();
    }

    // The expiresAt field's DFutureDateInput, once moment is there.
    function mountExpires(form) {
      var content = form.querySelector('[data-name="expiresAt"] .form-kit__container-content');
      var picker = Discourse.timeShortcutPicker;
      if (!content || !picker) {
        return;
      }
      picker.load().then(function () {
        if (!content.isConnected) {
          return;
        }
        expiresInput = picker.futureDateInput(content, {
          input: data.expiresAt,
          onChangeInput: function (value) {
            data.expiresAt = value;
            setError(form, "expiresAt", null);
          },
        });
      });
    }

    function drawBody() {
      if (expiresInput) {
        expiresInput.destroy();
        expiresInput = null;
      }
      if (simpleMode()) {
        bodyEl.innerHTML =
          (created()
            ? (document.documentElement.classList.contains("mobile-view")
                ? ""
                : "<p>" + escapeHtml(t("user.invited.invite.copy_link_and_share_it")) + "</p>") + linkHtml()
            : "<p>" + escapeHtml(t("user.invited.invite.create_link_to_invite")) + "</p>") +
          '<p class="link-limits-info">' +
          escapeHtml(s.validity) +
          '</p><p><button class="btn btn-text btn-link edit-link-options" type="button"><span class="d-button-label">' +
          escapeHtml(
            t(s.allowEmailInvites ? "user.invited.invite.edit_link_options" : "user.invited.invite.edit_link_options_only")
          ) +
          "</span></button></p>";
        return;
      }
      bodyEl.innerHTML = formHtml();
      var form = bodyEl.querySelector("form");
      form.elements.description.value = data.description;
      form.elements.restrictTo.value = data.restrictTo;
      form.elements.maxRedemptions.value = data.maxRedemptions;
      form.elements.customMessage.value = data.customMessage;
      var arriveAt = form.querySelector('[data-name="inviteToTopic"] .form-kit__container-content');
      if (arriveAt) {
        arriveAt.appendChild(
          topicChooser({
            content: topics,
            value: data.inviteToTopic,
            onChange: function (id, topic) {
              // onChangeTopic
              if (topic) {
                topics = [topic];
              }
              data.inviteToTopic = id;
            },
          })
        );
      }
      var groups = form.querySelector('[data-name="inviteToGroups"] .form-kit__container-content');
      if (groups) {
        groups.appendChild(
          groupChooser({
            content: s.groups || [],
            value: data.inviteToGroups,
            onChange: function (ids) {
              data.inviteToGroups = ids;
            },
          })
        );
      }
      form.addEventListener("submit", function (event) {
        event.preventDefault();
        saveInvite(false);
      });
      form.addEventListener("input", function (event) {
        var name = event.target.name;
        if (name && Object.prototype.hasOwnProperty.call(data, name)) {
          data[name] = event.target.value;
          setError(form, name, null);
        }
        if (name === "restrictTo") {
          // handleRestrictToChange
          syncForm();
        }
      });
      form.addEventListener("change", function (event) {
        if (event.target.name === "expiresAfterDays") {
          data.expiresAfterDays = Number(event.target.value);
        }
      });
      mountExpires(form);
      syncForm();
    }

    function drawFooter() {
      if (!footerEl) {
        return;
      }
      // @hideFooter: the simple mode, once the link is made.
      footerEl.style.display = simpleMode() && created() ? "none" : "";
      var disabled = saving ? ' disabled=""' : "";
      var html =
        '<button class="btn btn-text btn-primary save-invite"' +
        disabled +
        ' type="button"><span class="d-button-label">' +
        escapeHtml(t(!simpleMode() && created() ? "user.invited.invite.update_invite" : "user.invited.invite.create_link")) +
        "</span></button>";
      if (!simpleMode() && canSendEmailInvite()) {
        html +=
          '<button class="btn btn-text btn-primary save-invite-and-send-email"' +
          disabled +
          ' type="button"><span class="d-button-label">' +
          escapeHtml(
            t(
              created()
                ? "user.invited.invite.update_invite_and_send_email"
                : "user.invited.invite.create_link_and_send_email"
            )
          ) +
          "</span></button>";
      }
      html +=
        '<button class="btn btn-text btn-transparent cancel-button" type="button"><span class="d-button-label">' +
        escapeHtml(t("user.invited.invite.cancel")) +
        "</span></button>";
      footerEl.innerHTML = html;
    }

    function draw() {
      drawAlert();
      drawBody();
      drawFooter();
    }

    // save(data): Invite#save, a POST for a new invite and a PUT for the
    // one made here.
    function save(submit) {
      var isLink = true;
      var restrictTo = submit.emailOrDomain;
      delete submit.emailOrDomain;
      if (restrictTo) {
        if (isEmailInvite()) {
          isLink = false;
          submit.email = restrictTo;
        } else if (HOSTNAME.test(restrictTo)) {
          submit.domain = restrictTo;
        }
      }
      if (isLink) {
        if (invite && invite.email) {
          submit.email = submit.custom_message = "";
        }
        // The server touches the column only when the key is there: an
        // emptied field sends a blank to drop the restriction.
        if (!restrictTo && invite && invite.domain) {
          submit.domain = "";
        }
      } else {
        if (submit.max_redemptions_allowed > 1) {
          submit.max_redemptions_allowed = 1;
        }
        if (sendEmail) {
          submit.send_email = true;
          if (submit.topic_id) {
            submit.invite_to_topic = true;
          }
        } else {
          submit.skip_email = true;
        }
      }
      saving = true;
      drawFooter();
      var wasCreated = created();
      var sent = wasCreated
        ? request("PUT", "/invites/" + encodeURIComponent(invite.id), submit)
        : request("POST", "/invites", submit);
      return sent
        .then(function (saved) {
          invite = Object.assign(invite || {}, saved);
          if (!wasCreated) {
            // The form's field from here on is the moment it expires.
            data.expiresAt = invite.expires_at || null;
          }
          if (!simpleMode()) {
            flashText = t(
              sendEmail
                ? "user.invited.invite.invite_saved_with_sending_email"
                : "user.invited.invite.invite_saved_without_sending_email"
            );
            flashClass = "success";
          }
          if (model.onSaved) {
            model.onSaved(invite);
          }
          return !wasCreated;
        })
        .catch(function (error) {
          flashText = error && error.message ? error.message : String(error);
          flashClass = "error";
          return false;
        })
        .then(function (focusLink) {
          saving = false;
          draw();
          // maybeFocusCopyButton: the link's button, once it is made.
          if (focusLink) {
            var button = document.querySelector(".create-invite-modal .link-share-container button");
            if (button) {
              button.focus();
            }
          }
        });
    }

    // createLink: the simple mode's, with the defaults.
    function createLink() {
      sendEmail = false;
      var submit = {
        max_redemptions_allowed: s.defaultRedemptions,
        expires_at: expiresAt(s.expiryDays),
      };
      if (data.inviteToGroups.length > 0) {
        submit.group_ids = data.inviteToGroups;
      }
      return save(submit);
    }

    // onFormSubmit, after the form's validations.
    function saveInvite(withEmail) {
      var form = bodyEl.querySelector("form.form-kit");
      if (!form || saving) {
        return;
      }
      var valid = true;
      if (data.description.length > INVITE_DESCRIPTION_MAX_LENGTH) {
        setError(
          form,
          "description",
          t("form_kit.errors.too_long", { count: INVITE_DESCRIPTION_MAX_LENGTH, maximum: INVITE_DESCRIPTION_MAX_LENGTH })
        );
        valid = false;
      }
      if (!isEmailInvite() && String(data.maxRedemptions).trim() === "") {
        setError(form, "maxRedemptions", t("form_kit.errors.required"));
        valid = false;
      }
      if (created() && !data.expiresAt) {
        setError(form, "expiresAt", t("form_kit.errors.required"));
        valid = false;
      }
      if (!valid) {
        return;
      }
      sendEmail = withEmail;
      var submit = {
        description: data.description,
        emailOrDomain: data.restrictTo.trim(),
        group_ids: data.inviteToGroups,
        max_redemptions_allowed: Number(data.maxRedemptions),
        custom_message: data.customMessage,
        expires_at: created() ? data.expiresAt : expiresAt(Number(data.expiresAfterDays)),
      };
      // inviteToTopic, blank for none as jQuery's param sends it: the
      // server takes the key without a value as the topic taken away.
      submit.topic_id = data.inviteToTopic === null ? "" : data.inviteToTopic;
      save(submit);
    }

    // DCopyButton: the link to the clipboard, the button saying so a while.
    function copy(button) {
      var link = button.parentNode.querySelector("input.invite-link");
      if (!navigator.clipboard || !link) {
        return;
      }
      navigator.clipboard.writeText(link.value).then(
        function () {
          button.classList.add("ok");
          button.querySelector("svg").outerHTML = icon("check");
          button.querySelector(".d-button-label").textContent = t("user.invited.invite.link_copied");
          clearTimeout(button.restore);
          button.restore = setTimeout(function () {
            if (!button.isConnected) {
              return;
            }
            button.classList.remove("ok");
            button.querySelector("svg").outerHTML = icon("copy");
            button.querySelector(".d-button-label").textContent = t("user.invited.invite.copy_link");
          }, COPIED_MS);
        },
        function () {}
      );
    }

    function onClick(event) {
      var button = event.target.closest("button");
      if (!button) {
        return;
      }
      if (button.classList.contains("copy-button")) {
        copy(button);
      } else if (button.classList.contains("share-invite-link")) {
        navigator.share({ url: invite.link }).catch(function () {});
      } else if (button.classList.contains("edit-link-options")) {
        // showAdvancedMode, the first field focused.
        advanced = true;
        draw();
        var first = bodyEl.querySelector(".form-kit__control-input");
        if (first) {
          first.focus();
        }
      } else if (button.classList.contains("save-invite")) {
        if (simpleMode()) {
          createLink();
        } else {
          saveInvite(false);
        }
      } else if (button.classList.contains("save-invite-and-send-email")) {
        saveInvite(true);
      } else if (button.classList.contains("cancel-button")) {
        close();
      }
    }

    close = Discourse.modal.show({
      className: "create-invite-modal",
      title: t(model.editing ? "user.invited.invite.edit_title" : "user.invited.invite.new_title"),
      onClose: function () {
        if (expiresInput) {
          expiresInput.destroy();
          expiresInput = null;
        }
        if (model.onClose) {
          model.onClose();
        }
      },
      body: function (body) {
        bodyEl = body;
      },
      footer: function (footer) {
        footerEl = footer;
      },
    });
    var modal = document.querySelector(".d-modal.create-invite-modal");
    if (!modal || !bodyEl) {
      return;
    }
    modal.addEventListener("click", onClick);
    draw();
    // autofocus: the simple mode's button, or the one that sends the email.
    var primary =
      footerEl && footerEl.querySelector(simpleMode() ? ".save-invite" : ".save-invite-and-send-email");
    if (primary) {
      primary.focus();
    }
  }

  // routes/new-invite.js from within the app: the modal where the member
  // is. Taken ahead of the link's navigation.
  document.addEventListener(
    "click",
    function (event) {
      if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }
      var link = event.target.closest && event.target.closest("a[href]");
      if (!link || (link.target && link.target !== "_self")) {
        return;
      }
      var url = new URL(link.href, location.href);
      if (url.origin !== location.origin || url.pathname !== base() + "/new-invite") {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      open({});
    },
    true
  );

  // The page load of /new-invite lands on the homepage with this
  // fragment (routes/invites.rs): the modal there, the address cleaned.
  if (location.hash === "#new-invite") {
    history.replaceState(history.state, "", location.pathname + location.search);
    // Once the scripts after this one (the dialog's) have run.
    if (document.readyState !== "complete") {
      document.addEventListener("DOMContentLoaded", function () {
        open({});
      });
    } else {
      open({});
    }
  }

  // What offers the modal elsewhere looks for this: not there for who
  // may not invite.
  if (!s.cannotInvite) {
    Discourse.createInviteModal = { open: open };
  }
})();
