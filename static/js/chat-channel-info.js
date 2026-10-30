// A chat channel's info routes (ChatRouteChannelInfoSettings and
// ChatRouteChannelInfoMembers), drawn by plugins/chat/info.rs. The
// settings: the name, slug and emoji modal (ChatModalEditChannelName),
// the description modal (ChatModalEditChannelDescription), the member's
// mute switch and notification level (PUT notifications-settings/me),
// the admin switches (auto join after its confirmation, channel-wide
// mentions, threading: PUT /chat/api/channels/:id), closing and opening
// (ChatModalToggleChannelStatus, PUT .../status), deleting
// (ChatModalDeleteChannel, DELETE /chat/api/channels/:id) and leaving or
// joining (ToggleChannelMembershipButton). The members: the filter, the
// next members as the list scrolls (the page's members/page route), a
// member's profile and removing a member (DELETE .../memberships/:id).
// A change redraws the route from the page as the server draws it now.
(function () {
  "use strict";

  if (!Discourse.once("chat-channel-info")) {
    return;
  }

  // discourse/lib/environment's INPUT_DELAY.
  var INPUT_DELAY = 250;
  var DESCRIPTION_MAX_LENGTH = 280;
  var SLUG_MAX_LENGTH = 100;

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

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function settingsRoot() {
    return document.querySelector(".c-channel-settings[data-channel-id]");
  }

  function strings() {
    var root = settingsRoot();
    try {
      return JSON.parse((root && root.dataset.strings) || "{}");
    } catch (_) {
      return {};
    }
  }

  // extractError: the response's errors, else the generic one.
  function errorText(body, status) {
    if (body && body.errors && body.errors.length) {
      return body.errors.join("\n");
    }
    if (body && body.error) {
      return body.error;
    }
    return strings().generic_error || String(status);
  }

  // A chat API request; a failure rejects with its error text.
  function api(method, path, data) {
    return fetch(basePath() + "/chat/api" + path, {
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
            throw new Error(errorText(body, r.status));
          }
          return body;
        });
    });
  }

  // popupAjaxError
  function popupError(error) {
    if (Discourse.dialog) {
      Discourse.dialog.popupAjaxError(error.message);
    } else {
      window.alert(error.message);
    }
  }

  // The toasts service's success toast.
  function toast(text) {
    var tpl = document.querySelector("template.chat-toast-template");
    if (!tpl) {
      return;
    }
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var output = tpl.content.querySelector(".fk-d-toast").cloneNode(true);
    output.querySelector(".fk-d-default-toast__message").textContent = text;
    section.appendChild(output);
    var close = function () {
      output.remove();
    };
    output.querySelector(".fk-d-default-toast__close-container button").addEventListener("click", close);
    setTimeout(close, 3000);
  }

  // A route in the app: through the drawer's router when there is one.
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
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // The info route drawn again, as the server draws it now.
  function redraw() {
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var next = doc.querySelector(".c-routes.--channel-info");
        var current = document.querySelector(".c-routes.--channel-info");
        if (next && current) {
          current.replaceWith(next);
          if (window.htmx) {
            window.htmx.process(next);
          }
        }
      });
  }

  function channelId() {
    var root = settingsRoot() || document.querySelector(".c-channel-members[data-channel-id]");
    return root ? root.dataset.channelId : null;
  }

  function channelPath(slug) {
    return "/chat/c/" + (slug || "-") + "/" + channelId();
  }

  function updateChannel(changes) {
    return api("PUT", "/channels/" + channelId(), { channel: changes });
  }

  // _updateChannelProperty: saved, then the route redrawn.
  function updateProperty(property, value) {
    var changes = {};
    changes[property] = value;
    return updateChannel(changes)
      .then(function () {
        toast(strings().saved);
        return redraw();
      })
      .catch(popupError);
  }

  function saveNotificationSettings(key, value) {
    var settings = {};
    settings[key] = value;
    return api("PUT", "/channels/" + channelId() + "/notifications-settings/me", {
      notifications_settings: settings,
    })
      .then(function () {
        toast(strings().saved);
        return redraw();
      })
      .catch(popupError);
  }

  // A standard emoji's image, as the picker's template gives it.
  function emojiUrl(name) {
    var tpl = document.querySelector("template.emoji-picker-template");
    var url = (tpl && tpl.dataset.emojiUrl) || basePath() + "/images/emoji/twitter/%{name}.png";
    return url.replace("%{name}", encodeURIComponent(name));
  }

  function switchState(button) {
    return button.getAttribute("aria-checked") === "true";
  }

  function footerButton(classes, label, extra) {
    return (
      '<button class="btn btn-text ' +
      classes +
      '"' +
      (extra || "") +
      ' type="button"><span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  // DModal's flash.
  function setFlash(body, text, type) {
    var flash = body.parentNode.querySelector(".d-modal__flash");
    if (!text) {
      if (flash) {
        flash.remove();
      }
      return;
    }
    if (!flash) {
      flash = document.createElement("div");
      body.parentNode.insertBefore(flash, body);
    }
    flash.className = "alert d-modal__flash alert-" + (type || "error");
    flash.setAttribute("role", "alert");
    flash.innerHTML = '<span class="d-modal__flash-message">' + escapeHtml(text) + "</span>";
  }

  // ChatModalEditChannelName
  function editName() {
    var root = settingsRoot();
    var s = strings();
    var original = { name: root.dataset.title, slug: root.dataset.slug, emoji: root.dataset.emoji || null };
    var edited = { name: original.name, slug: original.slug, emoji: original.emoji };
    var autoSlug = original.slug;
    var maxTitle = Number(root.dataset.maxTitle) || 255;
    var slugTimer = null;
    var bodyEl;
    var saveButton;

    function saveDisabled() {
      return (
        (original.name === edited.name && original.slug === edited.slug && original.emoji === edited.emoji) ||
        (edited.name || "").length > maxTitle ||
        (edited.slug || "").length > SLUG_MAX_LENGTH
      );
    }

    function refresh() {
      if (saveButton) {
        saveButton.disabled = saveDisabled();
      }
      var emojiButton = bodyEl.querySelector(".btn-emoji");
      emojiButton.innerHTML = edited.emoji
        ? '<img width="20" height="20" class="emoji" src="' +
          escapeHtml(emojiUrl(edited.emoji)) +
          '" title="' +
          escapeHtml(edited.emoji) +
          '" alt="' +
          escapeHtml(edited.emoji) +
          '">' +
          icon("angle-down")
        : icon("discourse-emojis") + icon("angle-down");
      bodyEl.querySelector(".edit-channel-clear-emoji").disabled = !edited.emoji;
    }

    // #debouncedGenerateSlug: POST /slugs.json for the name.
    function generateSlug(name) {
      clearTimeout(slugTimer);
      autoSlug = "";
      bodyEl.querySelector(".chat-channel-edit-name-slug-modal__slug-input").placeholder = "";
      if (!name) {
        return;
      }
      slugTimer = setTimeout(function () {
        fetch(basePath() + "/slugs.json", {
          method: "POST",
          credentials: "same-origin",
          headers: headers(true),
          body: JSON.stringify({ name: name }),
        })
          .then(function (r) {
            return r.json();
          })
          .then(function (body) {
            autoSlug = body.slug || "";
            bodyEl.querySelector(".chat-channel-edit-name-slug-modal__slug-input").placeholder = autoSlug;
          })
          .catch(function () {});
      }, 300);
    }

    Discourse.modal.show({
      className: "chat-modal-edit-channel-name",
      title: s.edit_name_title,
      body: function (body) {
        bodyEl = body;
        body.innerHTML =
          '<div class="edit-channel-control"><label class="edit-channel-label" for="channel-name">' +
          escapeHtml(s.name_label) +
          '</label><input class="chat-channel-edit-name-slug-modal__name-input ember-text-field ember-view" name="channel-name" placeholder="' +
          escapeHtml(s.name_placeholder) +
          '" type="text"></div>' +
          '<div class="edit-channel-control"><label class="edit-channel-label" for="channel-slug">' +
          escapeHtml(s.slug_label) +
          '&nbsp;<span class="fk-d-tooltip__trigger" data-tooltip="' +
          escapeHtml(s.slug_description) +
          '"><span class="fk-d-tooltip__trigger-container">' +
          icon("circle-info") +
          '</span></span></label><input class="chat-channel-edit-name-slug-modal__slug-input ember-text-field ember-view" name="channel-slug" placeholder="' +
          escapeHtml(autoSlug) +
          '" type="text"></div>' +
          '<div class="edit-channel-control"><label class="edit-channel-label" for="channel-emoji">' +
          escapeHtml(s.emoji_label) +
          '</label><button class="btn no-text btn-icon btn-default btn-emoji" type="button"></button>' +
          footerButton("btn-flat edit-channel-clear-emoji", s.reset_emoji) +
          "</div>";
        var nameInput = body.querySelector(".chat-channel-edit-name-slug-modal__name-input");
        var slugInput = body.querySelector(".chat-channel-edit-name-slug-modal__slug-input");
        nameInput.value = edited.name || "";
        slugInput.value = edited.slug || "";
        nameInput.addEventListener("input", function () {
          edited.name = nameInput.value;
          setFlash(body, null);
          generateSlug(nameInput.value);
          refresh();
        });
        slugInput.addEventListener("input", function () {
          edited.slug = slugInput.value;
          setFlash(body, null);
          generateSlug(edited.name);
          refresh();
        });
        body.querySelector(".btn-emoji").addEventListener("click", function (event) {
          if (!Discourse.emojiPicker) {
            return;
          }
          Discourse.emojiPicker.open(event.currentTarget, {
            context: "channel-emoji",
            onSelect: function (emoji) {
              edited.emoji = emoji;
              refresh();
            },
          });
        });
        body.querySelector(".edit-channel-clear-emoji").addEventListener("click", function () {
          edited.emoji = null;
          refresh();
        });
        refresh();
      },
      footer: function (footer, close) {
        footer.innerHTML =
          footerButton("btn-primary create", s.save) + footerButton("btn-flat d-modal-cancel", s.cancel);
        saveButton = footer.querySelector(".create");
        saveButton.disabled = saveDisabled();
        footer.querySelector(".d-modal-cancel").addEventListener("click", close);
        saveButton.addEventListener("click", function () {
          updateChannel({
            name: edited.name,
            slug: edited.slug || autoSlug || original.slug,
            emoji: edited.emoji,
          })
            .then(function (result) {
              close();
              go(channelPath(result.channel.slug));
            })
            .catch(function (error) {
              setFlash(bodyEl, error.message);
            });
        });
      },
    });
  }

  // ChatModalEditChannelDescription, with DCharCounter.
  function editDescription() {
    var root = settingsRoot();
    var s = strings();
    var original = root.dataset.description || "";
    var edited = original;
    var bodyEl;
    var saveButton;

    function saveDisabled() {
      return original === edited || edited.length > DESCRIPTION_MAX_LENGTH;
    }

    Discourse.modal.show({
      className: "chat-modal-edit-channel-description",
      title: s.edit_description_title,
      body: function (body) {
        bodyEl = body;
        body.innerHTML =
          '<span class="chat-modal-edit-channel-description__description">' +
          escapeHtml(s.description_label) +
          '</span><div class="char-counter"><textarea class="chat-modal-edit-channel-description__description-input" placeholder="' +
          escapeHtml(s.description_placeholder) +
          '"></textarea><small class="char-counter__ratio">' +
          edited.length +
          "/" +
          DESCRIPTION_MAX_LENGTH +
          "</small></div>";
        var textarea = body.querySelector("textarea");
        textarea.value = edited;
        textarea.addEventListener("input", function () {
          edited = textarea.value;
          setFlash(body, null);
          var counter = body.querySelector(".char-counter");
          counter.classList.toggle("exceeded", edited.length > DESCRIPTION_MAX_LENGTH);
          body.querySelector(".char-counter__ratio").textContent = edited.length + "/" + DESCRIPTION_MAX_LENGTH;
          if (saveButton) {
            saveButton.disabled = saveDisabled();
          }
        });
      },
      footer: function (footer, close) {
        footer.innerHTML =
          footerButton("btn-primary create", s.save) + footerButton("btn-flat d-modal-cancel", s.cancel);
        saveButton = footer.querySelector(".create");
        saveButton.disabled = saveDisabled();
        footer.querySelector(".d-modal-cancel").addEventListener("click", close);
        saveButton.addEventListener("click", function () {
          updateChannel({ description: edited })
            .then(function () {
              close();
              return redraw();
            })
            .catch(function (error) {
              setFlash(bodyEl, error.message);
            });
        });
      },
    });
  }

  // ChatModalToggleChannelStatus
  function toggleStatus(closed) {
    var s = strings();
    var root = settingsRoot();
    Discourse.modal.show({
      className: "chat-modal-toggle-channel-status",
      title: closed ? s.open_title : s.close_title,
      body: function (body) {
        body.innerHTML =
          '<p class="chat-modal-toggle-channel-status__instructions">' +
          (closed ? s.open_instructions : s.close_instructions) +
          "</p>";
      },
      footer: function (footer, close) {
        footer.innerHTML = footerButton(
          "btn-primary",
          closed ? s.open_channel : s.close_channel,
          ' id="chat-channel-toggle-btn"'
        );
        footer.querySelector("button").addEventListener("click", function () {
          api("PUT", "/channels/" + channelId() + "/status", { status: closed ? "open" : "closed" })
            .then(function () {
              close();
              go(channelPath(root.dataset.slug));
            })
            .catch(popupError);
        });
      },
    });
  }

  // ChatModalDeleteChannel: the channel's name typed to confirm.
  function deleteChannel() {
    var s = strings();
    var root = settingsRoot();
    var title = root.dataset.title || "";
    var confirmation = "";
    var deleting = false;
    var confirmed = false;
    var bodyEl;
    var button;

    function disabled() {
      return deleting || confirmed || !confirmation || confirmation.toLowerCase() !== title.toLowerCase();
    }

    Discourse.modal.show({
      className: "chat-modal-delete-channel",
      title: s.delete_title,
      body: function (body) {
        bodyEl = body;
        body.innerHTML =
          '<p class="chat-modal-delete-channel__instructions">' +
          (s.delete_instructions || "").replace("%{name}", escapeHtml(title)) +
          '</p><input autocapitalize="off" autocorrect="off" id="channel-delete-confirm-name" placeholder="' +
          escapeHtml(s.delete_confirm_name) +
          '" type="text">';
        body.querySelector("input").addEventListener("input", function (event) {
          confirmation = event.target.value;
          if (button) {
            button.disabled = disabled();
          }
        });
      },
      footer: function (footer, close) {
        footer.innerHTML =
          footerButton("btn-danger", s.delete_confirm, ' id="chat-confirm-delete-channel"') +
          footerButton("btn-flat", s.cancel);
        button = footer.querySelector(".btn-danger");
        button.disabled = true;
        footer.querySelector(".btn-flat").addEventListener("click", close);
        button.addEventListener("click", function () {
          deleting = true;
          button.disabled = true;
          api("DELETE", "/channels/" + channelId())
            .then(function () {
              confirmed = true;
              setFlash(bodyEl, s.delete_started, "success");
              setTimeout(function () {
                close();
                go("/chat");
              }, 3000);
            })
            .catch(popupError)
            .finally(function () {
              deleting = false;
              button.disabled = disabled();
            });
        });
      },
    });
  }

  // The notification level ComboBox: its rows open under the header.
  function openCombo(details) {
    var body = details.querySelector(".select-kit-body");
    var rows = details.querySelector("template.select-kit-rows");
    if (!body || !rows) {
      return;
    }
    body.innerHTML = '<ul class="select-kit-collection" role="menu"></ul>';
    body.firstChild.appendChild(rows.content.cloneNode(true));
    details.classList.add("is-expanded");
  }

  function closeCombo(details) {
    details.open = false;
    details.classList.remove("is-expanded");
    var body = details.querySelector(".select-kit-body");
    if (body) {
      body.innerHTML = "";
    }
  }

  function onSettingsClick(event) {
    var target = event.target;
    var root = settingsRoot();
    if (!root || !root.contains(target)) {
      return;
    }
    var s = strings();
    var hit = function (selector) {
      return target.closest(selector);
    };
    var row = hit(".c-channel-settings__notifications-selector .select-kit-row");
    if (row) {
      event.preventDefault();
      closeCombo(row.closest("details"));
      saveNotificationSettings("notification_level", row.dataset.value);
      return;
    }
    var button;
    if (hit(".edit-name-slug-btn")) {
      editName();
    } else if (hit(".edit-description-btn")) {
      editDescription();
    } else if ((button = hit(".c-channel-settings__mute-switch"))) {
      saveNotificationSettings("muted", !switchState(button));
    } else if ((button = hit(".c-channel-settings__auto-join-switch"))) {
      if (switchState(button)) {
        updateProperty("auto_join_users", false);
      } else if (Discourse.dialog) {
        Discourse.dialog.confirm({
          message: escapeHtml((s.auto_join_warning || "").replace("%{category}", root.dataset.categoryName || "")),
          didConfirm: function () {
            updateProperty("auto_join_users", true);
          },
        });
      }
    } else if ((button = hit(".c-channel-settings__channel-wide-mentions"))) {
      updateProperty("allow_channel_wide_mentions", !switchState(button));
    } else if ((button = hit(".c-channel-settings__threading-switch"))) {
      updateProperty("threading_enabled", !switchState(button));
    } else if (hit(".close-btn")) {
      toggleStatus(false);
    } else if ((button = hit(".open-btn")) && !button.disabled) {
      toggleStatus(true);
    } else if (hit(".delete-btn")) {
      deleteChannel();
    } else if ((button = hit(".toggle-channel-membership-button"))) {
      button.disabled = true;
      var leaving = button.classList.contains("-leave");
      var group = !!root.querySelector(".c-channel-settings__leave-info");
      var request = leaving
        ? group
          ? api("DELETE", "/channels/" + channelId() + "/memberships/me")
          : api("DELETE", "/channels/" + channelId() + "/memberships/me/follows")
        : api("POST", "/channels/" + channelId() + "/memberships/me");
      request
        .then(function () {
          if (leaving) {
            go("/chat");
          } else {
            return redraw();
          }
        })
        .catch(function (error) {
          button.disabled = false;
          popupError(error);
        });
    }
  }

  document.addEventListener("toggle", function (event) {
    var details = event.target;
    if (!details.matches || !details.matches(".c-channel-settings details.combo-box")) {
      return;
    }
    if (details.open) {
      openCombo(details);
    } else {
      closeCombo(details);
    }
  }, true);

  document.addEventListener("click", onSettingsClick);

  // The members list: the filter's results, the next page as the list
  // scrolls, a profile, a removal.
  var filterTimer = null;
  var loading = false;
  var done = false;

  function membersRoot() {
    return document.querySelector(".c-channel-members[data-channel-id]");
  }

  function loadMembers(replace) {
    var root = membersRoot();
    if (!root || loading || (done && !replace)) {
      return Promise.resolve();
    }
    var list = root.querySelector(".c-channel-members__list");
    var filter = root.querySelector(".c-channel-members__filter input");
    var params = new URLSearchParams();
    var offset = replace ? 0 : list.querySelectorAll(".-member").length;
    params.set("offset", offset);
    if (filter && filter.value) {
      params.set("username", filter.value);
    }
    loading = true;
    return fetch(root.dataset.pageUrl + "?" + params.toString(), { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        if (replace) {
          list.innerHTML = html;
          done = false;
        } else if (html.trim()) {
          list.insertAdjacentHTML("beforeend", html);
        } else {
          done = true;
        }
      })
      .catch(function () {})
      .finally(function () {
        loading = false;
      });
  }

  document.addEventListener("input", function (event) {
    if (!event.target.closest(".c-channel-members__filter")) {
      return;
    }
    clearTimeout(filterTimer);
    filterTimer = setTimeout(function () {
      loadMembers(true);
    }, INPUT_DELAY);
  });

  document.addEventListener("click", function (event) {
    var root = membersRoot();
    if (!root || !root.contains(event.target)) {
      return;
    }
    var item = event.target.closest(".c-channel-members__list-item.-member");
    if (!item) {
      return;
    }
    if (event.target.closest(".-remove-member")) {
      api("DELETE", "/channels/" + root.dataset.channelId + "/memberships/" + item.dataset.userId)
        .then(function () {
          return loadMembers(true);
        })
        .catch(popupError);
      return;
    }
    if (event.target.closest(".-user-info")) {
      event.preventDefault();
      go("/u/" + encodeURIComponent(item.dataset.username.toLowerCase()));
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key !== "Enter") {
      return;
    }
    var info = event.target.closest && event.target.closest(".c-channel-members .-user-info");
    if (info) {
      info.click();
    }
  });

  Discourse.onPage(function () {
    var root = membersRoot();
    if (!root || !window.IntersectionObserver) {
      return;
    }
    done = false;
    var observer = new IntersectionObserver(function (entries) {
      if (entries.some(function (e) { return e.isIntersecting; })) {
        loadMembers(false);
      }
    });
    var sentinel = root.querySelector(".c-channel-members__load-more");
    if (sentinel) {
      observer.observe(sentinel);
    }
    return function () {
      observer.disconnect();
    };
  });
})();
