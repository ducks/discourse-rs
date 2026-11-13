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
// recovering PUTs /t/:id/recover, Make Personal Message PUTs
// convert-topic/private and Reset Bump Date PUTs reset-bump-date; the
// page is then drawn again, where Ember re-renders it in place. Pinning
// and banners (FeatureTopic), Make Public (ConvertToPublicTopic), the
// timestamp (ChangeTimestamp), slow mode (EditSlowMode) and the topic
// timer (EditTopicTimer) open their modals
// (static/js/topic-admin-modals.js). The post menu is the menu
// service's (modalForMobile, autofocus); its Moderation History links
// work, and its actions are the topic controller's on the post its
// template's data-post describes: the staff color (PUT post_type),
// unhiding, locking, wiki and rebuilding the HTML PUT to the post, the
// notice opens ChangePostNoticeModal (PUT notice), and the post is drawn
// again after (GET /live/post/:id), where Ember re-renders it. What
// either opens that is not ported is refused, named.
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

  // One of topic-admin-modals.js's modals.
  function modals(name, topic) {
    if (Discourse.topicAdminModals) {
      Discourse.topicAdminModals[name](topic);
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
      unported("selecting posts (the topic's multi-select mode, 194c1)");
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
    // Topic#recover. Ember leaves a failure unhandled.
    "topic-admin-recover": function (topic) {
      request("PUT", "/t/" + topic.id + "/recover", { context: location.pathname })
        .then(function (r) {
          if (!r.ok) {
            throw new Error("/t/" + topic.id + "/recover: " + r.status);
          }
          location.reload();
        })
        .catch(function (e) {
          console.error(e);
        });
    },
    "topic-admin-close": function (topic) {
      toggleStatus(topic, "closed", true);
    },
    "topic-admin-open": function (topic) {
      toggleStatus(topic, "closed", false);
    },
    "topic-admin-pin": function (topic) {
      modals("featureTopic", topic);
    },
    "topic-admin-archive": function (topic) {
      toggleStatus(topic, "archived", !topic.archived);
    },
    "topic-admin-visible": function (topic) {
      toggleStatus(topic, "visible", !topic.visible);
    },
    "topic-admin-convert": function (topic, item) {
      if (item.querySelector(".d-icon-comment")) {
        modals("makePublic", topic);
        return;
      }
      request("PUT", "/t/" + topic.id + "/convert-topic/private").then(function (r) {
        if (!r.ok) {
          return popupError(r);
        }
        location.reload();
      });
    },
    "admin-topic-timer-update": function (topic) {
      modals("editTopicTimer", topic);
    },
    "topic-admin-change-timestamp": function (topic) {
      modals("changeTimestamp", topic);
    },
    // Topic#resetBumpDate: nothing on the page changes.
    "topic-admin-reset-bump-date": function (topic) {
      request("PUT", "/t/" + topic.id + "/reset-bump-date").then(function (r) {
        if (!r.ok) {
          popupError(r);
        }
      });
    },
    "topic-admin-slow-mode": function (topic) {
      modals("slowMode", topic);
    },
    "topic-admin-nested-replies": function () {
      unported("the nested replies view (PUT /n/:slug/:id/toggle, 194c1)");
    },
  };

  // The post admin menu's post, as its template has it: id, post_type,
  // wiki and the staff notice's text.
  function postData(template) {
    try {
      return JSON.parse(template.dataset.post);
    } catch (e) {
      return null;
    }
  }

  // scheduleRerender: the post drawn again as the viewer sees it now.
  function redrawPost(post) {
    if (window.htmx) {
      window.htmx.ajax("GET", base() + "/live/post/" + post.id, { swap: "none" });
    }
  }

  // Post#updatePostField: PUT /posts/:id/<field>, popupAjaxError on a
  // failure. jQuery leaves an undefined value out of the form.
  function updatePostField(post, field, value) {
    var form = {};
    if (value !== undefined) {
      form[field] = String(value);
    }
    return request("PUT", "/posts/" + post.id + "/" + field, form).then(function (r) {
      if (!r.ok) {
        return popupError(r).then(function () {
          return null;
        });
      }
      return r.json().catch(function () {
        return {};
      });
    });
  }

  function postAdminStrings() {
    var el = document.getElementById("post-admin-config");
    try {
      return (el && JSON.parse(el.textContent).strings) || {};
    } catch (e) {
      return {};
    }
  }

  function footerButton(className, label) {
    var button = document.createElement("button");
    button.className = "btn " + className;
    button.type = "button";
    button.innerHTML = '<span class="d-button-label"></span>';
    button.firstChild.textContent = label;
    return button;
  }

  // ChangePostNoticeModal: the notice's text, saved (disabled while it is
  // empty or unchanged) or deleted; the modal closes once the PUT is done.
  function changeNotice(post) {
    var s = postAdminStrings();
    // The post's notice (any kind), and a staff notice's text.
    var current = post.notice || null;
    var raw = current && typeof current.raw === "string" ? current.raw : null;
    return new Promise(function (resolve) {
      if (!Discourse.modal) {
        resolve();
        return;
      }
      var textarea;
      Discourse.modal.show({
        className: "change-post-notice-modal",
        title: current ? s.change_post_notice : s.add_post_notice,
        body: function (body) {
          var form = document.createElement("form");
          textarea = document.createElement("textarea");
          textarea.value = raw || "";
          form.appendChild(textarea);
          body.appendChild(form);
        },
        footer: function (footer, close) {
          var saving = false;
          var save = footerButton("btn-primary", s.save);
          var remove = current ? footerButton("btn-danger", s.delete_post_notice) : null;
          var cancel = footerButton("btn-flat d-modal-cancel", s.cancel);
          function refresh() {
            var value = textarea.value;
            save.disabled = saving || value === "" || value === raw;
            if (remove) {
              remove.disabled = saving;
            }
          }
          function setNotice(notice) {
            saving = true;
            save.firstChild.textContent = s.saving;
            refresh();
            updatePostField(post, "notice", notice).then(close, close);
          }
          textarea.addEventListener("input", refresh);
          save.addEventListener("click", function () {
            setNotice(textarea.value);
          });
          footer.appendChild(save);
          if (remove) {
            remove.addEventListener("click", function () {
              setNotice(undefined);
            });
            footer.appendChild(remove);
          }
          cancel.addEventListener("click", close);
          footer.appendChild(cancel);
          refresh();
        },
        onClose: resolve,
      });
    });
  }

  // The topic controller's post actions, by the button's class.
  var POST_ACTIONS = {
    // togglePostType: a moderator action back to regular, else one.
    "toggle-post-type": function (post) {
      return updatePostField(post, "post_type", post.post_type === 2 ? 1 : 2);
    },
    "add-notice": changeNotice,
    "change-notice": changeNotice,
    // Post#unhide: Ember leaves a failure unhandled.
    "unhide-post": function (post) {
      return request("PUT", "/posts/" + post.id + "/unhide").then(function (r) {
        if (!r.ok) {
          console.error("/posts/" + post.id + "/unhide: " + r.status);
        }
      });
    },
    "lock-post": function (post) {
      return updatePostField(post, "locked", true);
    },
    "unlock-post": function (post) {
      return updatePostField(post, "locked", false);
    },
    // toggleWiki
    wiki: function (post) {
      return updatePostField(post, "wiki", !post.wiki);
    },
    // Post#rebake
    "rebuild-html": function (post) {
      return request("PUT", "/posts/" + post.id + "/rebake").then(function (r) {
        if (!r.ok) {
          return popupError(r);
        }
      });
    },
  };

  // What the post admin menu opens that is not ported.
  var POST_UNPORTED = {
    "change-owner": "the change ownership modal (ChangePostOwner, 194d1)",
    "grant-badge": "the grant badge modal (GrantBadgeModal, 194d1)",
    "permanently-delete": "permanently deleting posts (force_destroy, 194d1)",
    "publish-page": "publishing pages (PublishPageModal, 194d1)",
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

  // TopicTimerInfo's buttons under the posts, for who may set timers:
  // the pencil opens EditTopicTimer (showTopicTimerModal), the trash can
  // is the topic controller's removeTopicTimer (TopicTimer.update with
  // the timer's type and no time), the page drawn again where Ember
  // empties the notice. popupAjaxError on a failure.
  document.addEventListener("click", function (event) {
    var button = event.target.closest("#topic .topic-timer-info .topic-timer-modify button");
    var topic = button && topicData();
    if (!topic) {
      return;
    }
    if (button.classList.contains("topic-timer-edit")) {
      modals("editTopicTimer", topic);
    } else if (button.classList.contains("topic-timer-remove") && topic.topic_timer) {
      request("POST", "/t/" + topic.id + "/timer", { time: "", status_type: topic.topic_timer.status_type })
        .then(function (r) {
          if (!r.ok) {
            return popupError(r);
          }
          location.reload();
        })
        .catch(function (e) {
          console.error(e);
        });
    }
  });

  // SlowModeInfo's button under the posts, for who may manage topics:
  // disableSlowMode (Topic.setSlowMode with no seconds), the page drawn
  // again where Ember empties the notice. popupAjaxError on a failure.
  document.addEventListener("click", function (event) {
    var button = event.target.closest("#topic .topic-status-info button.slow-mode-remove");
    var topic = button && topicData();
    if (!topic) {
      return;
    }
    request("PUT", "/t/" + topic.id + "/slow_mode", { seconds: "0" })
      .then(function (r) {
        if (!r.ok) {
          return popupError(r);
        }
        location.reload();
      })
      .catch(function (e) {
        console.error(e);
      });
  });

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
          var post = postData(postTemplate);
          if (!button || !post) {
            return;
          }
          var name;
          for (name in POST_UNPORTED) {
            if (button.classList.contains(name)) {
              unported(POST_UNPORTED[name]);
              return;
            }
          }
          for (name in POST_ACTIONS) {
            if (button.classList.contains(name)) {
              // topicAction: the action, then the post drawn again.
              Promise.resolve(POST_ACTIONS[name](post))
                .catch(function (e) {
                  console.error(e);
                })
                .then(function () {
                  redrawPost(post);
                });
              return;
            }
          }
        }
      );
    }
  });
})();
