// The review queue's page (review.index, src/review_view.rs draws it):
// ReviewFilters' Refresh (ReviewIndexController#refresh's query params)
// and Options (the secondary filters, kept while hidden), each
// reviewable's moderator actions (ReviewableBundledAction and
// ReviewableItem#perform: the reviewable redrawn from the server after
// the action, its completed message a toast), the permalink copy, the
// note form's counter, and the watched words its scores name
// highlighted in the post and title (highlightHTML).
//
// What leans on parts not ported says so instead: claiming (which a
// confirmation or a client action claims through first), the client
// actions (edit, suspend, silence), the reject reason and revise modals,
// notes, the insights tab, editing and the pin toggle.
(function () {
  "use strict";

  if (!Discourse.once("review-page")) {
    return;
  }

  var QUERY_PARAMS = [
    "priority",
    "type",
    "status",
    "category_id",
    "topic_id",
    "username",
    "reviewed_by",
    "claimed_by",
    "from_date",
    "to_date",
    "sort_order",
    "additional_filters",
    "flagged_by",
    "score_type",
  ];

  function pageData() {
    var el = document.getElementById("review-page-data");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      return {};
    }
  }

  function ui() {
    return Discourse.adminUi;
  }

  function notPorted(what) {
    Discourse.dialog.alert("Not ported yet: " + what + ".");
  }

  function dasherize(s) {
    return String(s || "")
      .replace(/([a-z\d])([A-Z])/g, "$1-$2")
      .replace(/[_\s]+/g, "-")
      .toLowerCase();
  }

  // ---- Filters ----------------------------------------------------------

  // The secondary filters while collapsed: kept out of the page so the
  // values chosen come back with them.
  var stash = null;

  function filtersRoot() {
    return document.querySelector(".reviewable-filters");
  }

  // Where the secondary filters are: the filters while expanded, else
  // the stash, else the template they start from.
  function secondaryScope() {
    var root = filtersRoot();
    if (root && root.classList.contains("--expanded")) {
      return root;
    }
    var template = document.getElementById("review-secondary-filters");
    return stash || (template ? template.content : null);
  }

  function secondary(selector) {
    var scope = secondaryScope();
    return scope ? scope.querySelector(selector) : null;
  }

  // ReviewFilters' controls by their place: status and type first; then
  // the score type and priority ComboBoxes (the category and sort order
  // ones named by their filter), the flagged by and reviewed by choosers
  // (the claimed by and user ones named).
  function primaryCombo(index) {
    var filters = filtersRoot().querySelectorAll(":scope > .reviewable-filter");
    return filters[index] ? filters[index].querySelector("details.combo-box") : null;
  }

  function secondaryCombos() {
    var scope = secondaryScope();
    if (!scope) {
      return [];
    }
    return Array.prototype.filter.call(
      scope.querySelectorAll(".reviewable-filter:not(.sort-order) > details.combo-box"),
      function (d) {
        return !d.classList.contains("category-chooser") && d !== primaryCombo(0) && d !== primaryCombo(1);
      }
    );
  }

  function secondaryChoosers() {
    var scope = secondaryScope();
    if (!scope) {
      return [];
    }
    return Array.prototype.slice.call(
      scope.querySelectorAll(
        ".reviewable-filter.topic-filter:not(.claimed-by) > details.email-group-user-chooser:not(.user-selector)"
      )
    );
  }

  // A ComboBox's value: what was chosen, else what it was drawn with.
  function comboValue(details) {
    if (!details) {
      return null;
    }
    if ("value" in details.dataset) {
      return details.dataset.value || null;
    }
    var summary = details.querySelector("summary");
    return (summary && summary.dataset.value) || null;
  }

  function chooserValue(details) {
    return details ? details.dataset.value || null : null;
  }

  function dateValue(selector) {
    var input = secondary(selector);
    return input && input.value ? input.value : null;
  }

  function currentParams() {
    return new URLSearchParams(window.location.search);
  }

  function go(values) {
    var data = pageData();
    var defaults = { status: "pending", priority: data.default_priority };
    var params = new URLSearchParams();
    QUERY_PARAMS.forEach(function (key) {
      var value = values[key];
      if (value === null || value === undefined || value === "" || value === defaults[key]) {
        return;
      }
      params.set(key, value);
    });
    var query = params.toString();
    window.location.href = ui().base() + "/review" + (query ? "?" + query : "");
  }

  // ReviewIndexController#refresh
  function refresh() {
    var current = currentParams();
    var currentStatus = current.get("status") || "pending";
    var nextStatus = comboValue(primaryCombo(0)) || "pending";
    var currentOrder = (pageData().filters || {}).sort_order;
    var nextOrder = comboValue(secondary(".sort-order > details.combo-box"));
    var createdAtStatuses = ["reviewed", "all"];
    var priorityStatuses = ["approved", "rejected", "deleted", "ignored", "pending"];
    if (
      createdAtStatuses.indexOf(currentStatus) !== -1 &&
      currentOrder === "created_at" &&
      priorityStatuses.indexOf(nextStatus) !== -1 &&
      nextOrder === "created_at"
    ) {
      nextOrder = "score";
    }
    if (
      priorityStatuses.indexOf(currentStatus) !== -1 &&
      currentOrder === "score" &&
      createdAtStatuses.indexOf(nextStatus) !== -1 &&
      nextOrder === "score"
    ) {
      nextOrder = "created_at";
    }
    var combos = secondaryCombos();
    var choosers = secondaryChoosers();
    go({
      type: comboValue(primaryCombo(1)),
      priority: comboValue(combos[1]),
      status: nextStatus,
      category_id: comboValue(secondary("details.category-chooser")),
      topic_id: current.get("topic_id"),
      username: chooserValue(secondary("details.user-selector")),
      reviewed_by: chooserValue(choosers[1]),
      claimed_by: chooserValue(secondary(".claimed-by > details.email-group-user-chooser")),
      flagged_by: chooserValue(choosers[0]),
      score_type: comboValue(combos[0]),
      from_date: dateValue(".d-date-time-input.from .date-picker"),
      to_date: dateValue(".d-date-time-input.to .date-picker"),
      sort_order: nextOrder,
      additional_filters: "{}",
    });
  }

  // resetTopic: the same params without the topic.
  function resetTopic() {
    var values = {};
    var current = currentParams();
    QUERY_PARAMS.forEach(function (key) {
      values[key] = current.get(key);
    });
    values.topic_id = null;
    go(values);
  }

  function toggleFilters(button) {
    var root = filtersRoot();
    var actions = root.querySelector(".reviewable-filters-actions");
    var expanded = root.classList.contains("--expanded");
    if (expanded) {
      stash = document.createDocumentFragment();
      Array.prototype.slice.call(root.children).forEach(function (child, i) {
        if (i >= 2 && child !== actions) {
          stash.appendChild(child);
        }
      });
    } else {
      var content = stash || document.getElementById("review-secondary-filters").content.cloneNode(true);
      stash = null;
      root.insertBefore(content, actions);
    }
    root.classList.toggle("--expanded", !expanded);
    root.classList.toggle("--collapsed", expanded);
    var svg = button.querySelector(".d-icon");
    var name = expanded ? "chevron-down" : "chevron-up";
    if (svg) {
      svg.setAttribute("class", "fa d-icon d-icon-" + name + " svg-icon fa-width-auto svg-string");
      svg.querySelector("use").setAttribute("href", "#" + name);
    }
  }

  // ---- Actions ----------------------------------------------------------

  function itemOf(el) {
    return el.closest(".review-item[data-reviewable-id]");
  }

  function reviewableData(item) {
    return ((pageData().reviewables || {})[item.dataset.reviewableId]) || {};
  }

  function actionByName(item, name) {
    var actions = reviewableData(item).actions || {};
    var found = null;
    Object.keys(actions).forEach(function (id) {
      if (actions[id].action_name === name) {
        found = actions[id];
      }
    });
    return found;
  }

  // A single action's button: the action whose dasherized name it carries.
  function actionOfButton(item, button) {
    var actions = reviewableData(item).actions || {};
    var found = null;
    Object.keys(actions).forEach(function (id) {
      if (button.classList.contains(dasherize(actions[id].action_name))) {
        found = actions[id];
      }
    });
    return found;
  }

  function setDisabled(item, disabled) {
    item.querySelectorAll(".review-item__aside .reviewable-action, .review-item__aside summary").forEach(function (el) {
      if (el.tagName === "BUTTON") {
        el.disabled = disabled;
      } else {
        el.closest("details").classList.toggle("is-disabled", disabled);
      }
    });
  }

  // The reviewable as the server draws it now (store.find), with any
  // other reviewables the action updated: the queue of its topic, every
  // status.
  function redraw(item, ids) {
    var data = reviewableData(item);
    var params = new URLSearchParams({ priority: "any", status: "all" });
    if (data.topic_id) {
      params.set("topic_id", data.topic_id);
    }
    return fetch(ui().base() + "/review?" + params.toString(), {
      credentials: "same-origin",
      headers: { Accept: "text/html" },
    })
      .then(function (response) {
        return response.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var fresh = pageData();
        var incoming = doc.getElementById("review-page-data");
        var incomingData = {};
        try {
          incomingData = incoming ? JSON.parse(incoming.textContent) : {};
        } catch (_) {
          incomingData = {};
        }
        ids.forEach(function (id) {
          var mine = document.querySelector('.review-item[data-reviewable-id="' + id + '"]');
          var theirs = doc.querySelector('.review-item[data-reviewable-id="' + id + '"]');
          if (!mine || !theirs) {
            return;
          }
          mine.closest(".review-container").replaceWith(document.importNode(theirs.closest(".review-container"), true));
          if (incomingData.reviewables && incomingData.reviewables[id]) {
            fresh.reviewables[id] = incomingData.reviewables[id];
          }
        });
        document.getElementById("review-page-data").textContent = JSON.stringify(fresh);
        highlightAll();
      });
  }

  // ReviewableItem#_performConfirmed, without a client action.
  function performConfirmed(item, action) {
    var data = reviewableData(item);
    var id = item.dataset.reviewableId;
    setDisabled(item, true);
    ui()
      .ajax(
        "PUT",
        "/review/" + id + "/perform/" + encodeURIComponent(action.server_action) + "?version=" + data.version
      )
      .then(function (json) {
        var result = (json && json.reviewable_perform_result) || {};
        if (action.completed_message) {
          ui().toast(action.completed_message);
        }
        var ids = [id];
        Object.keys(result.reviewable_updates || {}).forEach(function (other) {
          if (ids.indexOf(String(other)) === -1) {
            ids.push(String(other));
          }
        });
        return redraw(item, ids);
      })
      .catch(function (error) {
        setDisabled(item, false);
        ui().failed(error);
      });
  }

  // ReviewableItem#perform
  function perform(item, action) {
    if (!action) {
      return;
    }
    // A confirmation, a modal or a client action claims the reviewable's
    // topic first (#claimReviewable).
    if (action.confirm_message) {
      notPorted("claiming a reviewable before its confirmation");
      return;
    }
    if (action.require_reject_reason) {
      notPorted("the reject reason modal (RejectReasonReviewableModal)");
      return;
    }
    if (action.server_action === "revise_and_reject_post") {
      notPorted("the revise and reject modal (ReviseAndRejectPostReviewable)");
      return;
    }
    if (action.client_action) {
      notPorted("the review queue's " + action.client_action + " action");
      return;
    }
    performConfirmed(item, action);
  }

  // DropdownSelectBox: its rows drawn as it opens.
  document.addEventListener(
    "toggle",
    function (event) {
      var details = event.target;
      if (!details.matches || !details.matches(".review-item details.reviewable-action-dropdown")) {
        return;
      }
      details.classList.toggle("is-expanded", details.open);
      var body = details.querySelector(".select-kit-body");
      var caret = details.querySelector("summary .angle-icon");
      if (caret) {
        var name = details.open ? "angle-up" : "angle-down";
        caret.setAttribute("class", "fa d-icon d-icon-" + name + " svg-icon fa-width-auto angle-icon svg-string");
        caret.querySelector("use").setAttribute("href", "#" + name);
      }
      if (!details.open) {
        body.innerHTML = "";
        body.removeAttribute("style");
        return;
      }
      if (details.classList.contains("is-disabled")) {
        details.open = false;
        return;
      }
      body.innerHTML =
        '<div class="select-kit-filter"></div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
        details.querySelector("template.select-kit-rows").innerHTML +
        "</ul>";
      ui().floatBody(details);
    },
    true
  );

  // ---- Clicks -----------------------------------------------------------

  document.addEventListener("click", function (event) {
    var target = event.target;
    if (!target.closest || !target.closest(".reviewable")) {
      return;
    }

    if (target.closest(".reviewable-filters .refresh")) {
      refresh();
      return;
    }
    var expand = target.closest(".reviewable-filters .expand-secondary-filters");
    if (expand) {
      toggleFilters(expand);
      return;
    }
    if (target.closest(".reviewable-filters .topic-filter > button")) {
      resetTopic();
      return;
    }

    var item = itemOf(target);
    if (!item) {
      return;
    }

    var row = target.closest("details.reviewable-action-dropdown .select-kit-row");
    if (row) {
      var details = row.closest("details");
      details.open = false;
      perform(item, actionByName(item, row.dataset.value));
      return;
    }
    // Close an open action menu on a click elsewhere.
    document.querySelectorAll(".review-item details.reviewable-action-dropdown[open]").forEach(function (d) {
      if (!d.contains(target)) {
        d.open = false;
      }
    });

    var button = target.closest(".review-item__moderator-actions button.reviewable-action");
    if (button && !button.disabled) {
      if (button.classList.contains("edit")) {
        notPorted("editing a reviewable");
        return;
      }
      perform(item, actionOfButton(item, button));
      return;
    }

    if (target.closest(".reviewable-claimed-topic button")) {
      notPorted("claiming a topic in the review queue");
      return;
    }

    var copy = target.closest(".reviewable-permalink-copy");
    if (copy) {
      copyPermalink(item, copy, event);
      return;
    }

    var tab = target.closest(".review-item__insights .d-nav-submenu__tabs a");
    if (tab) {
      event.preventDefault();
      if (tab.closest("li").classList.contains("insights")) {
        notPorted("the review queue's insights");
      }
      return;
    }

    var pin = target.closest(".pin-toggle-button");
    if (pin) {
      event.preventDefault();
      notPorted("pinning a topic for yourself from the review queue");
    }
  });

  // ReviewableItem#copyPermalink and showAlert's feedback.
  function copyPermalink(item, button, event) {
    var url = window.location.origin + ui().base() + "/review/" + item.dataset.reviewableId;
    if (event.metaKey || event.ctrlKey || event.button === 1) {
      window.open(url, "_blank");
      return;
    }
    if (!navigator.clipboard) {
      return;
    }
    navigator.clipboard.writeText(url).then(function () {
      button.classList.add("post-action-feedback-button");
      var alert = document.createElement("div");
      alert.className = "post-action-feedback-alert -success";
      alert.textContent = (pageData().strings || {}).copy_link_feedback || "";
      button.appendChild(alert);
      setTimeout(function () {
        alert.remove();
      }, 2500);
    });
  }

  // The note form: FormKit's counter as it is typed; adding a note goes
  // to ReviewableNotesController, which isn't ported.
  document.addEventListener("input", function (event) {
    var textarea = event.target.closest && event.target.closest(".reviewable-note-form__textarea");
    if (!textarea) {
      return;
    }
    var counter = textarea.closest(".form-kit__container-content").querySelector(".form-kit__char-counter");
    var length = textarea.value.length;
    counter.textContent = length + "/2000";
    counter.classList.toggle("--insufficient", textarea.value.trim().length < 1);
    counter.classList.toggle("--exceeded", length > 2000);
  });

  document.addEventListener("submit", function (event) {
    if (!event.target.closest || !event.target.closest(".reviewable-note-form__form")) {
      return;
    }
    event.preventDefault();
    notPorted("review queue notes");
  });

  // ---- Watched words ----------------------------------------------------

  // highlightHTML: each word, delimited by non-word characters, in a
  // mark in the text nodes under the element.
  function highlight(node, pattern) {
    if (node.nodeType === Node.ELEMENT_NODE) {
      if (node.tagName === "SCRIPT" || node.tagName === "STYLE") {
        return 0;
      }
      if (node.tagName === "MARK" && node.className === "watched-word-highlight") {
        return 0;
      }
      for (var i = 0; i < node.childNodes.length; i++) {
        i += highlight(node.childNodes[i], pattern);
      }
      return 0;
    }
    if (node.nodeType !== Node.TEXT_NODE) {
      return 0;
    }
    var match = node.data.match(pattern);
    if (!match) {
      return 0;
    }
    var mark = document.createElement("mark");
    mark.className = "watched-word-highlight";
    mark.innerText = match[2];
    var matchNode = node.splitText(match.index + match[1].length);
    matchNode.splitText(match[2].length);
    matchNode.parentNode.replaceChild(mark, matchNode);
    return 1;
  }

  // Items already highlighted, so a redraw only marks the new ones.
  var highlighted = new WeakSet();

  function highlightAll() {
    var reviewables = pageData().reviewables || {};
    Object.keys(reviewables).forEach(function (id) {
      var words = (reviewables[id].watched_words || []).filter(Boolean);
      var item = document.querySelector('.review-item[data-reviewable-id="' + id + '"]');
      if (!words.length || !item || highlighted.has(item)) {
        return;
      }
      highlighted.add(item);
      // highlightWatchedWords: highlightHTML once for each word.
      words.forEach(function (word) {
        var escaped = word.replace(/[-/\\^$*+?.()|[\]{}]/g, "\\$&");
        var pattern = new RegExp("(\\W|^)(" + escaped + ")(\\W|$)", "i");
        item
          .querySelectorAll(".review-item__post-content, .reviewable-topic-link__title-link .title-text")
          .forEach(function (el) {
            highlight(el, pattern);
          });
      });
    });
  }

  Discourse.onPage(function () {
    stash = null;
    highlightAll();
  });
})();
