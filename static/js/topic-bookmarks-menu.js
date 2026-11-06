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
// Editing a bookmark (the topic's or a post's) opens BookmarkModal, which
// is not ported (196): refused, named.
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

  function unported(what) {
    var notice = document.getElementById("page-error");
    if (notice) {
      notice.textContent = "Not ported yet: " + what + ".";
      notice.hidden = false;
    }
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
            unported("editing a bookmark (BookmarkModal, 196)");
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
            unported("editing a bookmark (BookmarkModal, 196)");
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

  // A post's bookmark saved or removed from its own menu: the footer
  // counts it.
  document.addEventListener("htmx:afterRequest", function (event) {
    var elt = event.detail && event.detail.elt;
    if (elt && elt.matches && elt.matches(".post-action-menu__bookmark") && event.detail.successful) {
      redraw([]);
    }
  });
})();
