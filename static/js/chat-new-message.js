// Chat's new message modal (ChatModalNewMessage: DModal --quick-palette
// over MessageCreator) and the ways in: the DMs list's "Start a
// conversation" (drawer and full page), the browse page's empty state, a
// link to /chat/new-message followed in the app (the sidebar's "Start DM
// or group chat"), and Ctrl/Cmd+K. The modal's search mode lists the
// member's channels (ChatablesLoader's preloaded channels) and searches
// GET /chat/api/chatables as typed; a user opens the direct message with
// them (chat.upsertDmChannel, POST /chat/api/direct-message-channels), a
// group or "New group chat" goes to the new group mode (NewGroup with
// MembersSelector), which creates a group direct message. The profile's
// and the user card's Chat buttons (ChatDirectMessageButton) open the
// direct message with their user directly, as /chat/new-message with
// recipients (ChatNewMessageRoute) does with them. A channel opens through
// chat-drawer.js, in the drawer when it is preferred.
(function () {
  "use strict";

  if (!Discourse.once("chat-new-message")) {
    return;
  }

  var script = document.currentScript;
  var cfg = {};
  try {
    cfg = JSON.parse((script && script.dataset.config) || "{}");
  } catch (_) {}
  var s = cfg.strings || {};
  var MAX_RESULTS = 10;
  // discourse/lib/environment's INPUT_DELAY.
  var INPUT_DELAY = 250;
  var MODES = { search: "SEARCH", new_group: "NEW_GROUP" };

  function basePath() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function headers() {
    var h = {};
    try {
      h = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (_) {}
    h["X-Requested-With"] = "XMLHttpRequest";
    h.Accept = "application/json";
    return h;
  }

  function escapeHtml(v) {
    return String(v == null ? "" : v)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // iconHTML, with the icon library's d-chat.
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

  function noTextButton(classes, iconName) {
    return (
      '<button class="btn no-text btn-icon ' +
      classes +
      '" type="button">' +
      icon(iconName) +
      '<span aria-hidden="true">&#8203;</span></button>'
    );
  }

  // jQuery.param as ajax sends data: null as empty, undefined left out,
  // arrays as `key[]`, empty ones left out.
  function param(data) {
    var pairs = [];
    Object.keys(data).forEach(function (key) {
      var value = data[key];
      if (value === undefined) {
        return;
      }
      if (Array.isArray(value)) {
        value.forEach(function (item) {
          pairs.push(encodeURIComponent(key + "[]") + "=" + encodeURIComponent(item));
        });
        return;
      }
      pairs.push(encodeURIComponent(key) + "=" + encodeURIComponent(value === null ? "" : value).replace(/%20/g, "+"));
    });
    return pairs.join("&");
  }

  function getJson(path, data) {
    var query = data ? param(data) : "";
    return fetch(basePath() + path + (query ? "?" + query : ""), {
      credentials: "same-origin",
      headers: headers(),
    }).then(function (r) {
      return r.json().then(function (json) {
        if (!r.ok) {
          throw json;
        }
        return json;
      });
    });
  }

  // popupAjaxError: the server's error in a dialog.
  function popupAjaxError(error) {
    var message =
      (error && error.errors && error.errors.join("\n")) ||
      (error && error.error) ||
      (window.I18n && I18n.t ? I18n.t("generic_error") : "Sorry, an error has occurred.");
    if (window.Discourse && Discourse.dialog && Discourse.dialog.alert) {
      Discourse.dialog.alert(message);
    } else {
      window.alert(message);
    }
  }

  // chat.createDmChannel: the channel as the server serialized it.
  function createDmChannel(targets, opts) {
    var h = headers();
    h["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
    return fetch(basePath() + "/chat/api/direct-message-channels.json", {
      method: "POST",
      credentials: "same-origin",
      headers: h,
      body: param({
        target_usernames: targets.usernames ? unique(targets.usernames) : null,
        target_groups: targets.groups ? unique(targets.groups) : null,
        upsert: opts.upsert,
        name: opts.name,
      }),
    }).then(function (r) {
      return r
        .json()
        .catch(function () {
          return {};
        })
        .then(function (json) {
          if (!r.ok) {
            throw json;
          }
          return json.channel;
        });
    });
  }

  function upsertDmChannel(targets) {
    return createDmChannel(targets, { name: null, upsert: true });
  }

  function unique(items) {
    return items.filter(function (item, i) {
      return items.indexOf(item) === i;
    });
  }

  // utilities' slugify and chat's slugifyChannel.
  function slugify(string) {
    return string
      .trim()
      .toLowerCase()
      .replace(/\s|_+/g, "-")
      .replace(/[^\w\-]+/g, "")
      .replace(/\-\-+/g, "-")
      .replace(/^-+/, "")
      .replace(/-+$/, "");
  }

  function slugifyChannel(channel) {
    if (channel.slug) {
      return channel.slug;
    }
    if (!channel.title) {
      return "-";
    }
    var slug = slugify(escapeHtml(channel.title));
    var resolved = (slug.length ? slug : channel.title.trim().toLowerCase().replace(/\s|_+/g, "-")).slice(0, 100);
    return resolved || "-";
  }

  // router.transitionTo("chat.channel", ...channel.routeModels)
  function openChannel(channel) {
    var url = "/chat/c/" + slugifyChannel(channel) + "/" + channel.id;
    if (window.Discourse && Discourse.chatDrawer && Discourse.chatDrawer.transitionTo) {
      Discourse.chatDrawer.transitionTo(url);
    } else {
      window.location.assign(basePath() + url);
    }
  }

  // ChatChatable
  function chatable(args) {
    var c = {
      identifier: args.identifier,
      type: args.type,
      model: args.model,
      enabled: true,
      matchQuality: args.match_quality == null ? 2 : args.match_quality,
    };
    if (c.type === "channel") {
      var users = (c.model.chatable && c.model.chatable.users) || [];
      if (users.length === 1) {
        c.enabled = users[0].has_chat_enabled;
      }
    } else if (c.type === "user") {
      c.enabled = c.model.has_chat_enabled;
    } else if (c.type === "group") {
      c.enabled = c.model.can_chat;
    }
    return c;
  }

  function isDirectMessage(channel) {
    return channel.chatable_type === "DirectMessage";
  }

  // The member's channels (chatChannelsManager), from /chat/api/me/channels:
  // each with its tracking.
  function myChannels() {
    return getJson("/chat/api/me/channels").then(function (json) {
      var tracking = (json.tracking && json.tracking.channel_tracking) || {};
      var all = (json.public_channels || []).concat(json.direct_message_channels || []);
      all.forEach(function (channel) {
        var t = tracking[channel.id] || {};
        channel.tracking = {
          unreadCount: t.unread_count || 0,
          mentionCount: t.mention_count || 0,
          watchedThreadsUnreadCount: t.watched_threads_unread_count || 0,
        };
      });
      return all;
    });
  }

  function following(channel) {
    return !!(channel.current_user_membership && channel.current_user_membership.following);
  }

  function starred(channel) {
    return !!(channel.current_user_membership && channel.current_user_membership.starred);
  }

  // #withStarredPriority
  function withStarred(property, compare) {
    return function (a, b) {
      if (starred(a) && starred(b)) {
        return (a[property] || "").localeCompare(b[property] || "");
      }
      if (starred(a) || starred(b)) {
        return starred(a) ? -1 : 1;
      }
      return compare(a, b);
    };
  }

  function urgent(channel) {
    var t = channel.tracking;
    return t.unreadCount + t.mentionCount + t.watchedThreadsUnreadCount;
  }

  function lastMessageAt(channel) {
    return new Date((channel.last_message || {}).created_at);
  }

  // #compareDirectMessageChannelsByActivity (no unread threads: threads
  // aren't ported).
  function compareDms(a, b) {
    if (!(a.last_message || {}).id) {
      return 1;
    }
    if (!(b.last_message || {}).id) {
      return -1;
    }
    var au = urgent(a);
    var bu = urgent(b);
    if (au > 0 && bu > 0) {
      return lastMessageAt(a) > lastMessageAt(b) ? -1 : 1;
    }
    if (au > 0 || bu > 0) {
      return au > bu ? -1 : 1;
    }
    return lastMessageAt(a) > lastMessageAt(b) ? -1 : 1;
  }

  // chatChannelsManager.allChannels: the followed public channels by slug,
  // then the followed DMs by activity, starred ones first in each. (Its
  // sort by lastViewedAt compares Dates with localeCompare, which they
  // don't have: the order stays.)
  function allChannels(channels) {
    var publics = channels
      .filter(function (c) {
        return !isDirectMessage(c) && following(c);
      })
      .sort(
        withStarred("slug", function (a, b) {
          return (a.slug || "").localeCompare(b.slug || "");
        })
      );
    var dms = channels
      .filter(function (c) {
        return isDirectMessage(c) && following(c);
      })
      .sort(withStarred("title", compareDms));
    return publics.concat(dms);
  }

  // ChatablesLoader#search: the preloaded channels without a term, else
  // /chat/api/chatables, sorted (sortChatables) and cut to ten.
  function search(context, term, options) {
    if (!term && options.preloadChannels) {
      return Promise.resolve(
        allChannels(context.channels)
          .map(function (channel) {
            var users = (channel.chatable && channel.chatable.users) || [];
            var c =
              users.length === 1
                ? chatable({ type: "user", model: users[0], identifier: "u-" + users[0].id })
                : chatable({ type: "channel", model: channel, identifier: "c-" + channel.id });
            c.tracking = channel.tracking;
            c.unreadThreadCount = 0;
            return c;
          })
          .slice(0, MAX_RESULTS)
      );
    }
    if (!term) {
      return Promise.resolve([]);
    }
    return getJson("/chat/api/chatables", {
      term: term,
      include_users: options.includeUsers,
      include_category_channels: options.includeCategoryChannels,
      include_direct_message_channels: options.includeDirectMessageChannels,
      excluded_memberships_channel_id: options.excludedMembershipsChannelId,
    }).then(function (results) {
      var items = []
        .concat(results.users || [], results.groups || [], results.direct_message_channels || [], results.category_channels || [])
        .map(function (item) {
          var c = chatable(item);
          var channel = findChannel(context.channels, c);
          if (channel) {
            c.tracking = channel.tracking;
            c.unreadThreadCount = 0;
          }
          return c;
        });
      return sortChatables(items).slice(0, MAX_RESULTS);
    });
  }

  function findChannel(channels, c) {
    if (c.type === "user") {
      return channels.find(function (channel) {
        var users = (channel.chatable && channel.chatable.users) || [];
        return users.length === 1 && users[0].id === c.model.id;
      });
    }
    if (c.type === "channel") {
      return channels.find(function (channel) {
        return channel.id === c.model.id;
      });
    }
  }

  function typePriority(c) {
    if (c.type === "user") {
      return 0;
    }
    if (c.type === "group") {
      return 3;
    }
    return isDirectMessage(c.model) ? 1 : 2;
  }

  function sortChatables(items) {
    return items.sort(function (a, b) {
      if (a.matchQuality !== b.matchQuality) {
        return a.matchQuality - b.matchQuality;
      }
      if (typePriority(a) !== typePriority(b)) {
        return typePriority(a) - typePriority(b);
      }
      if (a.enabled !== b.enabled) {
        return a.enabled ? -1 : 1;
      }
      return 0;
    });
  }

  // ChatUserAvatar, not interactive (renderAvatar at the tiny size).
  function avatar(user) {
    var src = String(user.avatar_template || "").replace("{size}", "24");
    var root = basePath();
    if (root && src.charAt(0) === "/" && src.charAt(1) !== "/" && src.indexOf(root) !== 0) {
      src = root + src;
    }
    return (
      '<div class="chat-user-avatar" data-username="' +
      escapeHtml(user.username) +
      '"><span class="chat-user-avatar__container"><img alt="" width="24" height="24" src="' +
      escapeHtml(src) +
      '" class="avatar" title="' +
      escapeHtml(user.username) +
      '"></span></div>'
    );
  }

  // ChatUserDisplayName
  function displayName(user) {
    var hasName = !!(user.name && user.name.trim().length);
    var nameFirst = !cfg.prioritizeUsername && hasName;
    var nameLast = cfg.prioritizeUsername && hasName;
    return (
      '<span class="chat-user-display-name">' +
      (nameFirst ? '<span class="chat-user-display-name__name -first">' + escapeHtml(user.name) + "</span>" : "") +
      '<span class="chat-user-display-name__username' +
      (nameFirst ? "" : " -first") +
      '">' +
      escapeHtml(user.username) +
      "</span>" +
      (nameLast ? '<span class="chat-user-display-name__name">' + escapeHtml(user.name) + "</span>" : "") +
      "</span>"
    );
  }

  function emoji(name) {
    var code = String(name).toLowerCase();
    var custom = cfg.customEmoji || {};
    var src = custom[code] || String(cfg.emojiUrl || "").replace("%{name}", code.replace(":t", "/"));
    return (
      '<img width="20" height="20" src="' +
      escapeHtml(src) +
      '" title="' +
      escapeHtml(code) +
      '" alt="' +
      escapeHtml(code) +
      '" class="emoji">'
    );
  }

  // ChatChannelIcon
  function channelIcon(channel) {
    var chatable = channel.chatable || {};
    var users = chatable.users || [];
    if (isDirectMessage(channel)) {
      if (chatable.group) {
        if (channel.emoji) {
          return '<div class="chat-channel-icon --emoji">' + emoji(channel.emoji) + "</div>";
        }
        if (users.length === 1) {
          return '<div class="chat-channel-icon --avatar">' + avatar(users[0]) + "</div>";
        }
        return '<div class="chat-channel-icon --users-count">' + escapeHtml(channel.memberships_count) + "</div>";
      }
      return '<div class="chat-channel-icon --avatar">' + (users[0] ? avatar(users[0]) : "") + "</div>";
    }
    return (
      '<div class="chat-channel-icon --icon" style="color: #' +
      escapeHtml(chatable.color) +
      '">' +
      (channel.emoji ? emoji(channel.emoji) : icon("d-chat")) +
      (chatable.read_restricted ? icon("lock", "chat-channel-icon__restricted-category-icon") : "") +
      "</div>"
    );
  }

  // ChatChannelName: a DM's title, else the member's own name for a DM
  // with no one else.
  function channelName(channel) {
    var title = channel.unicode_title != null ? channel.unicode_title : channel.title;
    if (isDirectMessage(channel) && title == null) {
      var users = (channel.chatable && channel.chatable.users) || [];
      title = users.length
        ? users
            .map(function (u) {
              return cfg.prefersName ? u.name || u.username : u.username;
            })
            .join(", ")
        : cfg.username;
    }
    return '<div class="chat-channel-name"><div class="chat-channel-name__label">' + escapeHtml(title) + "</div></div>";
  }

  // ChatChannelTitle
  function channelTitle(channel, isUnread, isUrgent) {
    return (
      '<div class="chat-channel-title ' +
      (isDirectMessage(channel) ? "is-dm" : "is-category") +
      '">' +
      channelIcon(channel) +
      channelName(channel) +
      (isUnread ? '<div class="unread-indicator ' + (isUrgent ? "-urgent" : "") + '"></div>' : "") +
      "</div>"
    );
  }

  function disabledAttr(disabled) {
    return disabled ? ' data-disabled="true"' : "";
  }

  // The list's items: ListAction, ChatableUser, ChatableGroup, Channel.
  function itemHtml(item, membersCount) {
    if (item.type === "list-action") {
      return (
        '<div class="chat-message-creator__chatable -group"><div class="chat-message-creator__group-icon">' +
        icon(item.icon) +
        '</div><div class="chat-message-creator__group-name">' +
        escapeHtml(item.label) +
        "</div></div>"
      );
    }
    var t = item.tracking || {};
    if (item.type === "user") {
      var userUrgent = t.unreadCount > 0 || t.mentionCount > 0 || t.watchedThreadsUnreadCount > 0;
      var userUnread = item.unreadThreadCount > 0;
      return (
        '<div class="chat-message-creator__chatable -user"' +
        disabledAttr(!item.enabled) +
        ">" +
        avatar(item.model) +
        displayName(item.model) +
        (userUrgent || userUnread ? '<div class="unread-indicator' + (userUrgent ? " -urgent" : "") + '"></div>' : "") +
        '<div class="user-status"></div>' +
        (item.enabled ? "" : '<span class="chat-message-creator__disabled-warning">' + escapeHtml(s.disabledUser) + "</span>") +
        "</div>"
      );
    }
    if (item.type === "group") {
      var disabled = membersCount
        ? membersCount + item.model.chat_enabled_user_count > cfg.maxDmUsers
        : !item.enabled;
      return (
        '<div class="chat-message-creator__chatable -group"' +
        disabledAttr(disabled) +
        '><div class="chat-message-creator__group-icon">' +
        icon("user-group") +
        '</div><div class="chat-message-creator__group-name">' +
        escapeHtml(item.model.name) +
        "</div>" +
        (disabled
          ? '<span class="chat-message-creator__disabled-warning">' +
            escapeHtml(String(s.groupTooMany).replace("%{membersCount}", item.model.chat_enabled_user_count)) +
            "</span>"
          : "") +
        "</div>"
      );
    }
    var hasUnreads = t.unreadCount > 0;
    var hasUrgent = t.mentionCount > 0 || t.watchedThreadsUnreadCount > 0;
    var isUrgent = isDirectMessage(item.model) ? hasUnreads || hasUrgent : hasUrgent;
    return (
      '<div class="chat-message-creator__chatable -category-channel"' +
      disabledAttr(!item.enabled) +
      ">" +
      channelTitle(item.model, hasUnreads || item.unreadThreadCount > 0 || hasUrgent, isUrgent) +
      "</div>"
    );
  }

  // List: the items, or the warning once the group is full.
  function listHtml(items, highlighted, maxReached, membersCount) {
    if (!items.length) {
      return "";
    }
    if (maxReached) {
      return (
        '<div class="chat-message-creator__list-container"><div class="chat-message-creator__warning-max-members">' +
        escapeHtml(s.cantAddMore) +
        "</div></div>"
      );
    }
    return (
      '<div class="chat-message-creator__list-container"><ul class="chat-message-creator__list">' +
      items
        .map(function (item) {
          return (
            '<li class="chat-message-creator__list-item' +
            (highlighted && item.identifier === highlighted.identifier ? " -highlighted" : "") +
            '" data-identifier="' +
            escapeHtml(item.identifier) +
            '"' +
            (item.id ? ' id="' + escapeHtml(item.id) + '"' : "") +
            ' role="button" tabindex="0">' +
            itemHtml(item, membersCount) +
            "</li>"
          );
        })
        .join("") +
      "</ul></div>"
    );
  }

  // iterate-list's getNext and getPrevious, over the enabled items.
  function step(list, current, forward) {
    if (!list.length) {
      return null;
    }
    list = list.filter(function (item) {
      return item.enabled;
    });
    if (!list.length) {
      return undefined;
    }
    var index = current && current.identifier
      ? list.findIndex(function (item) {
          return item.identifier === current.identifier;
        })
      : -1;
    if (index === -1) {
      return forward ? list[0] : list[list.length - 1];
    }
    if (forward) {
      return index < list.length - 1 ? list[index + 1] : list[0];
    }
    return index > 0 ? list[index - 1] : list[list.length - 1];
  }

  // MessageCreator in the modal: its mode, its members, and the search
  // or the new group drawn into the creator.
  function showModal(initialMode) {
    // ChatModalNewMessage#shouldRender
    if (!cfg.publicChannels && !cfg.canDirectMessage) {
      return;
    }
    if (!window.Discourse || !Discourse.modal) {
      return;
    }
    var context = { channels: [] };
    var state = {
      mode: initialMode || MODES.search,
      members: [],
      term: "",
      chatables: [],
      highlighted: null,
      // MembersSelector's
      filter: "",
      highlightedMember: null,
      newGroupTitle: "",
    };
    var root;
    var closeModal;
    var searchTimer = null;
    var seq = 0;

    function items() {
      if (state.mode === MODES.search) {
        var list = [];
        if (!state.term && cfg.canGroupChat) {
          list.push({
            identifier: "new-group",
            type: "list-action",
            label: s.newGroupChat,
            enabled: true,
            icon: "user-group",
            id: "new-group-chat",
          });
        }
        return list.concat(state.chatables);
      }
      var selected = state.members.map(function (m) {
        return m.identifier;
      });
      return state.chatables.filter(function (c) {
        return selected.indexOf(c.identifier) === -1;
      });
    }

    function membersCount() {
      return state.members.reduce(function (acc, member) {
        return acc + (member.type === "group" ? member.model.chat_enabled_user_count : 1);
      }, 0);
    }

    function searchModeHtml() {
      return (
        '<span style="display: contents"><div class="chat-message-creator__search-container"><div class="chat-message-creator__search"><div class="chat-message-creator__section"><div class="chat-message-creator__search-input-container"><div class="chat-message-creator__search-input">' +
        icon("magnifying-glass", "chat-message-creator__search-input__search-icon") +
        '<input class="ember-text-field chat-message-creator__search-input__input" placeholder="' +
        escapeHtml(s.filter) +
        '" type="text"></div></div>' +
        noTextButton("btn-flat chat-message-creator__search-input__cancel-button", "xmark") +
        "</div>" +
        listHtml(items(), state.highlighted, false, 0) +
        "</div></div></span>"
      );
    }

    function membersHtml() {
      return state.members
        .map(function (member) {
          var inner =
            member.type === "user"
              ? avatar(member.model) +
                '<span class="chat-message-creator__member-username">' +
                escapeHtml(member.model.username) +
                "</span>"
              : '<div class="chat-message-creator__group-icon">' +
                icon("user-group") +
                '</div><span class="chat-message-creator__member-group">' +
                escapeHtml(member.model.name) +
                "</span>";
          return (
            '<button class="btn no-text chat-message-creator__member btn-default btn-small' +
            (member === state.highlightedMember ? " -highlighted" : "") +
            '" type="button">' +
            inner +
            icon("xmark") +
            "</button>"
          );
        })
        .join("");
    }

    function countHtml() {
      var count = membersCount();
      var label = (count === 1 ? s.membersOne : s.membersOther) || "";
      return (
        '<div class="chat-message-creator__members-count' +
        (count >= cfg.maxDmUsers ? " -reached-limit" : "") +
        '">' +
        escapeHtml(label.replace("%{count}", count).replace("%{max}", cfg.maxDmUsers)) +
        "</div>"
      );
    }

    function footerHtml() {
      if (!state.members.length) {
        return "";
      }
      return (
        '<div class="chat-message-creator__new-group-footer-container"><div class="chat-message-creator__new-group-footer"><button class="btn btn-flat" type="button"><span class="d-button-label">' +
        escapeHtml(s.cancel) +
        '</span></button><button class="btn btn-primary create-chat-group" type="button"><span class="d-button-label">' +
        escapeHtml(s.createGroup) +
        "</span></button></div></div>"
      );
    }

    function newGroupHtml() {
      return (
        '<div class="chat-message-creator__new-group-container"><div class="chat-message-creator__new-group"><div class="chat-message-creator__new-group-header-container"><div class="chat-message-creator__new-group-header"><input class="ember-text-field chat-message-creator__new-group-header__input" name="channel-name" placeholder="' +
        escapeHtml(s.groupName) +
        '" type="text">' +
        countHtml() +
        '</div></div><span style="display: contents"><div class="chat-message-creator__add-members-header-container"><div class="chat-message-creator__add-members-header"><div class="chat-message-creator__members-container"><div class="chat-message-creator__members">' +
        icon("magnifying-glass") +
        membersHtml() +
        '<input autofocus class="ember-text-field chat-message-creator__members-input" placeholder="' +
        escapeHtml(s.addMembers) +
        '" type="text"></div></div>' +
        noTextButton("btn-flat chat-message-creator__add-members__close-btn", "xmark") +
        "</div></div>" +
        listHtml(items(), state.highlighted, membersCount() >= cfg.maxDmUsers, membersCount()) +
        "</span>" +
        footerHtml() +
        "</div></div>"
      );
    }

    // Draws the creator for the state, keeping the field being typed in
    // focused with its caret.
    var FIELDS = [
      "chat-message-creator__search-input__input",
      "chat-message-creator__members-input",
      "chat-message-creator__new-group-header__input",
    ];
    function render() {
      var creator = root.querySelector(".chat-message-creator");
      var active = document.activeElement;
      var focused = active && creator.contains(active)
        ? FIELDS.find(function (c) {
            return active.classList.contains(c);
          })
        : null;
      var caret = focused ? active.selectionStart : null;
      creator.innerHTML = state.mode === MODES.search ? searchModeHtml() : newGroupHtml();
      var values = {
        "chat-message-creator__search-input__input": state.term,
        "chat-message-creator__members-input": state.filter,
        "chat-message-creator__new-group-header__input": state.newGroupTitle,
      };
      FIELDS.forEach(function (c) {
        var input = creator.querySelector("." + c);
        if (input) {
          input.value = values[c] || "";
        }
      });
      var target = (focused && creator.querySelector("." + focused)) ||
        creator.querySelector(state.mode === MODES.search ? ".chat-message-creator__search-input__input" : ".chat-message-creator__members-input");
      if (target) {
        target.focus();
        if (caret !== null && target.setSelectionRange) {
          target.setSelectionRange(caret, caret);
        }
      }
    }
    var refresh = render;

    function changeMode(mode, members) {
      state.mode = mode;
      state.members = members || [];
      state.chatables = [];
      state.term = "";
      state.filter = "";
      state.highlightedMember = null;
      state.highlighted = null;
      clearTimeout(searchTimer);
      seq++;
      render();
      if (mode === MODES.search) {
        fetchSearch(null);
      }
    }

    // ChatMessageCreatorSearch#fetch and MembersSelector#fetch
    function fetchSearch(term) {
      var mine = ++seq;
      var options =
        state.mode === MODES.search
          ? { preloadChannels: !state.term }
          : { includeCategoryChannels: false, includeDirectMessageChannels: false };
      search(context, state.mode === MODES.search ? state.term : term, options)
        .then(function (found) {
          if (mine !== seq) {
            return;
          }
          state.chatables = found || [];
          state.highlighted = items()[0] || null;
          refresh();
        })
        .catch(function (e) {
          if (mine === seq) {
            popupAjaxError(e);
          }
        });
    }

    function onFilter(value) {
      clearTimeout(searchTimer);
      if (state.mode === MODES.search) {
        state.chatables = [];
        state.term = value;
      } else {
        state.filter = value;
      }
      refresh();
      searchTimer = setTimeout(function () {
        fetchSearch(value);
      }, INPUT_DELAY);
    }

    function done(channel) {
      if (!channel) {
        return;
      }
      closeModal();
      openChannel(channel);
    }

    // ChatMessageCreatorSearch#selectChatable
    function selectInSearch(item) {
      if (item.type === "list-action") {
        changeMode(MODES.new_group);
      } else if (item.type === "user") {
        if (item.enabled) {
          upsertDmChannel({ usernames: [item.model.username] }).then(done, popupAjaxError);
        }
      } else if (item.type === "group") {
        if (item.enabled) {
          changeMode(MODES.new_group, [item]);
        }
      } else {
        closeModal();
        openChannel(item.model);
      }
    }

    // MembersSelector#selectChatable
    function selectMember(item) {
      if (!item.enabled) {
        return;
      }
      var adds = item.type === "group" ? item.model.chat_enabled_user_count : 1;
      if (membersCount() + adds > cfg.maxDmUsers) {
        return;
      }
      var at = state.members.findIndex(function (m) {
        return m.identifier === item.identifier;
      });
      if (at !== -1) {
        state.members.splice(at, 1);
      } else {
        state.members.push(item);
        state.highlighted = items()[0] || null;
      }
      state.filter = "";
      state.highlightedMember = null;
      refresh();
      root.querySelector(".chat-message-creator__members-input").focus();
    }

    // MembersSelector#unselectMember
    function unselectMember(member) {
      state.members = state.members.filter(function (m) {
        return m !== member;
      });
      state.highlightedMember = null;
      state.highlighted = items()[0] || null;
      refresh();
      root.querySelector(".chat-message-creator__members-input").focus();
    }

    function select(item, shift) {
      if (shift && state.mode === MODES.search) {
        // prefillAddMembers
        changeMode(MODES.new_group, [item]);
        return;
      }
      if (state.mode === MODES.search) {
        selectInSearch(item);
      } else {
        selectMember(item);
      }
    }

    // NewGroup#createGroup
    function createGroup() {
      var usernames = state.members
        .filter(function (m) {
          return m.type === "user";
        })
        .map(function (m) {
          return m.model.username;
        });
      var groups = state.members
        .filter(function (m) {
          return m.type === "group";
        })
        .map(function (m) {
          return m.model.name;
        });
      createDmChannel({ usernames: usernames, groups: groups }, { name: state.newGroupTitle }).then(done, popupAjaxError);
    }

    // The cancel action: back to the search, the members gone.
    function cancel() {
      changeMode(MODES.search);
    }

    function itemAt(li) {
      var id = li.getAttribute("data-identifier");
      return items().find(function (item) {
        return item.identifier === id;
      });
    }

    function highlight(item) {
      state.highlighted = item || null;
      root.querySelectorAll(".chat-message-creator__list-item").forEach(function (li) {
        var it = itemAt(li);
        li.classList.toggle("-highlighted", !!(it && state.highlighted && it.identifier === state.highlighted.identifier));
      });
    }

    closeModal = Discourse.modal.show({
      className: "chat-modal-new-message --quick-palette",
      hideHeader: true,
      title: s.title,
      body: function (body) {
        body.innerHTML = '<div class="chat-message-creator-container"><div class="chat-message-creator"></div></div>';
        root = body;
        render();

        // ListHandler and Members' keys.
        body.addEventListener("keydown", function (event) {
          var input = event.target;
          if (input.classList && input.classList.contains("chat-message-creator__members-input") && input.value === "") {
            if (event.key === "Backspace" && state.members.length) {
              event.preventDefault();
              event.stopPropagation();
              if (!state.highlightedMember) {
                state.highlightedMember = state.members[state.members.length - 1];
                refresh();
              } else {
                unselectMember(state.highlightedMember);
              }
              return;
            }
            if ((event.key === "ArrowLeft" || event.key === "ArrowRight") && state.members.length) {
              event.preventDefault();
              event.stopPropagation();
              state.highlightedMember = step(state.members.map(function (m) {
                return Object.assign({}, m, { enabled: true });
              }), state.highlightedMember, event.key === "ArrowRight");
              state.highlightedMember =
                state.members.find(function (m) {
                  return state.highlightedMember && m.identifier === state.highlightedMember.identifier;
                }) || null;
              refresh();
              return;
            }
            if (event.key === "Enter" && state.highlightedMember) {
              event.preventDefault();
              event.stopPropagation();
              unselectMember(state.highlightedMember);
              return;
            }
          }
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            event.stopPropagation();
            highlight(step(items(), state.highlighted, event.key === "ArrowDown"));
          } else if (event.key === "Enter" && state.highlighted && !input.closest(".chat-message-creator__new-group-header")) {
            event.preventDefault();
            event.stopPropagation();
            select(state.highlighted, event.shiftKey);
          }
        });
        body.addEventListener("input", function (event) {
          var input = event.target;
          if (input.classList.contains("chat-message-creator__search-input__input")) {
            onFilter(input.value);
          } else if (input.classList.contains("chat-message-creator__members-input")) {
            state.highlightedMember = null;
            onFilter(input.value);
          } else if (input.classList.contains("chat-message-creator__new-group-header__input")) {
            state.newGroupTitle = input.value;
          }
        });
        body.addEventListener("click", function (event) {
          var li = event.target.closest(".chat-message-creator__list-item");
          if (li) {
            var item = itemAt(li);
            if (item) {
              select(item, event.shiftKey);
            }
            return;
          }
          var member = event.target.closest(".chat-message-creator__member");
          if (member) {
            var chips = Array.prototype.slice.call(root.querySelectorAll(".chat-message-creator__member"));
            unselectMember(state.members[chips.indexOf(member)]);
            return;
          }
          if (event.target.closest(".chat-message-creator__search-input__cancel-button")) {
            closeModal();
          } else if (
            event.target.closest(".chat-message-creator__add-members__close-btn") ||
            event.target.closest(".chat-message-creator__new-group-footer .btn-flat")
          ) {
            cancel();
          } else if (event.target.closest(".create-chat-group")) {
            createGroup();
          }
        });
        body.addEventListener("mousemove", function (event) {
          var li = event.target.closest(".chat-message-creator__list-item");
          if (li) {
            var item = itemAt(li);
            if (item && (!state.highlighted || state.highlighted.identifier !== item.identifier)) {
              highlight(item);
            }
          }
        });
        body.addEventListener(
          "mouseleave",
          function (event) {
            if (event.target.classList && event.target.classList.contains("chat-message-creator__list-item")) {
              highlight(null);
            }
          },
          true
        );
      },
    });

    // The member's channels for the preloaded list, then the first fetch.
    myChannels()
      .catch(function () {
        return [];
      })
      .then(function (channels) {
        context.channels = channels;
        if (state.mode === MODES.search) {
          fetchSearch(null);
        }
      });
  }

  // ChatDirectMessageButton#startChatting: the DM with the profile's or
  // the card's user, the card closed.
  function startChatting(username, fromCard) {
    upsertDmChannel({ usernames: [username] })
      .then(function (channel) {
        if (channel) {
          openChannel(channel);
        }
      }, popupAjaxError)
      .finally(function () {
        if (fromCard && Discourse.closeUserCard) {
          Discourse.closeUserCard();
        }
      });
  }

  function isInputSelection(el) {
    var tag = el && el.tagName ? el.tagName.toLowerCase() : "";
    return ["input", "textarea", "select", "button"].indexOf(tag) !== -1 || !!(el && el.closest && el.closest('[contenteditable="true"]'));
  }

  // route-recognizer's reading of a link's query.
  function linkParams(url) {
    var out = {};
    url.searchParams.forEach(function (value, key) {
      out[key] = value;
    });
    return out;
  }

  document.addEventListener(
    "click",
    function (event) {
      if (event.defaultPrevented || event.button !== 0 || !(event.target instanceof Element)) {
        return;
      }
      var target = event.target;
      var chatButton = target.closest(".chat-direct-message-btn");
      if (chatButton) {
        event.preventDefault();
        var card = chatButton.closest("#user-card");
        var username = null;
        if (card) {
          var match = card.className.match(/(?:^|\s)user-card-(\S+)/);
          username = match && match[1];
        } else {
          var path = location.pathname.slice(basePath().length).match(/^\/u\/([^/?#]+)/);
          username = path && decodeURIComponent(path[1]);
        }
        if (username) {
          startChatting(username, !!card);
        }
        return;
      }
      // ChannelsListDirect's and BrowseChannels' empty state buttons.
      if (target.closest("#direct-message-channels .empty-state__cta button, .chat-browse-view .empty-state__cta button")) {
        event.preventDefault();
        event.stopPropagation();
        showModal();
        return;
      }
      // A transition to chat.new-message in the app: without recipients or
      // a channel the route opens the modal where the member is; with
      // them, the page load goes where the route goes.
      var link = target.closest("a[href]");
      if (!link || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey || (link.target && link.target !== "_self")) {
        return;
      }
      var url = new URL(link.href, location.href);
      if (url.origin !== location.origin || url.pathname !== basePath() + "/chat/new-message") {
        return;
      }
      var p = linkParams(url);
      if (p.recipients || p.channel_id || p.channel) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      showModal();
    },
    true
  );

  // chat-keyboard-shortcuts' openQuickChannelSelector: Ctrl/Cmd+K, except
  // in a field other than the chat composer.
  var mac = /Mac|iPhone|iPad/.test(navigator.platform || "");
  document.addEventListener("keydown", function (event) {
    if (event.key !== "k" || event.altKey || event.shiftKey || !(mac ? event.metaKey : event.ctrlKey)) {
      return;
    }
    var el = event.target;
    if (isInputSelection(el) && !(el.classList && el.classList.contains("chat-composer__input"))) {
      return;
    }
    if (document.querySelector(".d-modal")) {
      return;
    }
    event.preventDefault();
    event.stopPropagation();
    showModal();
  });

  // ChatNewMessageRoute#beforeModel with recipients: the server serves
  // /chat/new-message without writing (it redirects for a channel), and
  // the direct message is found or created from here, then opened. On
  // an error the dialog shows over the page, which stays.
  Discourse.onPage(function () {
    if (location.pathname !== basePath() + "/chat/new-message") {
      return;
    }
    var p = new URLSearchParams(location.search);
    var recipients = p.get("recipients");
    if (recipients === null || p.get("channel_id") || p.get("channel")) {
      return;
    }
    var left = false;
    upsertDmChannel({ usernames: recipients.split(",") }).then(function (channel) {
      if (channel && !left) {
        openChannel(channel);
      }
    }, popupAjaxError);
    return function () {
      left = true;
    };
  });

  Discourse.chatNewMessage = { show: showModal, startChatting: startChatting };
})();
