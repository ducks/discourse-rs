// Chat's channel menus and the create channel modal, wherever chat's
// lists are drawn (the sidebar, the drawer, full-page and mobile chat):
//
// - ChatChannelListOptionsButton and the sidebar section headers'
//   channelListOptions action open ChatChannelListSidebarMenu: New
//   channel (staff), Browse channels, the new DM and group chat modes of
//   chat-new-message.js, and the section's filter and sort submenus
//   (ChatChannelListFilterMenu, ChatChannelListSortMenu), saved as the
//   member's chat_channel_list_* options (PUT /u/:username.json) and the
//   page drawn again. ChatChannelListFilterToggle and toggleChannelFilter
//   go back to all.
// - A sidebar channel link's hover button opens
//   ChatChannelSidebarContextMenu: the notification submenu (levels and
//   mute, PUT notifications-settings/me), the channel's settings, star
//   and unstar (PUT memberships/me), and leaving.
// - The browse page's New channel button and the menu's New channel open
//   ChatModalCreateChannel: name, slug (POST /slugs.json), description,
//   the category with the security hint (GET
//   /chat/api/category-chatables/:id/permissions), emoji, auto join and
//   threading; POST /chat/api/channels, then the new channel.
(function () {
  "use strict";

  if (!Discourse.once("chat-channel-menus")) {
    return;
  }

  var script = document.currentScript;
  var cfg = {};
  try {
    cfg = JSON.parse((script && script.dataset.config) || "{}");
  } catch (_) {}
  var s = cfg.strings || {};
  var FILTERS = ["all", "active", "unread", "mentions"];
  var SORTS = ["alphabetical", "recent_activity", "priority"];
  var FIELDS = {
    channels: { filter: "chat_channel_list_filter", sort: "chat_channel_list_sort" },
    starred: { filter: "chat_channel_list_filter_starred", sort: "chat_channel_list_sort_starred" },
    dms: { filter: "chat_channel_list_filter_dms", sort: "chat_channel_list_sort_dms" },
  };
  // CHANNEL_LIST_SECTION_OPTIONS
  var SECTION_OPTIONS = {
    channels: { create: true, browse: true },
    starred: {},
    dms: { newMessage: true },
  };

  function basePath() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function headers(json) {
    var h = {};
    try {
      h = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (_) {}
    h["X-Requested-With"] = "XMLHttpRequest";
    h.Accept = "application/json";
    if (json) {
      h["Content-Type"] = "application/json";
    }
    return h;
  }

  function escapeHtml(v) {
    return String(v == null ? "" : v)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name, extra) {
    var href = name === "d-chat" ? "comment" : name;
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      href +
      '"></use></svg>'
    );
  }

  function request(method, url, data) {
    return fetch(basePath() + url, {
      method: method,
      credentials: "same-origin",
      headers: headers(!!data),
      body: data ? JSON.stringify(data) : undefined,
    }).then(function (r) {
      return r
        .json()
        .catch(function () {
          return {};
        })
        .then(function (body) {
          if (!r.ok) {
            var text =
              (body.errors && body.errors.join("\n")) || body.error || s.generic_error || String(r.status);
            throw new Error(text);
          }
          return body;
        });
    });
  }

  function popupError(error) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(error.message);
    } else {
      window.alert(error.message);
    }
  }

  // A route in the app, through the drawer's router when there is one.
  function go(url) {
    if (Discourse.chatDrawer && Discourse.chatDrawer.transitionTo) {
      Discourse.chatDrawer.transitionTo(url);
      return;
    }
    if (!window.htmx) {
      window.location.assign(basePath() + url);
      return;
    }
    var link = document.createElement("a");
    link.href = basePath() + url;
    link.hidden = true;
    (document.getElementById("main-outlet") || document.body).appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // The page drawn again where it is, for a preference or membership
  // change the lists show.
  function reload() {
    var path = location.pathname.slice(basePath().length) + location.search;
    if (Discourse.chatDrawer && Discourse.chatDrawer.isActive && Discourse.chatDrawer.isActive()) {
      window.location.reload();
      return;
    }
    go(path);
  }

  function item(inner, cls) {
    return '<li class="dropdown-menu__item' + (cls ? " " + cls : "") + '" role="none">' + inner + "</li>";
  }

  function menuButton(id, label, opts) {
    opts = opts || {};
    return (
      '<button class="btn ' +
      (opts.icon ? "btn-icon-text" : "btn-text") +
      (opts.cls ? " " + opts.cls : "") +
      '" data-menu-option-id="' +
      escapeHtml(id) +
      '" role="' +
      (opts.role || "menuitem") +
      '"' +
      (opts.checked == null ? "" : ' aria-checked="' + (opts.checked ? "true" : "false") + '"') +
      ' type="button">' +
      (opts.icon ? icon(opts.icon) : "") +
      '<span class="d-button-label">' +
      escapeHtml(label) +
      "</span>" +
      (opts.suffix ? icon(opts.suffix, "d-button__suffix-icon") : "") +
      "</button>"
    );
  }

  function sectionOf(el) {
    var named = el.closest("[data-section-name]");
    var name = named ? named.dataset.sectionName : "";
    if (/starred/.test(name) || el.closest(".starred-channels, .--starred-channels")) {
      return "starred";
    }
    if (/direct-messages|chat-dms/.test(name) || el.closest(".direct-message-channels, .--direct-messages")) {
      return "dms";
    }
    return "channels";
  }

  function current(section, kind) {
    var prefs = (cfg.prefs || {})[section] || {};
    return prefs[kind] || (kind === "filter" ? "all" : "alphabetical");
  }

  function savePreference(section, kind, value) {
    var field = FIELDS[section][kind];
    var data = {};
    data[field] = value;
    return request("PUT", "/u/" + encodeURIComponent(cfg.username || "") + ".json", data)
      .then(function () {
        cfg.prefs = cfg.prefs || {};
        cfg.prefs[section] = cfg.prefs[section] || {};
        cfg.prefs[section][kind] = value;
        reload();
      })
      .catch(popupError);
  }

  // ChatChannelListFilterMenu and ChatChannelListSortMenu.
  function submenu(trigger, section, kind) {
    var values = kind === "filter" ? FILTERS : SORTS;
    var selected = current(section, kind);
    var html =
      '<ul class="dropdown-menu chat-channel-list-' +
      kind +
      '-menu" role="group" aria-label="' +
      escapeHtml(s[kind + "_title"]) +
      '"><li class="dropdown-menu__subheader" role="presentation">' +
      escapeHtml(s[kind + "_title"]) +
      "</li>" +
      values
        .map(function (v) {
          return item(
            menuButton(v, s[kind + "_" + v], {
              role: "menuitemradio",
              checked: v === selected,
              icon: "check",
              cls: "chat-channel-list-" + kind + "-menu__" + v,
            })
          );
        })
        .join("") +
      "</ul>";
    var instance = Discourse.float.menu(trigger, {
      identifier: "chat-channel-list-" + kind + "-menu",
      content: html,
      placement: "right-start",
      modalForMobile: true,
    });
    instance.content.addEventListener("click", function (event) {
      var button = event.target.closest("[data-menu-option-id]");
      if (!button) {
        return;
      }
      Discourse.float.close("chat-channel-list-" + kind + "-menu");
      Discourse.float.close("chat-channel-list-options-menu");
      savePreference(section, kind, button.dataset.menuOptionId);
    });
  }

  // ChatChannelListSidebarMenu
  function optionsMenu(trigger) {
    var section = sectionOf(trigger);
    var options = SECTION_OPTIONS[section];
    var rows = [];
    if (options.create && cfg.staff) {
      rows.push(item(menuButton("createChannel", s.create, { suffix: "plus" })));
    }
    if (options.browse) {
      rows.push(item(menuButton("browseChannels", s.browse)));
    }
    if (options.newMessage && cfg.canDirectMessage) {
      rows.push(item(menuButton("startDm", s.new_dm, { suffix: "plus" })));
      if (cfg.canGroupChat) {
        rows.push(item(menuButton("startGroupChat", s.new_group, { suffix: "plus" })));
      }
    }
    if (rows.length) {
      rows.push('<li role="none"><hr class="dropdown-menu__divider"></li>');
    }
    ["filter", "sort"].forEach(function (kind) {
      var label = s[kind + "_" + current(section, kind)];
      rows.push('<li class="dropdown-menu__subheader" role="presentation">' + escapeHtml(s[kind]) + "</li>");
      rows.push(
        item(
          '<button aria-haspopup="menu" class="btn btn-text fk-d-menu__trigger" data-menu-option-id="' +
            kind +
            'Channels" role="menuitem" aria-label="' +
            escapeHtml((s["current_" + kind] || "").replace("%{" + kind + "}", label)) +
            '" type="button"><span class="d-button-label">' +
            escapeHtml(label) +
            '</span><span class="d-button__suffix-icon">' +
            icon("angle-right") +
            "</span></button>"
        )
      );
    });
    var instance = Discourse.float.menu(trigger, {
      identifier: "chat-channel-list-options-menu",
      content: '<ul class="dropdown-menu chat-channel-list-options-menu" role="none">' + rows.join("") + "</ul>",
      placement: "right-start",
      modalForMobile: true,
    });
    instance.content.addEventListener("click", function (event) {
      var button = event.target.closest("[data-menu-option-id]");
      if (!button) {
        return;
      }
      var id = button.dataset.menuOptionId;
      if (id === "filterChannels" || id === "sortChannels") {
        submenu(button, section, id === "filterChannels" ? "filter" : "sort");
        return;
      }
      Discourse.float.close("chat-channel-list-options-menu");
      if (id === "createChannel") {
        createChannel();
      } else if (id === "browseChannels") {
        go("/chat/browse/open");
      } else if (id === "startDm" && Discourse.chatNewMessage) {
        Discourse.chatNewMessage.show();
      } else if (id === "startGroupChat" && Discourse.chatNewMessage) {
        Discourse.chatNewMessage.show("NEW_GROUP");
      }
    });
  }

  // ChatChannelSidebarContextMenu, from the channel as the API serializes
  // it for the member.
  function contextMenu(trigger) {
    var link = trigger.closest(".sidebar-section-link-wrapper");
    var anchor = link && link.querySelector("[class*='channel-']");
    var match = anchor && /(?:^|\s)channel-(\d+)/.exec(anchor.className);
    if (!match) {
      return;
    }
    var id = match[1];
    request("GET", "/chat/api/channels/" + id)
      .then(function (body) {
        var c = body.channel;
        var m = c.current_user_membership || {};
        var dm = c.chatable_type === "DirectMessage";
        var starLabel = m.starred ? s.unstar : s.star;
        var leaveLabel = dm ? s.close_dm : s.leave;
        var html =
          '<ul class="dropdown-menu chat-channel-sidebar-link-menu">' +
          item(
            menuButton("notifications", s.notifications, {
              icon: "bell",
              suffix: "angle-right",
              cls: "chat-channel-sidebar-link-menu__open-notification-settings",
            })
          ) +
          item(menuButton("settings", s.settings, { icon: "gear", cls: "chat-channel-sidebar-link-menu__channel-settings" })) +
          item(
            menuButton("star", starLabel, {
              icon: m.starred ? "star" : "far-star",
              cls: "chat-channel-sidebar-link-menu__star-channel",
            })
          ) +
          item(menuButton("leave", leaveLabel, { icon: "xmark", cls: "chat-channel-sidebar-link-menu__leave-channel --danger" })) +
          "</ul>";
        var identifier = dm ? "chat-direct-message-channel-menu" : "chat-channel-menu";
        var instance = Discourse.float.menu(trigger, {
          identifier: identifier,
          content: html,
          placement: "right-start",
          modalForMobile: true,
        });
        instance.content.addEventListener("click", function (event) {
          var button = event.target.closest("[data-menu-option-id]");
          if (!button) {
            return;
          }
          var action = button.dataset.menuOptionId;
          if (action === "notifications") {
            notificationMenu(button, c, function () {
              Discourse.float.close(identifier);
            });
            return;
          }
          Discourse.float.close(identifier);
          if (action === "settings") {
            go("/chat/c/" + (c.slug || "-") + "/" + c.id + "/info/settings");
          } else if (action === "star") {
            request("PUT", "/chat/api/channels/" + c.id + "/memberships/me", { starred: !m.starred })
              .then(reload)
              .catch(popupError);
          } else if (action === "leave") {
            var path = dm ? "/memberships/me/follows" : "/memberships/me";
            request("DELETE", "/chat/api/channels/" + c.id + path)
              .then(function () {
                go("/chat");
              })
              .catch(popupError);
          }
        });
      })
      .catch(popupError);
  }

  // ChatChannelSidebarContextNotificationSubmenu
  function notificationMenu(trigger, c, closeParent) {
    var m = c.current_user_membership || {};
    var selected = m.muted ? "muted" : m.notification_level;
    var level = function (v) {
      return item(
        menuButton(v, s["level_" + v], {
          cls: "chat-channel-sidebar-link-menu__notification-level-" + v + (selected === v ? " -selected" : ""),
        })
      );
    };
    var html =
      '<ul class="dropdown-menu">' +
      level("never") +
      level("mention") +
      level("always") +
      '<li><hr class="dropdown-menu__divider"></li>' +
      item(
        menuButton("muted", m.muted ? s.unmute : s.mute, {
          icon: m.muted ? "bell-slash" : "bell",
          cls: "chat-channel-sidebar-link-menu__mute-channel" + (selected === "muted" ? " -selected" : ""),
        })
      ) +
      "</ul>";
    var identifier = "chat-channel-menu-notification-submenu";
    var instance = Discourse.float.menu(trigger, {
      identifier: identifier,
      content: html,
      placement: "right-start",
      modalForMobile: true,
      onClose: closeParent,
    });
    instance.content.addEventListener("click", function (event) {
      var button = event.target.closest("[data-menu-option-id]");
      if (!button) {
        return;
      }
      var value = button.dataset.menuOptionId;
      var settings = value === "muted" ? { muted: !m.muted } : { notification_level: value };
      Discourse.float.close(identifier);
      request("PUT", "/chat/api/channels/" + c.id + "/notifications-settings/me", {
        notifications_settings: settings,
      })
        .then(reload)
        .catch(popupError);
    });
  }

  function slugify(name, done) {
    if (!name) {
      done("");
      return;
    }
    request("POST", "/slugs.json", { name: name })
      .then(function (body) {
        done(body.slug || "");
      })
      .catch(function () {});
  }

  // FKControlInput in its field.
  function field(name, title, control, opts) {
    opts = opts || {};
    return (
      '<div class="form-kit__container form-kit__field form-kit__field-' +
      control +
      (opts.format ? " --" + opts.format : "") +
      '" data-name="' +
      name +
      '" id="control-' +
      name +
      '" data-control-type="' +
      control +
      '">' +
      (control === "checkbox"
        ? ""
        : '<label class="form-kit__container-title" for="control-' +
          name +
          '-input"><span>' +
          escapeHtml(title) +
          "</span>" +
          (opts.tooltip
            ? '<span class="fk-d-tooltip__trigger form-kit__tooltip" data-tooltip="' +
              escapeHtml(opts.tooltip) +
              '"><span class="fk-d-tooltip__trigger-container"><span class="fk-d-tooltip__icon">' +
              icon("circle-question") +
              "</span></span></span>"
            : "") +
          "</label>") +
      '<div class="form-kit__container-content">' +
      opts.inner +
      '<p class="form-kit-errors" hidden></p></div></div>'
    );
  }

  function input(name) {
    return (
      '<div class="form-kit__control-input-wrapper"><input class="form-kit__control-input" id="control-' +
      name +
      '-input" name="' +
      name +
      '" type="text" value=""></div>'
    );
  }

  function checkbox(name, title, description) {
    return (
      '<label class="form-kit__control-checkbox-label" for="control-' +
      name +
      '-input"><input class="form-kit__control-checkbox" id="control-' +
      name +
      '-input" name="' +
      name +
      '" type="checkbox"><span class="form-kit__control-checkbox-checkmark">' +
      icon("check") +
      '</span><span class="form-kit__control-checkbox-content"><span class="form-kit__control-checkbox-title"><span>' +
      escapeHtml(title) +
      '</span></span><span class="form-kit__control-checkbox-description">' +
      escapeHtml(description) +
      "</span></span></label>"
    );
  }

  function categoryLabel(c, byId) {
    var parent = c.parent_category_id && byId[c.parent_category_id];
    return parent ? parent.name + " / " + c.name : c.name;
  }

  // ChatModalCreateChannel
  function createChannel() {
    if (!Discourse.modal) {
      return;
    }
    var state = { category: null, autoSlug: "", warning: "", emoji: null, timer: null };
    var categories = [];
    var bodyEl;

    function hint(permissions) {
      if (!permissions) {
        return s.default_hint;
      }
      var groups = permissions.allowed_groups;
      var link = basePath() + "/c/" + escapeHtml(state.category.fullSlug) + "/edit/security";
      if (groups.length === 1) {
        return s.hint_1_group.replace("%{settingLink}", link).replace("%{group}", escapeHtml(groups[0]));
      }
      if (groups.length === 2) {
        return s.hint_2_groups
          .replace("%{settingLink}", link)
          .replace("%{group1}", escapeHtml(groups[0]))
          .replace("%{group2}", escapeHtml(groups[1]));
      }
      var count = groups.length - 1;
      return (count === 1 ? s.hint_multiple_one : s.hint_multiple_other)
        .replace("%{settingLink}", link)
        .replace("%{group}", escapeHtml(groups[0]))
        .replace("%{count}", count);
    }

    // #updateAutoJoinConfirmWarning
    function warning(permissions) {
      var groups = permissions.allowed_groups;
      var count = permissions.members_count;
      var plural = function (key) {
        return count === 1 ? s[key + "_one"] : s[key + "_other"];
      };
      if (!permissions.private) {
        return s.public_warning.replace("%{category}", escapeHtml(state.category.name));
      }
      if (groups.length === 1) {
        return plural("warning_1_group").replace("%{count}", count).replace("%{group}", escapeHtml(groups[0]));
      }
      if (groups.length === 2) {
        return plural("warning_2_groups")
          .replace("%{count}", count)
          .replace("%{group1}", escapeHtml(groups[0]))
          .replace("%{group2}", escapeHtml(groups[1]));
      }
      var others = groups.length - 1;
      return s.warning_multiple
        .replace("%{groupName}", escapeHtml(groups[0]))
        .replace("%{groupCount}", others)
        .replace("%{userCount}", count)
        .replace("%{groups}", others === 1 ? s.group_one : s.group_other)
        .replace("%{users}", count === 1 ? s.user_one : s.user_other);
    }

    function setSlugPlaceholder() {
      bodyEl.querySelector("[name='slug']").placeholder = state.autoSlug;
    }

    function generateSlug(name) {
      clearTimeout(state.timer);
      state.autoSlug = "";
      setSlugPlaceholder();
      if (!name) {
        return;
      }
      state.timer = setTimeout(function () {
        slugify(name, function (slug) {
          state.autoSlug = slug;
          setSlugPlaceholder();
        });
      }, 300);
    }

    function updateAutoJoinInfo() {
      var info = bodyEl.querySelector("[data-name='auto_join_users'] .form-kit__control-checkbox-description");
      if (info) {
        info.textContent = state.category
          ? s.auto_join_info.replace("%{category}", state.category.name)
          : s.auto_join_info_no_category;
      }
    }

    function onCategory(id) {
      state.category = categories.filter(function (c) {
        return String(c.id) === String(id);
      })[0] || null;
      var hintEl = bodyEl.querySelector(".chat-modal-create-channel__hint");
      var nameInput = bodyEl.querySelector("[name='name']");
      if (!nameInput.value && state.category) {
        nameInput.value = state.category.name;
      }
      generateSlug(nameInput.value);
      updateAutoJoinInfo();
      if (!state.category) {
        hintEl.innerHTML = s.default_hint;
        state.warning = "";
        return;
      }
      hintEl.classList.add("loading-permissions");
      request("GET", "/chat/api/category-chatables/" + state.category.id + "/permissions")
        .then(function (permissions) {
          state.warning = warning(permissions);
          hintEl.innerHTML = hint(permissions);
        })
        .catch(function () {})
        .finally(function () {
          hintEl.classList.remove("loading-permissions");
        });
    }

    function refreshEmoji() {
      var button = bodyEl.querySelector(".chat-modal-create-channel__emoji");
      var tpl = document.querySelector("template.emoji-picker-template");
      var url = (tpl && tpl.dataset.emojiUrl) || basePath() + "/images/emoji/twitter/%{name}.png";
      button.innerHTML = state.emoji
        ? '<img width="20" height="20" class="emoji" src="' +
          escapeHtml(url.replace("%{name}", encodeURIComponent(state.emoji))) +
          '" alt="' +
          escapeHtml(state.emoji) +
          '">'
        : icon("discourse-emojis");
      bodyEl.querySelector(".edit-channel-clear-emoji").disabled = !state.emoji;
    }

    function showErrors(errors) {
      Object.keys(errors).forEach(function (name) {
        var el = bodyEl.querySelector("[data-name='" + name + "'] .form-kit-errors");
        if (el) {
          el.textContent = errors[name] || "";
          el.hidden = !errors[name];
          el.closest(".form-kit__field").classList.toggle("has-error", !!errors[name]);
        }
      });
    }

    function flash(text) {
      var existing = bodyEl.parentNode.querySelector(".d-modal__flash");
      if (existing) {
        existing.remove();
      }
      if (!text) {
        return;
      }
      var el = document.createElement("div");
      el.className = "alert d-modal__flash alert-error";
      el.setAttribute("role", "alert");
      el.innerHTML = '<span class="d-modal__flash-message">' + escapeHtml(text) + "</span>";
      bodyEl.parentNode.insertBefore(el, bodyEl);
    }

    function submit(close) {
      var form = bodyEl.querySelector("form");
      var name = form.querySelector("[name='name']").value;
      var categoryId = form.querySelector("[name='chatable_id']").value;
      var errors = {
        name: name ? "" : s.required,
        chatable_id: categoryId ? "" : s.required,
      };
      showErrors(errors);
      if (errors.name || errors.chatable_id) {
        return;
      }
      var autoJoin = form.querySelector("[name='auto_join_users']");
      var payload = {
        name: name,
        slug: form.querySelector("[name='slug']").value || state.autoSlug,
        description: form.querySelector("[name='description']").value,
        chatable_id: categoryId,
        auto_join_users: !!(autoJoin && autoJoin.checked),
        threading_enabled: form.querySelector("[name='threading_enabled']").checked,
        emoji: state.emoji,
      };
      var create = function () {
        request("POST", "/chat/api/channels", { channel: payload })
          .then(function (body) {
            close();
            go("/chat/c/" + (body.channel.slug || "-") + "/" + body.channel.id);
          })
          .catch(function (error) {
            flash(error.message);
          });
      };
      if (payload.auto_join_users && Discourse.dialog) {
        Discourse.dialog.yesNoConfirm({ message: state.warning, didConfirm: create });
      } else {
        create();
      }
    }

    var close = Discourse.modal.show({
      className: "chat-modal-create-channel",
      title: s.title,
      body: function (body) {
        bodyEl = body;
        var autoJoin = cfg.autoJoinAvailable
          ? field("auto_join_users", s.auto_join_label, "checkbox", {
              format: "full",
              inner: checkbox("auto_join_users", s.auto_join_label, s.auto_join_info_no_category),
            })
          : "";
        body.innerHTML =
          '<form class="form-kit chat-modal-create-channel__form" novalidate="">' +
          field("name", s.name, "input", { format: "full", inner: input("name") }) +
          field("slug", s.slug, "input", { format: "full", tooltip: s.slug_description, inner: input("slug") }) +
          field("description", s.description, "input", { format: "full", inner: input("description") }) +
          field("chatable_id", s.category, "custom", {
            format: "full",
            inner:
              '<select class="d-native-select category-chooser" name="chatable_id"><option value="">' +
              escapeHtml(s.category_none) +
              '</option></select><div class="chat-modal-create-channel__hint">' +
              s.default_hint +
              "</div>",
          }) +
          field("emoji", s.emoji, "emoji", {
            inner:
              '<button class="btn no-text btn-icon btn-default btn-emoji chat-modal-create-channel__emoji" type="button"></button><button class="btn btn-text btn-flat edit-channel-clear-emoji" type="button"><span class="d-button-label">' +
              escapeHtml(s.emoji_reset) +
              "</span></button>",
          }) +
          autoJoin +
          field("threading_enabled", s.threading, "checkbox", {
            format: "full",
            inner: checkbox("threading_enabled", s.threading, s.threading_description),
          }) +
          "</form>";
        var select = body.querySelector("[name='chatable_id']");
        select.addEventListener("change", function () {
          onCategory(select.value);
        });
        body.querySelector("[name='name']").addEventListener("input", function (event) {
          generateSlug(event.target.value);
        });
        body.querySelector(".chat-modal-create-channel__emoji").addEventListener("click", function (event) {
          if (Discourse.emojiPicker) {
            Discourse.emojiPicker.open(event.currentTarget, {
              context: "channel-emoji",
              onSelect: function (emoji) {
                state.emoji = emoji;
                refreshEmoji();
              },
            });
          }
        });
        body.querySelector(".edit-channel-clear-emoji").addEventListener("click", function () {
          state.emoji = null;
          refreshEmoji();
        });
        body.querySelector("form").addEventListener("submit", function (event) {
          event.preventDefault();
        });
        refreshEmoji();
        // The site's categories, as CategoryChooser lists them.
        request("GET", "/site.json")
          .then(function (site) {
            var byId = {};
            (site.categories || []).forEach(function (c) {
              byId[c.id] = c;
            });
            categories = (site.categories || []).map(function (c) {
              var parent = c.parent_category_id && byId[c.parent_category_id];
              return {
                id: c.id,
                name: c.name,
                fullSlug: parent ? parent.slug + "/" + c.slug : c.slug,
                label: categoryLabel(c, byId),
              };
            });
            categories.forEach(function (c) {
              var option = document.createElement("option");
              option.value = c.id;
              option.textContent = c.label;
              select.appendChild(option);
            });
          })
          .catch(function () {});
      },
      footer: function (footer, closeModal) {
        footer.innerHTML =
          '<button class="btn btn-text btn-primary create" type="button"><span class="d-button-label">' +
          escapeHtml(s.create_button) +
          "</span></button>";
        footer.querySelector(".create").addEventListener("click", function () {
          submit(closeModal);
        });
      },
    });
    return close;
  }

  document.addEventListener("click", function (event) {
    if (!window.Discourse || !Discourse.float) {
      return;
    }
    var target = event.target;
    var trigger;
    if ((trigger = target.closest(".chat-channel-list-options-button, [data-sidebar-action-id='channelListOptions']"))) {
      event.preventDefault();
      optionsMenu(trigger);
    } else if (
      (trigger = target.closest(
        ".chat-channel-list-filter-toggle, [data-sidebar-action-id='toggleChannelFilter'], .--empty-channels-list .empty-state__cta button:not([data-href])"
      ))
    ) {
      event.preventDefault();
      savePreference(sectionOf(trigger), "filter", "all");
    } else if ((trigger = target.closest(".c-navbar__new-channel-button"))) {
      event.preventDefault();
      createChannel();
    } else if (
      (trigger = target.closest(
        "[data-section-name^='chat-'] .sidebar-section-hover-button, [data-section-name^='chat-'] .sidebar-section-link-hover button"
      ))
    ) {
      event.preventDefault();
      event.stopPropagation();
      contextMenu(trigger);
    }
  });

  Discourse.chatChannelMenus = { createChannel: createChannel };
})();
