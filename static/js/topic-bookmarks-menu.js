// The topic footer's grouped bookmarks menu, TopicBookmarksMenu
// (components/topic-bookmarks-menu.gjs) with a post's bookmark among the
// member's in the topic: a DMenu (modalForMobile, no arrow) whose content
// is the server's, the page's template.topic-bookmarks-menu-template (its
// data-topic the bookmarks, the topic's URL, a new topic bookmark's
// saveData and the strings).
//
// A post's row opens TopicBookmarkPostSubmenu by it (the menu service's
// show, right-start, modalForMobile), closing the menu when it closes:
// Jump to the post (DiscourseURL.routeTo), Delete (DELETE /bookmarks/:id).
// Delete topic bookmark DELETEs the topic's, Bookmark topic POSTs
// /bookmarks with BookmarkFormData's saveData, and Delete all bookmarks
// asks first (dialog.yesNoConfirm) then PUTs /t/:id/remove_bookmarks, as
// Topic#deleteBookmarks. Each shows its toast; Ember then updates the
// topic's bookmarks in place, here the footer and the posts concerned are
// drawn again from the server. A post's bookmark toggled from its menu
// redraws the footer's button too, as Ember's follows the topic's
// bookmarks. A failed delete is popped up and Ember goes on as if it was
// done (bookmarkApi.delete catches it); a failed create is popped up.
//
// Editing a bookmark (the topic's or a post's) opens BookmarkModal
// (bookmark-modal.js); once it saved or deleted, the footer and the post
// are drawn again.
(function () {
  "use strict";

  if (!Discourse.once("topic-bookmarks-menu")) {
    return;
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  // The member's CSRF token and XHR header, from the body's hx-headers.
  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      return {};
    }
  }

  function request(method, path, form) {
    var headers = memberHeaders();
    var init = { method: method, headers: headers, credentials: "same-origin" };
    if (form) {
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      init.body = new URLSearchParams(form).toString();
    }
    return fetch(base() + path, init);
  }

  // popupAjaxError: the errors the server gave, in a dialog.
  function popupError(r) {
    return r
      .json()
      .catch(function () {
        return {};
      })
      .then(function (body) {
        var text = (body.errors || [r.statusText]).join("\n");
        if (Discourse.dialog) {
          Discourse.dialog.popupAjaxError(text);
        } else {
          window.alert(text);
        }
      });
  }

  function escapeHtml(value) {
    return String(value === null || value === undefined ? "" : value)
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

  // toasts.success({ duration: "short", data: { message, icon } }): the
  // success theme, its icon check unless given.
  function toast(message, iconName) {
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var output = document.createElement("output");
    output.className = "fk-d-toast";
    output.setAttribute("role", "status");
    output.innerHTML =
      '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
      icon(iconName || "check") +
      '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>';
    output.querySelector(".fk-d-default-toast__message").textContent = message;
    section.appendChild(output);
    var remove = function () {
      output.remove();
    };
    output.querySelector("button").addEventListener("click", remove);
    setTimeout(remove, 3000);
  }

  function template() {
    return document.querySelector("template.topic-bookmarks-menu-template");
  }

  function topicData() {
    var t = template();
    if (!t) {
      return null;
    }
    try {
      return JSON.parse(t.dataset.topic);
    } catch (e) {
      return null;
    }
  }

  var TRIGGER = "#topic-footer-buttons .bookmark-menu__trigger";

  // BookmarkModal for one of the topic's bookmarks. What it saved goes
  // into the menu's data at once, as Ember's bookmark model has it, so
  // the menu opened again before the redraw lands edits what was saved.
  function editBookmark(bookmark) {
    if (!bookmark || !Discourse.bookmarkModal) {
      return;
    }
    var again = function () {
      redraw(bookmark.bookmarkable_type === "Post" ? [bookmark.bookmarkable_id] : []);
    };
    Discourse.bookmarkModal.open({
      bookmark: bookmark,
      afterSave: function (saved) {
        var t = template();
        var topic = topicData();
        if (t && topic) {
          (topic.posts || []).concat(topic.topicBookmark ? [topic.topicBookmark] : []).forEach(function (held) {
            if (String(held.id) === String(saved.id)) {
              held.name = saved.name;
              held.reminder_at = saved.reminder_at || null;
              held.auto_delete_preference = saved.auto_delete_preference;
            }
          });
          t.dataset.topic = JSON.stringify(topic);
        }
        again();
      },
      afterDelete: again,
    });
  }

  // The footer's bookmark button and this menu's template as the server
  // now draws them, and the given posts again (their bookmark buttons).
  function redraw(postIds) {
    (postIds || []).forEach(function (id) {
      if (window.htmx && id) {
        window.htmx.ajax("GET", base() + "/live/post/" + id, { swap: "none" });
      }
    });
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var button = document.querySelector(TRIGGER);
        var freshButton = doc.querySelector(TRIGGER);
        var drawn = null;
        if (button && freshButton) {
          drawn = document.importNode(freshButton, true);
          button.replaceWith(drawn);
        }
        var current = template();
        var fresh = doc.querySelector("template.topic-bookmarks-menu-template");
        if (fresh) {
          fresh = document.importNode(fresh, true);
          if (current) {
            current.replaceWith(fresh);
          } else {
            document.body.appendChild(fresh);
          }
        } else if (current) {
          current.remove();
        }
        // BookmarkMenu's toggle, for the topic's bookmark alone, is htmx's.
        if (drawn && window.htmx) {
          window.htmx.process(drawn);
        }
      })
      .catch(function (e) {
        console.error(e);
      });
  }

  // bookmarkApi.delete: a failure is popped up and the caller goes on.
  function deleteBookmark(bookmark) {
    return request("DELETE", "/bookmarks/" + bookmark.id + ".json").then(function (r) {
      if (!r.ok) {
        return popupError(r);
      }
    });
  }

  var menu = null;
  var submenu = null;

  function closeMenu() {
    var open = menu;
    menu = null;
    return open ? Promise.resolve(open.close({ focusTrigger: false })) : Promise.resolve();
  }

  // onRemoveBookmark (a post's, from its submenu) and onRemoveTopicBookmark.
  function removeBookmark(topic, bookmark) {
    deleteBookmark(bookmark)
      .then(function () {
        redraw(bookmark.bookmarkable_type === "Post" ? [bookmark.bookmarkable_id] : []);
        toast(topic.strings.deleted, "trash-can");
      })
      .finally(closeMenu);
  }

  // onBookmarkTopic: TopicBookmarkManager#create.
  function bookmarkTopic(topic) {
    closeMenu().then(function () {
      request("POST", "/bookmarks.json", topic.newBookmark).then(function (r) {
        if (!r.ok) {
          return popupError(r);
        }
        toast(topic.strings.bookmarked);
        return redraw([]);
      });
    });
  }

  // onClearAllBookmarks: Topic#deleteBookmarks, then clearBookmarks.
  function clearAll(topic) {
    closeMenu().then(function () {
      Discourse.dialog.yesNoConfirm({
        message: topic.strings.confirmClear,
        didConfirm: function () {
          request("PUT", "/t/" + topic.id + "/remove_bookmarks").then(function (r) {
            if (!r.ok) {
              return popupError(r);
            }
            return redraw(
              topic.posts.map(function (b) {
                return b.bookmarkable_id;
              })
            );
          });
        },
      });
    });
  }

  // openPostBookmarkSubmenu: TopicBookmarkPostSubmenu by the row's button.
  function openSubmenu(topic, bookmark, rowButton) {
    var content = template().content.querySelector(".topic-bookmark-post-submenu").cloneNode(true);
    content.querySelector(".--jump .bookmark-menu__row-label").textContent = topic.strings.jumpToPost.replace(
      "%{post_number}",
      bookmark.post_number
    );
    if (submenu && submenu.expanded) {
      submenu.close({ focusTrigger: false });
    }
    var instance = Discourse.float.menu(rowButton, {
      identifier: "topic-bookmark-post-submenu",
      content: content,
      modalForMobile: true,
      placement: "right-start",
      offset: { mainAxis: 10, crossAxis: -5 },
      onClose: function () {
        closeMenu();
      },
    });
    submenu = instance;
    var close = function () {
      return Promise.resolve(instance.close({ focusTrigger: false }));
    };
    content.addEventListener("click", function (event) {
      var item = event.target.closest(".bookmark-menu__row");
      if (!item || !event.target.closest("button")) {
        return;
      }
      switch (item.dataset.menuOptionId) {
        case "jump":
          // onJumpToPost
          close();
          closeMenu().then(function () {
            Discourse.topicRoute.routeToPost(topic.url + "/" + bookmark.post_number, {
              skipIfOnScreen: false,
            });
          });
          break;
        case "edit":
          close();
          closeMenu().then(function () {
            editBookmark(bookmark);
          });
          break;
        case "delete":
          close();
          removeBookmark(topic, bookmark);
          break;
      }
    });
  }

  function openMenu(trigger) {
    var topic = topicData();
    if (!topic) {
      return;
    }
    if (menu && menu.expanded) {
      closeMenu();
      return;
    }
    var content = template().content.querySelector("ul.dropdown-menu").cloneNode(true);
    menu = Discourse.float.menu(trigger, {
      identifier: "topic-bookmarks-menu",
      modalForMobile: true,
      arrow: false,
      content: content,
    });
    content.addEventListener("click", function (event) {
      var button = event.target.closest(".bookmark-menu__row-btn");
      var item = button && button.closest(".bookmark-menu__row");
      if (!item) {
        return;
      }
      var id = item.dataset.menuOptionId;
      if (id.indexOf("post-") === 0) {
        var number = parseInt(id.slice(5), 10);
        var bookmark = topic.posts.filter(function (b) {
          return b.post_number === number;
        })[0];
        if (bookmark) {
          openSubmenu(topic, bookmark, button);
        }
        return;
      }
      switch (id) {
        case "edit-topic-bookmark":
          closeMenu().then(function () {
            editBookmark(topic.topicBookmark);
          });
          break;
        case "delete-topic-bookmark":
          removeBookmark(topic, topic.topicBookmark);
          break;
        case "bookmark-topic":
          bookmarkTopic(topic);
          break;
        case "clear-all":
          clearAll(topic);
          break;
      }
    });
  }

  document.addEventListener("click", function (event) {
    if (!window.Discourse || !Discourse.float) {
      return;
    }
    var trigger = event.target.closest("#topic-footer-buttons .topic-bookmarks-menu-trigger");
    if (trigger) {
      openMenu(trigger);
    }
  });

  // ---- BookmarkMenu (components/bookmark-menu.gjs) on a post's bookmark
  // button and the footer's while the topic's is the only bookmark: a
  // DMenu (no arrow). Showing it on what isn't bookmarked bookmarks it
  // at once (onBookmark) and offers a reminder: in two hours, tomorrow,
  // in three days, or more options (BookmarkModal). On a bookmark it
  // offers Edit (the modal), Clear reminder when there is one, and
  // Delete. The button's own htmx toggle is what is left without the
  // modal's script.

  function menuRow(id, className, iconName, label, title) {
    return (
      '<li class="dropdown-menu__item bookmark-menu__row' +
      (className ? " " + className : "") +
      '" data-menu-option-id="' +
      id +
      '"><button class="bookmark-menu__row-btn btn btn-icon-text' +
      (className === "--remove" ? " --danger" : " btn-transparent") +
      '"' +
      (title ? ' title="' + escapeHtml(title) + '"' : "") +
      ' type="button">' +
      icon(iconName) +
      '<span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button></li>"
    );
  }

  var bookmarkMenu = null;

  function openBookmarkMenu(trigger) {
    if (bookmarkMenu && bookmarkMenu.expanded) {
      bookmarkMenu.close();
      return;
    }
    var t = Discourse.bookmarkModal.t;
    var post = trigger.closest("[data-post-id]");
    var postId = post ? post.getAttribute("data-post-id") : null;
    var parse = function (value) {
      try {
        return value ? JSON.parse(value) : null;
      } catch (e) {
        return null;
      }
    };
    var existing = parse(trigger.dataset.bookmark);
    var quicksaved = false;
    var creating = null;
    var changed = false;
    var instance = null;
    var content = document.createElement("ul");
    content.className = "dropdown-menu";

    // The post drawn again, its own or (from the footer) the bookmark's.
    var again = function () {
      var bookmarked = existing && existing.bookmarkable_type === "Post" ? existing.bookmarkable_id : null;
      redraw(postId || bookmarked ? [postId || bookmarked] : []);
    };
    var close = function () {
      return Promise.resolve(instance && instance.close({ focusTrigger: false }));
    };
    // saveData, as BookmarkFormData sends it.
    var saveData = function (reminderAt) {
      return {
        reminder_at: reminderAt || "",
        name: existing.name || "",
        id: existing.id,
        auto_delete_preference:
          existing.auto_delete_preference === null || existing.auto_delete_preference === undefined
            ? 3
            : existing.auto_delete_preference,
        bookmarkable_id: existing.bookmarkable_id,
        bookmarkable_type: existing.bookmarkable_type,
      };
    };
    var save = function (reminderAt) {
      return request("PUT", "/bookmarks/" + encodeURIComponent(existing.id) + ".json", saveData(reminderAt)).then(
        function (r) {
          if (!r.ok) {
            return popupError(r).then(function () {
              return false;
            });
          }
          existing.reminder_at = reminderAt || null;
          changed = true;
          return true;
        }
      );
    };
    var openModal = function () {
      close().then(function () {
        Discourse.bookmarkModal.open({ bookmark: existing, afterSave: again, afterDelete: again });
      });
    };
    var mobile = function () {
      return document.documentElement.classList.contains("mobile-view");
    };

    var options = [];
    function draw() {
      var html = "";
      if (existing && !quicksaved) {
        // showEditDeleteMenu
        html += menuRow("edit", "-edit", "pencil", t("edit"));
        if (existing.reminder_at) {
          html += menuRow("clear-reminder", "--clear-reminder", "bell-slash", t("bookmarks.clear_reminder"));
        }
        html += menuRow("delete", "--remove", "trash-can", t("delete"));
      } else {
        html +=
          '<li class="dropdown-menu__item bookmark-menu__title">' +
          icon("circle-check") +
          "<span>" +
          escapeHtml(t("bookmarks.bookmarked_success")) +
          '</span></li><li class="dropdown-menu__item bookmark-menu__row-title">' +
          icon("bell") +
          escapeHtml(t("bookmarks.also_set_reminder")) +
          '</li><li><hr class="dropdown-menu__divider"></li>';
        options.forEach(function (option) {
          html += menuRow(
            option.id,
            "",
            option.icon,
            t(option.label),
            option.time ? option.time.format(t(option.format)) : ""
          );
        });
      }
      content.innerHTML = html;
    }

    // onBookmark: bookmarked at once, the reminder's menu kept (quicksaved).
    function create() {
      var vals = parse(trigger.getAttribute("hx-vals")) || {};
      creating = request("POST", "/bookmarks.json", vals)
        .then(function (r) {
          if (!r.ok) {
            return popupError(r).then(function () {
              close();
            });
          }
          return r.json().then(function (body) {
            existing = {
              id: body.id,
              name: "",
              reminder_at: null,
              auto_delete_preference: Number(vals.auto_delete_preference),
              bookmarkable_id: vals.bookmarkable_id,
              bookmarkable_type: vals.bookmarkable_type,
            };
            quicksaved = true;
            changed = true;
            trigger.classList.add("bookmarked");
            if (mobile()) {
              toast(t("bookmarks.bookmarked_success"));
            }
          });
        })
        .then(function () {
          creating = null;
        });
    }

    content.addEventListener("click", function (event) {
      var button = event.target.closest(".bookmark-menu__row-btn");
      var item = button && button.closest(".bookmark-menu__row");
      if (!item) {
        return;
      }
      var id = item.dataset.menuOptionId;
      // onChooseReminderOption waits for the bookmark being made.
      Promise.resolve(creating).then(function () {
        if (!existing) {
          return;
        }
        if (id === "edit" || id === "custom") {
          openModal();
        } else if (id === "clear-reminder") {
          save(null).then(function (saved) {
            if (saved) {
              toast(t("bookmarks.reminder_clear_success"));
            }
            close();
          });
        } else if (id === "delete") {
          request("DELETE", "/bookmarks/" + encodeURIComponent(existing.id) + ".json").then(function (r) {
            if (!r.ok) {
              return popupError(r).then(close);
            }
            changed = true;
            toast(t("bookmarks.deleted_bookmark_success"), "trash-can");
            close();
          });
        } else {
          var option = options.filter(function (o) {
            return o.id === id;
          })[0];
          if (!option || !option.time) {
            return;
          }
          save(option.time.toISOString()).then(function (saved) {
            if (saved && mobile()) {
              toast(t("bookmarks.reminder_set_success"));
            }
            close();
          });
        }
      });
    });

    Discourse.timeShortcutPicker.load().then(function () {
      // setReminderShortcuts
      var shortcuts = Discourse.timeShortcutPicker.shortcuts();
      options = [shortcuts.twoHours(), shortcuts.tomorrow(), shortcuts.threeDays()];
      options[0].icon = "far-clock";
      options[1].icon = "calendar-day";
      options[2].icon = "calendar-days";
      options.push({ id: "custom", label: "time_shortcut.more_options", icon: "far-calendar-plus", time: null });
      draw();
      instance = Discourse.float.menu(trigger, {
        identifier: "bookmark-menu",
        arrow: false,
        content: content,
        // onCloseMenu, then what changed drawn again: the post, the footer.
        // Closed while the bookmark is still being made, it waits for it.
        onClose: function () {
          bookmarkMenu = null;
          Promise.resolve(creating).then(function () {
            if (changed) {
              again();
            }
          });
        },
      });
      bookmarkMenu = instance;
      if (!existing) {
        create();
      }
    });
  }

  // Taken on the way down, ahead of the button's htmx toggle.
  document.addEventListener(
    "click",
    function (event) {
      var trigger =
        event.target.closest &&
        event.target.closest(".post-action-menu__bookmark.bookmark-menu-trigger, #topic-footer-buttons .bookmark-menu-trigger");
      if (!trigger || !window.Discourse || !Discourse.float || !Discourse.bookmarkModal || !Discourse.timeShortcutPicker) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      openBookmarkMenu(trigger);
    },
    true
  );

  // A post's bookmark saved or removed from its own menu: the footer
  // counts it.
  document.addEventListener("htmx:afterRequest", function (event) {
    var elt = event.detail && event.detail.elt;
    if (elt && elt.matches && elt.matches(".post-action-menu__bookmark") && event.detail.successful) {
      redraw([]);
    }
  });
})();
