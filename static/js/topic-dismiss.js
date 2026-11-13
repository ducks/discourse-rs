// TopicDismissButtons on the new and unread lists (html::dismiss_buttons
// draws them at the top and, past five topics, at the bottom), with the
// discovery controller's actions:
//
//   new:    the button is resetNew (Topic.resetNew, PUT /topics/reset-new)
//           with the subset's options under unified new (dismiss topics
//           unless the subset is replies, posts unless it is topics);
//           its menu's "Dismiss and stop tracking" adds untrack.
//   unread: "Dismiss…" opens DismissRead, whose Dismiss sends dismissRead
//           (Topic.bulkOperationByFilter, PUT /topics/bulk filter=unread):
//           dismiss_posts, or with "Stop tracking these topics" checked a
//           change to the regular notification level.
//
// Both send `tracked` from the list's f (or filter) query param and the
// list's category and tag from the row, then refresh the page where it
// stands (router.refresh), which redraws the list, the nav pills and
// the sidebar's counts. A failure is left unhandled as Ember leaves it.
//
// With topics selected in the list's bulk select mode
// (topic-list-bulk-select.js, @selectedTopics) the buttons carry the
// count ("Dismiss (2)"), resetNew sends the topics as topic_ids and
// dismissRead is Topic.bulkOperation on them instead of the filter. The
// Bulk Actions menu's Dismiss is these (Discourse.topicDismiss).
//
// A member's message lists (user-topics-list.gjs; messages::view draws
// the rows with the route's inbox, and the strings in
// #topic-dismiss-strings): the new list's button is the controller's
// resetNew (Topic.pmResetNew, PUT /topics/pm-reset-new with the inbox
// and a group inbox's group), which refreshes only when something was
// dismissed; the unread list's DismissRead sends the route's
// dismissReadTopics (private_message_inbox, and group_name).
(function () {
  "use strict";

  if (!Discourse.once("topic-dismiss")) {
    return;
  }

  var script = document.currentScript;
  var strings = {};
  try {
    var element = document.getElementById("topic-dismiss-strings");
    strings = JSON.parse(script.dataset.strings || (element && element.textContent) || "{}");
  } catch (_) {}

  function t(key) {
    return strings[key] || key;
  }

  function counted(key, count) {
    var text = (count === 1 && strings[key + ".one"]) || strings[key + ".other"] || key;
    return text.split("%{count}").join(String(count));
  }

  // BulkSelectHelper#selected, as ids.
  function selectedTopics() {
    return Discourse.bulkSelectTopics ? Discourse.bulkSelectTopics.selected() : [];
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function headers() {
    var h = {};
    try {
      h = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (_) {}
    h["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
    h.Accept = "application/json, text/javascript, */*; q=0.01";
    return h;
  }

  function tracked() {
    var params = new URLSearchParams(location.search);
    return (params.get("f") || params.get("filter")) === "tracked";
  }

  function put(path, data) {
    var body = new URLSearchParams();
    data.forEach(function (pair) {
      body.append(pair[0], pair[1]);
    });
    return fetch(basePath() + path, {
      method: "PUT",
      headers: headers(),
      credentials: "same-origin",
      body: body.toString(),
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(path + ": " + r.status);
      }
      return r.json();
    });
  }

  function refresh() {
    if (Discourse.refresh) {
      Discourse.refresh();
    } else {
      location.reload();
    }
  }

  // Topic.resetNew's data, in its order.
  function resetNew(row, untrack) {
    var data = [["tracked", String(tracked())]];
    if (row.dataset.categoryId) {
      data.push(["category_id", row.dataset.categoryId]);
      data.push(["include_subcategories", String(!row.hasAttribute("data-no-subcategories"))]);
    }
    // A tag's new list (the discovery model's tag).
    if (row.dataset.tagName) {
      data.push(["tag_name", row.dataset.tagName]);
    }
    selectedTopics().forEach(function (id) {
      data.push(["topic_ids[]", String(id)]);
    });
    // The combo is drawn under unified new only; the classic button
    // dismisses with no options.
    if (row.querySelector(".topic-dismiss-buttons__combo")) {
      var subset = new URLSearchParams(location.search).get("subset");
      if (subset !== "topics") {
        data.push(["dismiss_posts", "true"]);
      }
      if (subset !== "replies") {
        data.push(["dismiss_topics", "true"]);
      }
      if (untrack) {
        data.push(["untrack", "true"]);
      }
    }
    return put("/topics/reset-new", data).then(refresh);
  }

  // Topic.pmResetNew's data: the controller's selection is an empty
  // list here, which jQuery leaves out.
  function pmResetNew(row) {
    var data = [["inbox", row.dataset.pmInbox]];
    if (row.dataset.groupName) {
      data.push(["group_name", row.dataset.groupName]);
    }
    return put("/topics/pm-reset-new", data).then(function (result) {
      if (result && result.topic_ids && result.topic_ids.length > 0) {
        refresh();
      }
    });
  }

  // Topic.bulkOperationByFilter("unread", operation, options, isTracked),
  // or with topics selected Topic.bulkOperation(selected, operation, {},
  // isTracked).
  function dismissRead(row, stopTracking) {
    var ids = selectedTopics();
    var data = ids.length
      ? ids.map(function (id) {
          return ["topic_ids[]", String(id)];
        })
      : [["filter", "unread"]];
    if (stopTracking) {
      data.push(["operation[type]", "change_notification_level"]);
      data.push(["operation[notification_level_id]", "1"]);
    } else {
      data.push(["operation[type]", "dismiss_posts"]);
    }
    data.push(["tracked", String(tracked())]);
    if (ids.length) {
      return put("/topics/bulk", data);
    }
    // A message list's options are the inbox alone.
    if (row.dataset.pmInbox) {
      data.push(["private_message_inbox", row.dataset.pmInbox]);
      if (row.dataset.groupName) {
        data.push(["group_name", row.dataset.groupName]);
      }
      return put("/topics/bulk", data);
    }
    if (row.dataset.categoryId) {
      data.push(["category_id", row.dataset.categoryId]);
    }
    if (!row.hasAttribute("data-no-subcategories")) {
      data.push(["include_subcategories", "true"]);
    }
    // A tag's unread list. Ember passes the tag model's id here
    // (tagName: this.model.tag?.id), the tag's name until tags took
    // numeric ids; the endpoint matches tags.name, so the name is sent.
    if (row.dataset.tagName) {
      data.push(["tag_name", row.dataset.tagName]);
    }
    return put("/topics/bulk", data);
  }

  // DismissRead: its PreferenceCheckbox (a div in the body's paragraph,
  // as Ember inserts it) and the Dismiss button, which closes the modal
  // once the dismissal is done (BulkSelectHelper#dismissRead).
  function openDismissRead(row) {
    var icon = Discourse.cooked ? Discourse.cooked.icon : function () { return ""; };
    var count = selectedTopics().length;
    Discourse.modal.show({
      className: "dismiss-read-modal",
      title: count ? counted("topics.bulk.dismiss_read_with_selected", count) : t("topics.bulk.dismiss_read"),
      body: function (body) {
        var p = document.createElement("p");
        var controls = document.createElement("div");
        controls.className = "controls dismiss-read-modal__stop-tracking";
        var label = document.createElement("label");
        label.className = "checkbox-label";
        var input = document.createElement("input");
        input.className = "ember-checkbox";
        input.type = "checkbox";
        label.appendChild(input);
        label.appendChild(document.createTextNode(" " + t("topics.bulk.also_dismiss_topics")));
        controls.appendChild(label);
        p.appendChild(controls);
        body.appendChild(p);
      },
      footer: function (footer, close) {
        var button = document.createElement("button");
        button.className = "btn btn-icon-text btn-primary";
        button.id = "dismiss-read-confirm";
        button.type = "button";
        button.innerHTML = icon("check") + '<span class="d-button-label"></span>';
        button.querySelector(".d-button-label").textContent = t("topics.bulk.dismiss");
        button.addEventListener("click", function () {
          var stop = footer.parentNode.querySelector(".dismiss-read-modal__stop-tracking input").checked;
          dismissRead(row, stop).then(function () {
            close();
            refresh();
          });
        });
        footer.appendChild(button);
      },
    });
  }

  // The combo's DMenu, a bottom sheet in the mobile view.
  function openMenu(trigger, row) {
    var content = document.createElement("ul");
    content.className = "dropdown-menu";
    content.innerHTML =
      '<li class="dropdown-menu__item topic-dismiss-buttons__menu-item"><button class="btn dismiss-new-stop-tracking" type="button"><span class="d-button-label"></span></button></li>';
    content.querySelector(".d-button-label").textContent = t("topics.bulk.dismiss_and_stop_tracking");
    content.querySelector(".dismiss-new-stop-tracking").addEventListener("click", function () {
      Discourse.float.close("dismiss-new-menu");
      resetNew(row, true);
    });
    // DMenu marks its trigger -expanded, after its own class.
    trigger.setAttribute("aria-expanded", "true");
    trigger.className = trigger.className.replace("fk-d-menu__trigger ", "fk-d-menu__trigger -expanded ");
    Discourse.float.menu(trigger, {
      identifier: "dismiss-new-menu",
      mainClass: "dismiss-new-menu-content",
      // DComboButton's menu.
      placement: "bottom-end",
      modalForMobile: true,
      content: content,
      onClose: function () {
        trigger.setAttribute("aria-expanded", "false");
        trigger.classList.remove("-expanded");
      },
    });
  }

  // TopicDismissButtons#dismissLabel and #dismissNewLabel: the label
  // the page drew, or with topics selected its `_with_selected` form.
  function label(button) {
    var id = button.id || "";
    if (id.indexOf("dismiss-topics-") === 0) {
      return "topics.bulk.dismiss_button";
    }
    if (id.indexOf("dismiss-new-menu-") === 0 || id.indexOf("dismiss-new-") !== 0) {
      return null;
    }
    if (!button.closest(".topic-dismiss-buttons__combo")) {
      return "topics.bulk.dismiss_new";
    }
    var subset = new URLSearchParams(location.search).get("subset");
    return subset === "topics"
      ? "topics.bulk.dismiss_new_topics"
      : subset === "replies"
        ? "topics.bulk.dismiss_new_replies"
        : "topics.bulk.dismiss_all";
  }

  document.addEventListener("discourse:bulk-select-topics", function () {
    var count = selectedTopics().length;
    document.querySelectorAll(".dismiss-container-top button, .dismiss-container-bottom button").forEach(function (button) {
      var key = label(button);
      var text = button.querySelector(".d-button-label");
      if (!key || !text) {
        return;
      }
      if (button.dataset.label === undefined) {
        button.dataset.label = text.textContent;
      }
      text.textContent = count ? counted(key + "_with_selected", count) : button.dataset.label;
    });
  });

  // The list's own dismissal, for the Bulk Actions menu's Dismiss
  // (BulkSelectHelper's onResetNew, the dropdown's dismiss-unread).
  Discourse.topicDismiss = {
    resetNew: function () {
      var row = document.querySelector(".dismiss-container-top");
      if (!row) {
        return null;
      }
      return row.dataset.pmInbox ? pmResetNew(row) : resetNew(row, false);
    },
    dismissRead: function () {
      var row = document.querySelector(".dismiss-container-top");
      if (row) {
        openDismissRead(row);
      }
    },
  };

  document.addEventListener("click", function (event) {
    var button = event.target.closest(
      ".dismiss-container-top button, .dismiss-container-bottom button"
    );
    if (!button || button.disabled) {
      return;
    }
    var row = button.closest(".dismiss-container-top, .dismiss-container-bottom");
    var id = button.id || "";
    if (id.indexOf("dismiss-new-menu-") === 0) {
      openMenu(button, row);
    } else if (id.indexOf("dismiss-new-") === 0) {
      if (row.dataset.pmInbox) {
        pmResetNew(row);
      } else {
        resetNew(row, false);
      }
    } else if (id.indexOf("dismiss-topics-") === 0) {
      openDismissRead(row);
    }
  });
})();
