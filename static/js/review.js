// The review queue's page (review.index, src/review_view.rs draws it):
// ReviewFilters' Refresh (ReviewIndexController#refresh's query params)
// and Options (the secondary filters, kept while hidden), each
// reviewable's moderator actions (ReviewableBundledAction and
// ReviewableItem#perform: the reviewable redrawn from the server after
// the action, its completed message a toast), the permalink copy, the
// note form's counter, and the watched words its scores name
// highlighted in the post and title (highlightHTML).
//
// Claiming (ReviewableClaimedTopic's buttons, and the automatic claim a
// confirmation takes first and releases after: #claimReviewable,
// #unclaimAutomaticReviewable) and the notes (ReviewableNoteForm, the
// timeline's delete) go to the server, the reviewable drawn again after.
//
// A queued post's revision asks for its reason first
// (ReviseAndRejectPostReviewable, a DModal over a copy of the post).
//
// What leans on parts not ported says so instead: the client actions
// (edit, suspend, silence), the reject reason modal, the insights tab,
// editing, a queued post's raw email and the pin toggle.
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

  // lib/reviewable-penalty's survivingPenalty: a silence the action keeps
  // that the viewer may lift.
  function survivingPenalty(action, penalties) {
    if (!action || action.penalty_effect !== "retains_penalty") {
      return null;
    }
    return (
      (penalties || []).find(function (p) {
        return p.kind === "silence" && p.can_lift;
      }) || null
    );
  }

  function interpolateUsername(text, username) {
    return (text || "").replace("%{username}", username);
  }

  // ReviewableItem#showCompletedToast: the completed message, or, while
  // the author stays silenced, a toast that stays with an Undo that lifts
  // the silence (#liftPenalty), one per author.
  function showCompletedToast(item, action, data) {
    var penalty = survivingPenalty(action, data.author_penalties);
    var author = data.target_created_by;
    if (!penalty || !author) {
      if (action.completed_message) {
        ui().toast(action.completed_message);
      }
      return;
    }
    var strings = pageData().strings || {};
    var key = "reviewable-author-penalty-" + author.id;
    var section = document.querySelector("section.fk-d-toasts");
    if (!section) {
      section = document.createElement("section");
      section.className = "fk-d-toasts";
      document.body.appendChild(section);
    }
    var old = section.querySelector('output[data-toast-key="' + key + '"]');
    if (old) {
      old.remove();
    }
    var output = document.createElement("output");
    output.className = "fk-d-toast";
    output.setAttribute("role", "status");
    output.dataset.toastKey = key;
    output.innerHTML =
      '<div class="fk-d-default-toast -success"><div class="fk-d-default-toast__icon-container">' +
      ui().icon("check") +
      '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts">' +
      '<div class="fk-d-default-toast__title"></div><div class="fk-d-default-toast__message"></div></div>' +
      '<div class="fk-d-default-toast__actions"><button class="btn btn-text btn-transparent reviewable-undo-penalty" tabindex="0" type="button"><span class="d-button-label"></span></button></div>' +
      '</div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
      ui().icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>';
    var title = output.querySelector(".fk-d-default-toast__title");
    if (action.completed_message) {
      title.textContent = action.completed_message;
    } else {
      title.remove();
    }
    output.querySelector(".fk-d-default-toast__message").textContent = interpolateUsername(
      strings.still_silenced,
      author.username
    );
    output.querySelector(".reviewable-undo-penalty .d-button-label").textContent = strings.undo_penalty || "";
    var close = function () {
      output.remove();
    };
    output.querySelector(".fk-d-default-toast__close-container button").addEventListener("click", close);
    output.querySelector(".reviewable-undo-penalty").addEventListener("click", function () {
      ui()
        .ajax("PUT", "/admin/users/" + author.id + "/unsilence")
        .then(function () {
          close();
          ui().toast(interpolateUsername(strings.penalty_undone, author.username));
          return redraw(item, [item.dataset.reviewableId]);
        })
        .catch(function (error) {
          ui().failed(error);
        });
    });
    section.appendChild(output);
  }

  // ReviewableItem#_performConfirmed, without a client action; the
  // modal's fields (additionalData) go with it.
  function performConfirmed(item, action, additionalData) {
    var data = reviewableData(item);
    var id = item.dataset.reviewableId;
    setDisabled(item, true);
    return ui()
      .ajax(
        "PUT",
        "/review/" + id + "/perform/" + encodeURIComponent(action.server_action) + "?version=" + data.version,
        additionalData
      )
      .then(function (json) {
        var result = (json && json.reviewable_perform_result) || {};
        showCompletedToast(item, action, data);
        // An approved queued topic now has one: the redraw looks there.
        if (result.created_post_topic_id && !data.topic_id) {
          data.topic_id = result.created_post_topic_id;
          storeData(item, data);
        }
        var ids = [id];
        Object.keys(result.reviewable_updates || {}).forEach(function (other) {
          if (ids.indexOf(String(other)) === -1) {
            ids.push(String(other));
          }
        });
        // Then #unclaimAutomaticReviewable, before the redraw.
        return unclaimAutomatic(item).then(function () {
          return redraw(item, ids);
        });
      })
      .catch(function (error) {
        setDisabled(item, false);
        ui().failed(error);
        unclaimAutomatic(item);
      });
  }

  // #claimReviewable: a reviewable without a topic counts as claimed; an
  // unclaimed one is claimed automatically. True when the viewer holds it.
  function claimReviewable(item) {
    var data = reviewableData(item);
    if (!data.topic_id) {
      return Promise.resolve(true);
    }
    var claim = data.claimed_by
      ? Promise.resolve()
      : ui()
          .ajax("POST", "/reviewable_claimed_topics", {
            reviewable_claimed_topic: { topic_id: data.topic_id, automatic: true },
          })
          .then(function () {
            data.claimed_by = { user_id: pageData().viewer_id, automatic: true };
            storeData(item, data);
          });
    return claim.then(
      function () {
        return !!data.claimed_by && data.claimed_by.user_id === pageData().viewer_id;
      },
      function (error) {
        ui().failed(error);
        return false;
      }
    );
  }

  // #unclaimAutomaticReviewable
  function unclaimAutomatic(item) {
    var data = reviewableData(item);
    if (!data.topic_id || !data.claimed_by || !data.claimed_by.automatic) {
      return Promise.resolve();
    }
    return ui()
      .ajax("DELETE", "/reviewable_claimed_topics/" + data.topic_id, { automatic: true })
      .then(function () {
        data.claimed_by = null;
        storeData(item, data);
      })
      .catch(function (error) {
        ui().failed(error);
      });
  }

  function storeData(item, data) {
    var page = pageData();
    page.reviewables[item.dataset.reviewableId] = data;
    document.getElementById("review-page-data").textContent = JSON.stringify(page);
  }

  // ReviewableClaimedTopic#claim and #unclaim, then every reviewable of the
  // topic on the page drawn again (the /reviewable_claimed message's work
  // in Ember).
  function toggleClaim(item, button) {
    var topicId = reviewableData(item).topic_id;
    var request = button.classList.contains("unclaim")
      ? ui().ajax("DELETE", "/reviewable_claimed_topics/" + topicId)
      : ui().ajax("POST", "/reviewable_claimed_topics", { reviewable_claimed_topic: { topic_id: topicId } });
    button.disabled = true;
    request
      .then(function () {
        var reviewables = pageData().reviewables || {};
        var ids = Object.keys(reviewables).filter(function (id) {
          return reviewables[id].topic_id === topicId;
        });
        return redraw(item, ids);
      })
      .catch(function (error) {
        button.disabled = false;
        ui().failed(error);
      });
  }

  // ReviewableItem#perform
  function perform(item, action) {
    if (!action) {
      return;
    }
    // A confirmation claims the reviewable's topic first, and releases an
    // automatic claim when cancelled.
    if (action.confirm_message) {
      claimReviewable(item).then(function (claimed) {
        if (!claimed) {
          return;
        }
        var options = {
          message: action.confirm_message,
          didConfirm: function () {
            performConfirmed(item, action);
          },
          didCancel: function () {
            unclaimAutomatic(item);
          },
        };
        if (action.confirm_destructive) {
          Discourse.dialog.deleteConfirm(options);
        } else {
          Discourse.dialog.confirm(options);
        }
      });
      return;
    }
    if (action.require_reject_reason) {
      notPorted("the reject reason modal (RejectReasonReviewableModal)");
      return;
    }
    if (action.server_action === "revise_and_reject_post") {
      claimReviewable(item).then(function (claimed) {
        if (claimed) {
          reviseModal(item, action);
        }
      });
      return;
    }
    if (action.client_action) {
      notPorted("the review queue's " + action.client_action + " action");
      return;
    }
    performConfirmed(item, action);
  }

  // ReviseAndRejectPostReviewable: the queued post, the reason (Other
  // asks for a custom one), the optional feedback; Send PM performs with
  // them and closes the modal.
  function reviseModal(item, action) {
    var template = document.getElementById("review-revise-modal");
    if (!template) {
      return;
    }
    var OTHER_REASON = "other_reason";
    var reason = OTHER_REASON;
    var submitting = false;
    var controls = template.content.cloneNode(true);
    var footerButtons = controls.querySelector(".revise-and-reject-reviewable__footer");
    footerButtons.remove();
    Discourse.modal.show({
      className: "revise-and-reject-reviewable",
      title: (pageData().strings || {}).revise_title || "",
      body: function (body) {
        var queued = document.createElement("div");
        queued.className = "revise-and-reject-reviewable__queued-post";
        ["review-item__meta-content", "review-item__post"].forEach(function (name) {
          var part = item.querySelector(".review-item__flag-summary > ." + name);
          if (part) {
            queued.appendChild(part.cloneNode(true));
          }
        });
        body.appendChild(queued);
        body.appendChild(controls);
      },
      footer: function (footer, close) {
        Array.prototype.slice.call(footerButtons.children).forEach(function (button) {
          footer.appendChild(button);
        });
        var send = footer.querySelector(".btn-primary");
        var body = footer.parentNode.querySelector(".d-modal__body");
        var custom = body.querySelector(".revise-and-reject-reviewable__custom-reason");
        var customGroup = custom.closest(".control-group");
        var feedback = body.querySelector(".revise-and-reject-reviewable__feedback");
        var combo = body.querySelector("details.combo-box");

        // sendPMDisabled
        function update() {
          var other = reason === OTHER_REASON;
          customGroup.hidden = !other;
          send.disabled = !reason || (other && !custom.value) || submitting;
        }
        custom.addEventListener("input", update);
        combo.addEventListener("toggle", function () {
          var rows = combo.querySelector("template.select-kit-rows");
          var list = combo.querySelector(".select-kit-body");
          combo.classList.toggle("is-expanded", combo.open);
          if (!combo.open) {
            list.innerHTML = "";
            list.removeAttribute("style");
            return;
          }
          list.innerHTML = '<ul aria-live="polite" class="select-kit-collection" role="menu">' + rows.innerHTML + "</ul>";
          ui().floatBody(combo);
        });
        combo.addEventListener("click", function (event) {
          var row = event.target.closest(".select-kit-row");
          if (!row) {
            return;
          }
          event.preventDefault();
          reason = row.dataset.value;
          var name = row.dataset.name;
          var header = combo.querySelector("summary");
          var choice = header.querySelector(".select-kit-selected-name");
          [header, choice].forEach(function (el) {
            el.dataset.name = name;
            el.dataset.value = reason;
          });
          choice.title = name;
          choice.querySelector(".name").textContent = name;
          combo.open = false;
          update();
        });
        footer.querySelector(".d-modal-cancel").addEventListener("click", close);
        send.addEventListener("click", function () {
          submitting = true;
          update();
          // As Ember sends them: the custom reason typed goes along even
          // after another reason is chosen.
          var data = {
            revise_reason: reason,
            revise_custom_reason: custom.value,
            revise_feedback: feedback.value,
          };
          // A failure is shown by performConfirmed; the modal closes either way.
          performConfirmed(item, action, data).then(close);
        });
        update();
      },
    });
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

    var claimButton = target.closest(".reviewable-claimed-topic button");
    if (claimButton) {
      if (!claimButton.disabled) {
        toggleClaim(item, claimButton);
      }
      return;
    }

    // The timeline's delete: the viewer's own note, or any for an admin.
    var deleteNote = target.closest(".timeline-event__delete-note");
    if (deleteNote) {
      var index = Array.prototype.indexOf.call(item.querySelectorAll(".timeline-event__delete-note"), deleteNote);
      var noteId = (reviewableData(item).deletable_notes || [])[index];
      if (noteId !== undefined && !deleteNote.disabled) {
        deleteNote.disabled = true;
        ui()
          .ajax("DELETE", "/review/" + item.dataset.reviewableId + "/notes/" + noteId)
          .then(function () {
            return redraw(item, [item.dataset.reviewableId]);
          })
          .catch(function (error) {
            deleteNote.disabled = false;
            ui().failed(error);
          });
      }
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

    if (target.closest(".show-raw-email")) {
      event.preventDefault();
      notPorted("a queued post's raw email (RawEmailModal)");
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

  // The note form: FormKit's counter as it is typed; a note (trimmed, not
  // blank) POSTed, then the reviewable drawn again with it.
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
    var form = event.target.closest(".reviewable-note-form__form");
    var item = itemOf(form);
    var textarea = form.querySelector(".reviewable-note-form__textarea");
    var content = textarea.value.trim();
    if (!item || !content) {
      return;
    }
    var submit = form.querySelector("button[type=submit]");
    submit.disabled = true;
    ui()
      .ajax("POST", "/review/" + item.dataset.reviewableId + "/notes", { reviewable_note: { content: content } })
      .then(function () {
        return redraw(item, [item.dataset.reviewableId]);
      })
      .catch(function (error) {
        submit.disabled = false;
        ui().failed(error);
      });
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
          .querySelectorAll(
            ".review-item__post-content, .reviewable-topic-link__title-link .title-text, .reviewable-topic-link > .title-text"
          )
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
