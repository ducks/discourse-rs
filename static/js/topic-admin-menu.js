// The topic page's staff menus: TopicAdminMenu (components/
// topic-admin-menu.gjs), the wrench in the footer, the timeline's
// controls and the progress bar, and AdminPostMenu (admin-post-menu.gjs),
// a post's wrench. Their contents are the server's, in templates: the
// page's template.topic-admin-menu-template (its data-topic the topic's
// state) and each post's template.admin-post-menu-content.
//
// The topic menu is a DMenu (modalForMobile, autofocus): closing and
// opening, archiving, listing and unlisting PUT /t/:id/status as
// Topic#toggleStatus does, deleting DELETEs /t/:id (asking first through
// DeleteTopicConfirm for a topic past min_topic_views_for_delete_confirm),
// and Make Personal Message PUTs convert-topic/private; the page is then
// drawn again, where Ember re-renders it in place. The post menu is the
// menu service's (modalForMobile, autofocus); its Moderation History links
// work. What either opens that is not ported is refused, named.
(function () {
  "use strict";

  if (!Discourse.once("topic-admin-menu")) {
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

  function topicData() {
    var template = document.querySelector("template.topic-admin-menu-template");
    if (!template) {
      return null;
    }
    try {
      return JSON.parse(template.dataset.topic);
    } catch (e) {
      return null;
    }
  }

  // Topic#saveStatus. Ember leaves a failure unhandled.
  function toggleStatus(topic, status, enabled) {
    request("PUT", "/t/" + topic.id + "/status", { status: status, enabled: String(enabled) }).then(
      function (r) {
        if (!r.ok) {
          throw new Error("/t/" + topic.id + "/status: " + r.status);
        }
        location.reload();
      }
    ).catch(function (e) {
      console.error(e);
    });
  }

  // Topic#destroy: staff (and who moderates its category or may delete
  // every post) stay on the topic, now deleted; others go home.
  function destroyTopic(topic) {
    return request("DELETE", "/t/" + topic.id, { context: location.pathname }).then(function (r) {
      if (!r.ok) {
        return Promise.reject(r);
      }
      if (topic.stayAfterDelete) {
        location.reload();
      } else {
        location.href = base() + "/";
      }
    });
  }

  // DeleteTopicConfirm: its flash on a failure, the Yes button disabled
  // and labelled Deleting while it goes.
  function deleteTopicConfirm(topic) {
    var s = topic.deleteConfirm;
    Discourse.modal.show({
      className: "delete-topic-confirm-modal",
      title: s.title,
      body: function (body) {
        var p = document.createElement("p");
        p.textContent = s.body;
        body.appendChild(p);
      },
      footer: function (footer, close) {
        var yes = document.createElement("button");
        yes.className = "btn btn-text btn-danger";
        yes.type = "button";
        yes.innerHTML = '<span class="d-button-label"></span>';
        yes.firstChild.textContent = s.yes;
        var no = document.createElement("button");
        no.className = "btn btn-text btn-primary";
        no.type = "button";
        no.innerHTML = '<span class="d-button-label"></span>';
        no.firstChild.textContent = s.no;
        footer.appendChild(yes);
        footer.appendChild(no);
        no.addEventListener("click", close);
        yes.addEventListener("click", function () {
          yes.disabled = true;
          yes.firstChild.textContent = s.deleting;
          destroyTopic(topic).catch(function () {
            var modal = footer.closest(".d-modal");
            var flash = modal.querySelector("#modal-alert");
            if (!flash) {
              flash = document.createElement("div");
              flash.id = "modal-alert";
              flash.setAttribute("role", "alert");
              flash.className = "alert alert-error";
              var body = modal.querySelector(".d-modal__body");
              body.parentNode.insertBefore(flash, body);
            }
            flash.textContent = s.error;
            yes.disabled = false;
            yes.firstChild.textContent = s.yes;
          });
        });
      },
    });
  }

  // The topic controller's actions, by the item's class.
  var TOPIC_ACTIONS = {
    "topic-admin-multi-select": function () {
      unported("selecting posts (the topic's multi-select mode)");
    },
    "topic-admin-delete": function (topic) {
      if (topic.deleteConfirm && Discourse.modal) {
        deleteTopicConfirm(topic);
        return;
      }
      destroyTopic(topic).catch(function (r) {
        if (r instanceof Response) {
          popupError(r);
        } else {
          console.error(r);
        }
      });
    },
    "topic-admin-recover": function () {
      unported("recovering topics (PUT /t/:id/recover)");
    },
    "topic-admin-close": function (topic) {
      toggleStatus(topic, "closed", true);
    },
    "topic-admin-open": function (topic) {
      toggleStatus(topic, "closed", false);
    },
    "topic-admin-pin": function () {
      unported("the feature topic modal (FeatureTopic: pinning, unpinning and banners)");
    },
    "topic-admin-archive": function (topic) {
      toggleStatus(topic, "archived", !topic.archived);
    },
    "topic-admin-visible": function (topic) {
      toggleStatus(topic, "visible", !topic.visible);
    },
    "topic-admin-convert": function (topic, item) {
      if (item.querySelector(".d-icon-comment")) {
        unported("the make public topic modal (ConvertToPublicTopic)");
        return;
      }
      request("PUT", "/t/" + topic.id + "/convert-topic/private").then(function (r) {
        if (!r.ok) {
          return popupError(r);
        }
        location.reload();
      });
    },
    "admin-topic-timer-update": function () {
      unported("the topic timer modal (EditTopicTimer)");
    },
    "topic-admin-change-timestamp": function () {
      unported("changing a topic's timestamp (ChangeTimestamp, PUT /t/:id/change-timestamp)");
    },
    "topic-admin-reset-bump-date": function () {
      unported("resetting a topic's bump date (PUT /t/:id/reset-bump-date)");
    },
    "topic-admin-slow-mode": function () {
      unported("the slow mode modal (EditSlowMode)");
    },
    "topic-admin-nested-replies": function () {
      unported("the nested replies view (PUT /n/:slug/:id/toggle)");
    },
  };

  // AdminPostMenu's actions, by the button's class.
  var POST_ACTIONS = {
    "toggle-post-type": "changing a post's type (PUT /posts/:id/post_type)",
    "add-notice": "the post notice modal (ChangePostNoticeModal, PUT /posts/:id/notice)",
    "change-notice": "the post notice modal (ChangePostNoticeModal, PUT /posts/:id/notice)",
    "unhide-post": "unhiding a post (PUT /posts/:id/unhide)",
    "change-owner": "the change ownership modal (ChangePostOwner)",
    "grant-badge": "the grant badge modal (GrantBadgeModal)",
    "lock-post": "locking a post (PUT /posts/:id/locked)",
    "unlock-post": "unlocking a post (PUT /posts/:id/locked)",
    "permanently-delete": "permanently deleting posts (force_destroy)",
    wiki: "wiki posts (PUT /posts/:id/wiki)",
    "publish-page": "publishing pages (PublishPageModal)",
    "rebuild-html": "rebuilding a post's HTML (PUT /posts/:id/rebake)",
  };

  var openMenu = null;

  function toggle(trigger, options, onItem) {
    if (openMenu && openMenu.expanded) {
      var same = openMenu.trigger === trigger;
      openMenu.close({ focusTrigger: false });
      openMenu = null;
      if (same) {
        return;
      }
    }
    var menu = Discourse.float.menu(trigger, options);
    menu.trigger = trigger;
    openMenu = menu;
    options.content.addEventListener("click", function (event) {
      var item = event.target.closest(".dropdown-menu__item");
      if (!item) {
        return;
      }
      // A link goes where it points, the menu closing behind it.
      if (event.target.closest("a[href]")) {
        menu.close({ focusTrigger: false });
        return;
      }
      // onButtonAction: the menu closes first.
      Promise.resolve(menu.close({ focusTrigger: false })).then(function () {
        onItem(item);
      });
    });
  }

  document.addEventListener("click", function (event) {
    if (!window.Discourse || !Discourse.float) {
      return;
    }
    var topicTrigger = event.target.closest(".topic-admin-menu-trigger");
    if (topicTrigger) {
      var template = document.querySelector("template.topic-admin-menu-template");
      var topic = topicData();
      if (!template || !topic) {
        return;
      }
      toggle(
        topicTrigger,
        {
          identifier: "topic-admin-menu",
          modalForMobile: true,
          autofocus: true,
          content: template.content.cloneNode(true).firstElementChild,
        },
        function (item) {
          for (var name in TOPIC_ACTIONS) {
            if (item.classList.contains(name)) {
              TOPIC_ACTIONS[name](topic, item);
              return;
            }
          }
        }
      );
      return;
    }
    var postTrigger = event.target.closest(".show-post-admin-menu");
    if (postTrigger) {
      var nav = postTrigger.closest("nav.post-controls");
      var postTemplate = nav && nav.querySelector("template.admin-post-menu-content");
      if (!postTemplate) {
        return;
      }
      toggle(
        postTrigger,
        {
          identifier: "admin-post-menu",
          modalForMobile: true,
          autofocus: true,
          content: postTemplate.content.cloneNode(true).firstElementChild,
        },
        function (item) {
          var button = item.querySelector("button");
          if (!button) {
            return;
          }
          for (var name in POST_ACTIONS) {
            if (button.classList.contains(name)) {
              unported(POST_ACTIONS[name]);
              return;
            }
          }
        }
      );
    }
  });
})();
