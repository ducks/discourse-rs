// The discovery topic lists' bulk select mode (lib/bulk-select-helper.js,
// components/topic-list/list.gjs, item.gjs, header/sortable-column.gjs,
// header/bulk-select-cell.gjs, item/bulk-select-cell.gjs,
// bulk-select-topics-dropdown.gjs, modal/bulk-topic-actions.gjs), for the
// viewer the page gave script#topic-bulk-select (canBulkSelect):
//
//   The header's toggle (and BulkSelectToggle in the mobile view's
//   navigation controls) turns the mode on and off (toggleBulkSelect).
//   While on, the table is sticky-header bulk-select-enabled, every row
//   is bulk-selecting behind a checkbox cell (in the mobile layout the
//   checkbox takes the avatar's place), the header's column names give
//   way to Select All and Clear All, and NewListHeaderControls hides
//   (renderNewListHeaderControls). A click on a row selects or unselects
//   it, shift on a checkbox selects the range since the last one checked.
//   Rows that arrive later (the next page, the live list) get their
//   checkbox, unchecked: upstream's addTopics on loadMore reads a
//   bulkSelectHelper its component does not have.
//
//   With topics selected a topic manager (canDoBulkActions) gets the Bulk
//   Actions DMenu and the selected count. Its items follow `visible`,
//   read off the rows (their statuses). Close, archive, unlist, relist,
//   pin, un-pin, delete, reset bump dates, update category, update
//   notifications, manage tags and make personal message open
//   BulkTopicActions, which sends Topic.bulkOperation (PUT /topics/bulk,
//   thirty topics a request) and lists what failed; Mark Unread sends at
//   once; Dismiss is the list's dismiss button (topic-dismiss.js, which
//   also labels its buttons with the count and sends the selected
//   topics). Then the list is drawn again (router.refresh), out of the
//   mode, under a toast. The nested replies items are refused, named.
//
//   Manage tags is ManageTagsForm (modal/bulk-topic-actions/
//   manage-tags-form.gjs): tags to remove (or all of them), tags to add
//   and replacement rows, each a TagChooser offering existing tags only
//   (the composer's chooser, Discourse.miniTagChooser.copy), sent as one
//   manage_tags operation, as JSON. Pin is BulkPinOptions (modal/
//   feature-topic/bulk-pin-options.gjs): topic-admin-modals.js'
//   PinOptionSection for the topics' categories and for every list, each
//   until a date it requires, with /topics/feature_stats' counts. Not as
//   Ember: a replacement row's chooser does not move above its header on
//   mobile (mobilePlacement).
(function () {
  "use strict";

  if (!Discourse.once("topic-list-bulk-select")) {
    return;
  }

  var CHUNK = 30;
  // topicLevels: watching, tracking, regular, muted.
  var LEVELS = [
    [3, "watching"],
    [2, "tracking"],
    [1, "regular"],
    [0, "muted"],
  ];

  var enabled = false;
  // The selected topics' ids, in the order they were selected.
  var selected = [];
  var lastChecked = null;
  // What the mode took out of the page, to put back: the header cells'
  // children and the mobile rows' avatars.
  var headerStash = [];
  var avatars = new WeakMap();
  var observer = null;
  var menu = null;
  // An action's refresh on its way: { toast: [kind, text] or null }.
  var refreshing = null;
  // While the page is drawn again in place (page.js' redraw), and whether
  // the mode goes back on over the list it brings.
  var redrawing = false;
  var resume = false;

  function config() {
    var el = document.getElementById("topic-bulk-select");
    if (!el) {
      return null;
    }
    if (!el.bulkSelectConfig) {
      try {
        el.bulkSelectConfig = JSON.parse(el.textContent);
      } catch (_) {
        return null;
      }
    }
    return el.bulkSelectConfig;
  }

  function t(key, count) {
    var strings = (config() || {}).strings || {};
    if (count === undefined) {
      return strings[key] === undefined ? key : strings[key];
    }
    var text = (count === 1 && strings[key + ".one"]) || strings[key + ".other"] || key;
    return text.split("%{count}").join(String(count));
  }

  function icon(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      " svg-icon fa-width-auto" +
      (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  function el(html) {
    var template = document.createElement("template");
    template.innerHTML = html;
    return template.content.firstChild;
  }

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function table() {
    return document.querySelector("#list-area table.topic-list");
  }

  function rows() {
    return Array.from(document.querySelectorAll("#list-area tbody.topic-list-body > tr.topic-list-item"));
  }

  function rowOf(id) {
    return document.querySelector('#list-area tbody.topic-list-body > tr.topic-list-item[data-topic-id="' + Number(id) + '"]');
  }

  function escapeHtml(text) {
    return String(text == null ? "" : text)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  // A topic's category off its row's badge, as { id, link }
  // (categoryLinkHTML); a row drawn without one is in the config's
  // fallback_category.
  function categoryOf(row) {
    var badge = row.querySelector(
      ".link-bottom-line .badge-category[data-category-id], .topic-item-stats__category-tags .badge-category[data-category-id]"
    );
    if (!badge) {
      return config().fallback_category || null;
    }
    var wrapper = badge.closest(".badge-category__wrapper");
    return { id: Number(badge.dataset.categoryId), link: wrapper ? wrapper.outerHTML : "" };
  }

  // BulkTopicActions#soleCategory: the selected topics' category when
  // they share one (bulkSelectHelper.selectedCategoryIds).
  function soleCategory() {
    var found = [];
    selected
      .map(rowOf)
      .filter(Boolean)
      .map(categoryOf)
      .forEach(function (category) {
        var id = category ? category.id : null;
        if (
          !found.some(function (other) {
            return (other ? other.id : null) === id;
          })
        ) {
          found.push(category);
        }
      });
    return found.length === 1 ? found[0] : null;
  }

  // Category.findById's badge for a category on the page, or nothing.
  function categoryLink(id) {
    var fallback = config().fallback_category;
    if (fallback && fallback.id === id) {
      return fallback.link;
    }
    var badge = document.querySelector('#list-area tr.topic-list-item .badge-category[data-category-id="' + Number(id) + '"]');
    var wrapper = badge && badge.closest(".badge-category__wrapper");
    return wrapper ? wrapper.outerHTML : "";
  }

  // renderTag
  function renderTag(name) {
    var visible = escapeHtml(name);
    var lower = visible.toLowerCase();
    return (
      "<a href='" +
      basePath() +
      "/tag/" +
      lower.split(".").join("%2E") +
      "' data-tag-name=\"" +
      lower +
      "\" class='discourse-tag " +
      escapeHtml(config().tag_style || "simple") +
      "'>" +
      visible +
      "</a>"
    );
  }

  function unported(what) {
    var notice = document.getElementById("page-error");
    if (notice) {
      notice.textContent = "Not ported yet: " + what + ".";
      notice.hidden = false;
    }
  }

  // toasts.success / toasts.error ({ duration: "short" })
  function toast(kind, text) {
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
      '<div class="fk-d-default-toast -' +
      kind +
      '"><div class="fk-d-default-toast__icon-container">' +
      icon(kind === "success" ? "check" : "triangle-exclamation") +
      '</div><div class="fk-d-default-toast__main-container"><div class="fk-d-default-toast__texts"><div class="fk-d-default-toast__message"></div></div></div><div class="fk-d-default-toast__close-container"><button class="btn no-text btn-icon btn-transparent" type="button">' +
      icon("xmark") +
      '<span aria-hidden="true">&#8203;</span></button></div></div>';
    output.querySelector(".fk-d-default-toast__message").textContent = text;
    section.appendChild(output);
    var close = function () {
      output.remove();
    };
    output.querySelector("button").addEventListener("click", close);
    setTimeout(close, 3000);
  }

  // BulkSelectCheckbox
  function checkbox(id) {
    return el(
      '<label for="bulk-select-' + id + '"><input class="bulk-select" id="bulk-select-' + id + '" type="checkbox"></label>'
    );
  }

  // A row in the mode: ItemBulkSelectCell before its cells, or in the
  // mobile layout the checkbox where the avatar was.
  function decorate(row) {
    var id = Number(row.dataset.topicId);
    if (!id || row.querySelector("input.bulk-select")) {
      return;
    }
    row.classList.add("bulk-selecting");
    var pull = row.querySelector(":scope > td.topic-list-data > .pull-left");
    if (pull) {
      pull.parentNode.classList.add("bulk-select-enabled");
      avatars.set(row, Array.from(pull.childNodes));
      pull.textContent = "";
      pull.appendChild(checkbox(id));
    } else {
      var cell = document.createElement("td");
      cell.className = "bulk-select topic-list-data";
      cell.appendChild(checkbox(id));
      row.insertBefore(cell, row.firstChild);
    }
    // A selected row drawn again (the live list) stays selected.
    if (selected.indexOf(id) !== -1) {
      row.querySelector("input.bulk-select").checked = true;
      row.classList.add("bulk-selected");
    }
  }

  function undecorate(row) {
    row.classList.remove("bulk-selecting", "bulk-selected");
    var pull = row.querySelector(":scope > td.topic-list-data > .pull-left");
    if (pull) {
      pull.parentNode.classList.remove("bulk-select-enabled");
      pull.textContent = "";
      (avatars.get(row) || []).forEach(function (node) {
        pull.appendChild(node);
      });
      avatars.delete(row);
    } else {
      var cell = row.querySelector(":scope > td.bulk-select");
      if (cell) {
        cell.remove();
      }
    }
  }

  // SortableColumn while bulkSelectEnabled: Select All and Clear All.
  function controls() {
    var span = el(
      '<span class="bulk-select-topics"><div class="bulk-select-topics-actions"><button class="btn btn-default bulk-select-all"></button><button class="btn btn-default bulk-clear-all"></button></div></span>'
    );
    span.querySelector(".bulk-select-all").textContent = t("topics.bulk.select_all");
    span.querySelector(".bulk-clear-all").textContent = t("topics.bulk.clear_all");
    return span;
  }

  function closeMenu() {
    if (menu && menu.expanded) {
      menu.close({ focusTrigger: false });
    }
    menu = null;
  }

  // TopicBulkSelectDropdown beside the buttons once a topic manager has
  // topics selected (canDoBulkActions), and the dismiss buttons' labels.
  function sync() {
    var span = document.querySelector("#list-area thead .bulk-select-topics");
    var cfg = config();
    if (span && cfg) {
      var box = span.querySelector(":scope > .bulk-select-topics-dropdown");
      if (cfg.can_manage_topic && selected.length) {
        if (!box) {
          box = el(
            '<div class="bulk-select-topics-dropdown"><button aria-expanded="false" class="btn no-text fk-d-menu__trigger bulk-select-topics-dropdown-trigger btn-default" data-identifier="bulk-select-topics-dropdown" data-trigger="" type="button"><span class="d-button-label"></span>' +
              icon("angle-down") +
              '</button><span class="bulk-select-topic-dropdown__count"></span></div>'
          );
          box.querySelector(".d-button-label").textContent = t("select_kit.components.bulk_select_topics_dropdown.title");
          span.appendChild(box);
        }
        box.querySelector(".bulk-select-topic-dropdown__count").textContent = t("topics.bulk.selected_count", selected.length);
      } else if (box) {
        closeMenu();
        box.remove();
      }
    }
    document.dispatchEvent(new CustomEvent("discourse:bulk-select-topics"));
  }

  function enable() {
    var list = table();
    var cfg = config();
    if (!list || !cfg) {
      return;
    }
    enabled = true;
    selected = [];
    lastChecked = null;
    list.classList.add("sticky-header", "bulk-select-enabled");
    var head = list.querySelector("thead.topic-list-header > tr");
    headerStash = [];
    Array.from(head.children).forEach(function (th) {
      headerStash.push([th, Array.from(th.childNodes)]);
      th.textContent = "";
      if (th.classList.contains("default")) {
        th.appendChild(controls());
      }
    });
    // HeaderBulkSelectCell
    var cell = el(
      '<th class="bulk-select topic-list-data"><button class="btn-transparent bulk-select no-text --primary">' +
        icon("list-check") +
        "</button></th>"
    );
    cell.firstChild.title = t("topics.bulk.toggle");
    head.insertBefore(cell, head.firstChild);
    rows().forEach(decorate);
    var body = list.querySelector("tbody.topic-list-body");
    observer = new MutationObserver(function () {
      rows().forEach(decorate);
    });
    observer.observe(body, { childList: true });
    document.querySelectorAll("#list-area .topic-replies-toggle-wrapper").forEach(function (wrapper) {
      wrapper.hidden = true;
    });
    sync();
  }

  function disable() {
    enabled = false;
    selected = [];
    lastChecked = null;
    closeMenu();
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    var list = table();
    if (list) {
      list.classList.remove("sticky-header", "bulk-select-enabled");
      var cell = list.querySelector("thead.topic-list-header > tr > th.bulk-select");
      if (cell) {
        cell.remove();
      }
      headerStash.forEach(function (pair) {
        pair[0].textContent = "";
        pair[1].forEach(function (node) {
          pair[0].appendChild(node);
        });
      });
      rows().forEach(undecorate);
    }
    headerStash = [];
    document.querySelectorAll("#list-area .topic-replies-toggle-wrapper").forEach(function (wrapper) {
      wrapper.hidden = false;
    });
    sync();
  }

  // BulkSelectHelper#toggleBulkSelect
  function toggle() {
    if (enabled) {
      disable();
    } else {
      enable();
    }
  }

  // Item#selectTopic: with shift, the checkboxes between this row and the
  // last one checked are clicked too.
  function selectTopic(row, shiftKey) {
    var id = Number(row.dataset.topicId);
    if (selected.indexOf(id) === -1) {
      selected.push(id);
    }
    row.querySelector("input.bulk-select").checked = true;
    if (lastChecked && shiftKey) {
      var siblings = Array.from(row.parentNode.children);
      var from = siblings.indexOf(row);
      var to = siblings.findIndex(function (other) {
        return other.dataset.topicId === lastChecked;
      });
      Array.from(document.querySelectorAll("input.bulk-select"))
        .slice(Math.min(from, to), Math.max(from, to))
        .filter(function (input) {
          return !input.checked;
        })
        .forEach(function (input) {
          input.click();
        });
    }
    lastChecked = row.dataset.topicId;
    row.classList.add("bulk-selected");
    sync();
  }

  // Item#unselectTopic
  function unselectTopic(row) {
    var at = selected.indexOf(Number(row.dataset.topicId));
    if (at !== -1) {
      selected.splice(at, 1);
    }
    row.querySelector("input.bulk-select").checked = false;
    lastChecked = null;
    row.classList.remove("bulk-selected");
    sync();
  }

  // wantsNewWindow
  function wantsNewWindow(event) {
    return event.defaultPrevented || event.shiftKey || event.metaKey || event.ctrlKey || (event.button && event.button !== 0);
  }

  // What `visible` reads of a selected topic, off its row's statuses.
  function topics() {
    return selected
      .map(rowOf)
      .filter(Boolean)
      .map(function (row) {
        return {
          visible: !row.querySelector(".topic-statuses .topic-status.--invisible"),
          pinned: !!row.querySelector(".topic-statuses .topic-status.--pinned, .topic-statuses .topic-status.--unpinned"),
        };
      });
  }

  // BulkSelectTopicsDropdown#buttons on a discovery list, whose topics
  // are never personal messages: [id, icon, name key].
  function buttons() {
    var cfg = config();
    var list = topics();
    var some = function (test) {
      return list.some(test);
    };
    return [
      [cfg.filter === "unread", "dismiss-unread", "check", "dismiss"],
      [cfg.filter === "new", "dismiss-new", "check", "dismiss"],
      [true, "update-category", "pencil", "update_category"],
      [true, "update-notifications", "far-bell", "update_notifications"],
      [true, "reset-bump-dates", "anchor", "reset_bump_dates"],
      [true, "defer", "circle", "defer"],
      [true, "close-topics", "lock", "close_topics"],
      [cfg.manage_tags, "manage-tags", "tag", "manage_tags"],
      [true, "pin-topics", "thumbtack", "pin_topics"],
      [
        some(function (topic) {
          return topic.pinned;
        }),
        "unpin-topics",
        "thumbtack",
        "unpin_topics",
      ],
      [true, "archive-topics", "folder", "archive_topics"],
      [cfg.staff, "convert-to-private-message", "envelope", "convert_to_private_message"],
      // No topic here is in the nested view (its toggle is not ported),
      // so only enabling is offered.
      [cfg.nested_replies && list.length > 0, "enable-nested-replies", "nested-thread", "enable_nested_replies"],
      [
        some(function (topic) {
          return topic.visible;
        }),
        "unlist-topics",
        "far-eye-slash",
        "unlist_topics",
      ],
      [
        some(function (topic) {
          return !topic.visible;
        }),
        "relist-topics",
        "far-eye",
        "relist_topics",
      ],
      [cfg.can_delete_all_posts_and_topics, "delete-topics", "trash-can", "delete_topics"],
    ].filter(function (button) {
      return button[0];
    });
  }

  function openMenu(trigger) {
    if (menu && menu.expanded) {
      closeMenu();
      return;
    }
    var content = document.createElement("ul");
    content.className = "dropdown-menu";
    buttons().forEach(function (button) {
      var item = el(
        '<li class="dropdown-menu__item"><button class="btn btn-icon-text btn-transparent ' +
          button[1] +
          '" type="button">' +
          icon(button[2]) +
          '<span class="d-button-label"></span></button></li>'
      );
      item.querySelector(".d-button-label").textContent = t("topic_bulk_actions." + button[3] + ".name");
      item.firstChild.addEventListener("click", function () {
        closeMenu();
        onSelect(button[1]);
      });
      content.appendChild(item);
    });
    menu = Discourse.float.menu(trigger, {
      identifier: "bulk-select-topics-dropdown",
      autofocus: true,
      modalForMobile: true,
      content: content,
    });
  }

  // jQuery.param, for Topic.bulkOperation's data: topic_ids, operation,
  // tracked (sent empty when undefined). With asJSON (manage_tags, whose
  // operation nests lists) the data is JSON, the undefined left out.
  function bulkBody(ids, operation, tracked) {
    var body = new URLSearchParams();
    ids.forEach(function (id) {
      body.append("topic_ids[]", String(id));
    });
    Object.keys(operation).forEach(function (key) {
      var value = operation[key];
      body.append("operation[" + key + "]", value === undefined || value === null ? "" : String(value));
    });
    body.append("tracked", tracked === undefined ? "" : String(tracked));
    return body.toString();
  }

  function bulkOperation(ids, operation, asJSON) {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (_) {}
    headers["Content-Type"] = asJSON ? "application/json" : "application/x-www-form-urlencoded; charset=UTF-8";
    headers.Accept = "application/json, text/javascript, */*; q=0.01";
    return fetch(basePath() + "/topics/bulk", {
      method: "PUT",
      headers: headers,
      credentials: "same-origin",
      body: asJSON ? JSON.stringify({ topic_ids: ids, operation: operation }) : bulkBody(ids, operation),
    }).then(function (r) {
      if (!r.ok) {
        throw r;
      }
      return r.json();
    });
  }

  // extractError
  function extractError(r) {
    if (!(r instanceof Response)) {
      return Promise.resolve(t("generic_error"));
    }
    return r
      .json()
      .catch(function () {
        return null;
      })
      .then(function (json) {
        if (json && json.errors && json.errors.length) {
          return t("generic_error_with_reason").split("%{error}").join(json.errors.join(" "));
        }
        if (json && (json.error || json.message)) {
          return json.error || json.message;
        }
        return r.status + " " + (r.statusText || "");
      });
  }

  // BulkTopicActions#_processChunks: thirty topics a request, their
  // topic_ids, errors and tag_category_errors merged; a request that
  // fails counts its topics under its error and ends the run.
  function processChunks(ids, operation) {
    var done = [];
    var errors = {};
    var tagCategoryErrors = {};
    var asJSON = operation.type === "manage_tags";
    var chunks = [];
    for (var i = 0; i < ids.length; i += CHUNK) {
      chunks.push(ids.slice(i, i + CHUNK));
    }
    var run = chunks.reduce(function (before, chunk) {
      return before.then(function (go) {
        if (!go) {
          return false;
        }
        return bulkOperation(chunk, operation, asJSON).then(
          function (result) {
            ((result && result.topic_ids) || []).forEach(function (id) {
              done.push(id);
            });
            Object.keys((result && result.errors) || {}).forEach(function (message) {
              errors[message] = (errors[message] || 0) + result.errors[message];
            });
            ((result && result.tag_category_errors) || []).forEach(function (error) {
              var key = error.category_id + ":" + error.tag_names.join(",");
              if (tagCategoryErrors[key]) {
                tagCategoryErrors[key].count += error.count;
              } else {
                tagCategoryErrors[key] = Object.assign({}, error);
              }
            });
            return true;
          },
          function (r) {
            return extractError(r).then(function (message) {
              errors[message] = (errors[message] || 0) + chunk.length;
              return false;
            });
          }
        );
      });
    }, Promise.resolve(true));
    return run.then(function () {
      var restricted = Object.keys(tagCategoryErrors).map(function (key) {
        return tagCategoryErrors[key];
      });
      return {
        topics: done,
        errors: Object.keys(errors).length ? errors : null,
        tagCategoryErrors: restricted.length ? restricted : null,
      };
    });
  }

  // refreshClosure (router.refresh) and toggleBulkSelect, then the toast
  // over the list drawn again (the last refresh's, when one overtakes
  // another).
  function refresh(kind, text) {
    disable();
    if (!Discourse.refresh) {
      location.reload();
      return;
    }
    refreshing = { toast: text ? [kind, text] : null };
    Discourse.refresh();
  }

  function dButton(className, id, label) {
    var button = el('<button class="btn btn-text ' + className + '" id="' + id + '" type="button"><span class="d-button-label"></span></button>');
    button.querySelector(".d-button-label").textContent = label;
    return button;
  }

  // BulkTopicActions. opts: { title, description, confirm, allowSilent,
  // category, notifications, closeNote, manageTags, pin, showFooter } and
  // operation(state), the operation performAction sends (ManageTagsForm
  // and BulkPinOptions hand theirs over).
  function showModal(opts) {
    if (!Discourse.modal) {
      return;
    }
    var ids = selected.slice();
    var state = { categoryId: undefined, notificationLevelId: null, closeNote: null, notifyUsers: false };
    // registerCustomAction: ManageTagsForm's triggerSubmit.
    var customAction = null;
    var bodyEl = null;
    var footerEl = null;
    var closeModal = null;
    var section = null;

    function drawFooter() {
      footerEl.textContent = "";
      if (opts.showFooter === false) {
        return;
      }
      if (opts.allowSilent) {
        var options = el(
          '<div class="topic-bulk-actions-options"><label class="checkbox-label" for="topic-bulk-action-options__notify"><input id="topic-bulk-action-options__notify" class="ember-checkbox" type="checkbox"></label></div>'
        );
        options.firstChild.appendChild(document.createTextNode(t("topics.bulk.notify")));
        options.querySelector("input").addEventListener("change", function (event) {
          state.notifyUsers = event.target.checked;
        });
        footerEl.appendChild(options);
      }
      var cancel = dButton("btn-transparent d-modal-cancel", "bulk-topics-cancel", t("cancel"));
      cancel.addEventListener("click", function () {
        closeModal();
      });
      footerEl.appendChild(cancel);
      var confirm = dButton("btn-primary", "bulk-topics-confirm", opts.confirm ? t(opts.confirm, ids.length) : t("topics.bulk.confirm"));
      // disabledSubmit: no level yet, or ManageTagsForm with nothing set.
      confirm.disabled = !!opts.notifications || !!opts.manageTags;
      confirm.addEventListener("click", performAction);
      footerEl.appendChild(confirm);
    }

    function drawBody() {
      section.textContent = "";
      if (opts.description) {
        var description = document.createElement("p");
        description.className = "topic-bulk-actions-modal__description";
        description.innerHTML = t(opts.description);
        section.appendChild(description);
      }
      if (opts.category) {
        var p = document.createElement("p");
        var chooser =
          Discourse.topicAdminModals &&
          Discourse.topicAdminModals.categoryChooser({ strings: config().strings }, function (id) {
            state.categoryId = id;
          });
        if (chooser) {
          p.appendChild(chooser);
        }
        section.appendChild(p);
      }
      if (opts.notifications) {
        var list = document.createElement("div");
        list.className = "bulk-notification-list";
        LEVELS.forEach(function (level) {
          var controls = el(
            '<div class="controls"><label class="radio notification-level-radio checkbox-label"><input name="notification_level" type="radio" value="' +
              level[0] +
              '"><strong></strong><div class="description"></div></label></div>'
          );
          controls.querySelector("strong").textContent = t("topic.notifications." + level[1] + ".title");
          controls.querySelector(".description").innerHTML = t("topic.notifications." + level[1] + ".description");
          controls.querySelector("input").addEventListener("change", function (event) {
            state.notificationLevelId = event.target.value;
            footerEl.querySelector("#bulk-topics-confirm").disabled = false;
          });
          list.appendChild(controls);
        });
        section.appendChild(list);
      }
      if (opts.manageTags) {
        var form = manageTagsForm({
          category: soleCategory(),
          onPerform: perform,
          setSubmitDisabled: function (value) {
            footerEl.querySelector("#bulk-topics-confirm").disabled = value;
          },
        });
        customAction = form.submit;
        section.appendChild(form.el);
      }
      if (opts.pin) {
        bulkPinOptions(section, soleCategory(), function (pin) {
          perform(Object.assign({ type: "pin" }, pin));
        });
      }
      if (opts.closeNote) {
        var note = el(
          '<div class="bulk-close-note-section"><label></label><textarea id="bulk-close-note"></textarea></div>'
        );
        note.firstChild.innerHTML = '&nbsp;<span class="label-optional"></span>';
        note.firstChild.insertBefore(document.createTextNode(t("topic_bulk_actions.close_topics.note")), note.firstChild.firstChild);
        note.querySelector(".label-optional").textContent = t("topic_bulk_actions.close_topics.optional");
        note.querySelector("textarea").addEventListener("input", function (event) {
          state.closeNote = event.target.value;
        });
        section.appendChild(note);
      }
    }

    // DConditionalLoadingSection while the requests run.
    function loading() {
      section.classList.add("is-loading");
      section.innerHTML = '<span class="title"></span><div class="spinner "></div>';
      section.querySelector(".title").textContent = t("topics.bulk.performing");
      footerEl.querySelectorAll("button").forEach(function (button) {
        button.disabled = true;
      });
    }

    // _buildTagCategoryError
    function tagCategoryError(error) {
      return t("topics.bulk.tag_not_allowed_in_category", error.tag_names.length)
        .split("%{tags}")
        .join(error.tag_names.map(renderTag).join(" "))
        .split("%{category}")
        .join(categoryLink(error.category_id) || escapeHtml(error.category_name));
    }

    // _showErrors: what was done, skipped and refused, and why (the tags
    // a category does not allow first).
    function showErrors(errors, tagCategoryErrors, done) {
      errors = errors || {};
      tagCategoryErrors = tagCategoryErrors || [];
      var failed =
        tagCategoryErrors.reduce(function (sum, error) {
          return sum + error.count;
        }, 0) +
        Object.keys(errors).reduce(function (sum, message) {
          return sum + errors[message];
        }, 0);
      var skipped = ids.length - done - failed;
      section.classList.remove("is-loading");
      section.textContent = "";
      var box = document.createElement("div");
      box.className = "topic-bulk-actions-modal__errors";
      [
        [done, "topics.bulk.completed_count"],
        [skipped, "topics.bulk.skipped_count"],
        [failed, "topics.bulk.not_completed"],
      ].forEach(function (line, i) {
        if (line[0] || i === 2) {
          var p = document.createElement("p");
          p.innerHTML = t(line[1], line[0]);
          box.appendChild(p);
        }
      });
      var list = document.createElement("ul");
      tagCategoryErrors.forEach(function (error) {
        var item = document.createElement("li");
        item.innerHTML = tagCategoryError(error);
        item.appendChild(document.createTextNode(" (" + t("topics.bulk.error_topic_count", error.count) + ")"));
        list.appendChild(item);
      });
      Object.keys(errors).forEach(function (message) {
        var item = document.createElement("li");
        item.textContent = message + " (" + t("topics.bulk.error_topic_count", errors[message]) + ")";
        list.appendChild(item);
      });
      box.appendChild(list);
      section.appendChild(box);
      footerEl.textContent = "";
      var close = dButton("btn-primary", "bulk-topics-close", t("close"));
      // closeWithRefresh
      close.addEventListener("click", function () {
        closeModal();
        refresh();
      });
      footerEl.appendChild(close);
    }

    // performAction: the action's operation, or the form's own submit.
    function performAction() {
      if (customAction) {
        customAction();
      } else {
        perform(opts.operation(state));
      }
    }

    // forEachPerformed / performAndRefresh
    function perform(operation) {
      if (opts.allowSilent) {
        operation.silent = !state.notifyUsers;
      }
      if (opts.closeNote && state.closeNote) {
        operation.message = state.closeNote;
      }
      loading();
      processChunks(ids, operation).then(function (result) {
        if (result.errors || result.tagCategoryErrors) {
          showErrors(result.errors, result.tagCategoryErrors, result.topics.length);
          return;
        }
        closeModal();
        refresh("success", t("topics.bulk.completed"));
      });
    }

    closeModal = Discourse.modal.show({
      className: "topic-bulk-actions-modal",
      title: t(opts.title),
      body: function (body) {
        bodyEl = body;
        section = document.createElement("div");
        section.className = "conditional-loading-section";
        bodyEl.appendChild(section);
      },
      footer: function (footer) {
        footerEl = footer;
      },
    });
    drawBody();
    drawFooter();
    var note = section.querySelector("#bulk-close-note");
    if (note) {
      note.focus();
    }
  }

  // FKField: the container, title and description around a control, and
  // its error. opts: { title, description, full, error }.
  function fkField(name, type, control, opts) {
    var field = document.createElement("div");
    field.className =
      "form-kit__container form-kit__field form-kit__field-" +
      type +
      (opts.error ? " has-error" : "") +
      (opts.full ? " --full" : "");
    field.dataset.name = name;
    field.id = "control-" + name.split(".").join("-");
    field.dataset.controlType = type;
    if (opts.title) {
      var label = el('<label class="form-kit__container-title"><span></span></label>');
      label.firstChild.textContent = opts.title;
      if (control.id) {
        label.htmlFor = control.id;
      }
      field.appendChild(label);
    }
    if (opts.description) {
      var description = el('<p class="form-kit-text form-kit__container-description"></p>');
      description.textContent = opts.description;
      field.appendChild(description);
    }
    var content = el('<div class="form-kit__container-content"></div>');
    content.appendChild(control.el || control);
    if (opts.error) {
      var meta = el(
        '<div class="form-kit__meta"><p aria-live="assertive" class="form-kit__errors"><span>' +
          icon("triangle-exclamation") +
          " </span></p></div>"
      );
      meta.querySelector("span").appendChild(document.createTextNode(opts.error));
      content.appendChild(meta);
    }
    field.appendChild(content);
    return field;
  }

  // FKControlTagChooser: TagChooser over a copy of the composer's chooser,
  // existing tags only (no allowCreate). opts: { value, categoryId,
  // everyTag, maximum, placeholder, blocked(), onChange(tags) }.
  function tagChooser(opts) {
    var overrides = {
      chooser: "tag-chooser",
      allowAny: false,
      everyTag: !!opts.everyTag,
      labels: {
        optional: t("select_kit.default_header_text"),
        filter_placeholder: opts.placeholder || t("tagging.choose_for_topic"),
      },
    };
    if (opts.maximum) {
      overrides.maximum = opts.maximum;
    }
    var details = Discourse.miniTagChooser.copy(overrides);
    details.classList.add("form-kit__control-tag-chooser");
    var chooser = Discourse.miniTagChooser(details, {
      categoryId: function () {
        return opts.categoryId ? String(opts.categoryId) : "";
      },
      blocked: opts.blocked,
      changed: function () {
        opts.onChange(chooser.selected().slice());
      },
      opened: function () {
        if (Discourse.topicAdminModals) {
          Discourse.topicAdminModals.floatBody(details);
        }
      },
    });
    chooser.set(opts.value);
    return details;
  }

  // ManageTagsForm. api: { category, onPerform(operation),
  // setSubmitDisabled(value) }; returns its element and triggerSubmit.
  function manageTagsForm(api) {
    var key = function (name) {
      return t("topic_bulk_actions.manage_tags." + name);
    };
    var categoryId = api.category ? api.category.id : null;
    var data = { remove_all_tags: false, remove_tags: [], add_tags: [], replace_rows: [{ from: [], to: [] }] };
    // FormKit's errors, by field name.
    var errors = {};
    var form = document.createElement("form");
    form.className = "form-kit manage-tags-form";
    form.setAttribute("novalidate", "");

    // #replaceRowStatus
    function status(row) {
      var from = row.from[0];
      var to = row.to[0];
      if (!from && !to) {
        return "empty";
      }
      if (to && !from) {
        return "missing-from";
      }
      if (from && !to) {
        return "missing-to";
      }
      return "valid";
    }

    // #isEmpty
    function isEmpty() {
      return (
        !data.remove_all_tags &&
        data.add_tags.length === 0 &&
        data.remove_tags.length === 0 &&
        data.replace_rows.every(function (row) {
          return status(row) === "empty";
        })
      );
    }

    // validateReplaceRows
    function validate() {
      errors = {};
      data.replace_rows.forEach(function (row, index) {
        var state = status(row);
        if (state === "missing-from") {
          errors["replace_rows." + index + ".from"] = key("replace.missing_from");
        } else if (state === "missing-to") {
          errors["replace_rows." + index + ".to"] = key("replace.missing_to");
        }
      });
      return Object.keys(errors).length === 0;
    }

    // afterFieldSet; a field with an error is validated again.
    function fieldSet(name) {
      api.setSubmitDisabled(isEmpty());
      if (errors[name]) {
        validate();
        draw();
      }
    }

    // FKErrorsSummary
    function errorsSummary() {
      var names = Object.keys(errors);
      var summary = el(
        '<div aria-live="assertive" class="form-kit__errors-summary"><h2 class="form-kit__errors-summary-title">' +
          icon("triangle-exclamation") +
          ' </h2><ul class="form-kit__errors-summary-list"></ul></div>'
      );
      summary.firstChild.appendChild(document.createTextNode(t("form_kit.errors_summary_title", names.length)));
      names.forEach(function (name) {
        var id = "control-" + name.split(".").join("-");
        var item = el('<li><a rel="noopener noreferrer"></a></li>');
        item.firstChild.href = "#" + id;
        item.firstChild.textContent = key("replace.title");
        item.appendChild(document.createTextNode(": " + errors[name]));
        // focusField
        item.firstChild.addEventListener("click", function (event) {
          event.preventDefault();
          var header = form.querySelector("#" + id + " summary");
          if (header) {
            header.focus();
          }
        });
        summary.lastChild.appendChild(item);
      });
      return summary;
    }

    function removeSection() {
      var container = el(
        '<div class="form-kit__container manage-tags-form__remove-section --full"><div class="form-kit__container-content --full"></div></div>'
      );
      var content = container.firstChild;
      if (data.remove_all_tags) {
        var alert = el(
          '<div class="form-kit__alert alert alert-error manage-tags-form__warning"><span class="form-kit__alert-message"></span></div>'
        );
        alert.firstChild.innerHTML = key("remove.all_warning");
        content.appendChild(alert);
      } else {
        content.appendChild(
          fkField(
            "remove_tags",
            "tag-chooser",
            tagChooser({
              value: data.remove_tags,
              everyTag: true,
              onChange: function (tags) {
                data.remove_tags = tags;
                fieldSet("remove_tags");
              },
            }),
            { title: key("remove.title"), description: key("remove.description"), full: true }
          )
        );
      }
      // FKControlToggle (DToggleSwitch)
      var toggle = el(
        '<div class="d-toggle-switch"><label class="d-toggle-switch__label"><button class="d-toggle-switch__checkbox form-kit__control-toggle" id="manage-tags-remove-all" role="switch" type="button"></button><span class="d-toggle-switch__checkbox-slider"></span></label></div>'
      );
      var button = toggle.querySelector("button");
      button.setAttribute("aria-checked", data.remove_all_tags ? "true" : "false");
      if (data.remove_all_tags) {
        toggle.querySelector(".d-toggle-switch__checkbox-slider").innerHTML = icon("check");
      }
      button.addEventListener("click", function () {
        data.remove_all_tags = !data.remove_all_tags;
        api.setSubmitDisabled(isEmpty());
        draw();
        form.querySelector("#manage-tags-remove-all").focus();
      });
      content.appendChild(
        fkField("remove_all_tags", "toggle", { el: toggle, id: "manage-tags-remove-all" }, { title: key("remove.all_toggle") })
      );
      return container;
    }

    // A replacement row's chooser: one tag, the other side's left out
    // (blockedTagsFor).
    function replaceField(row, index, side) {
      var other = side === "from" ? "to" : "from";
      var name = "replace_rows." + index + "." + side;
      return fkField(
        name,
        "tag-chooser",
        tagChooser({
          value: row[side],
          maximum: 1,
          placeholder: key("replace." + side + "_placeholder"),
          categoryId: side === "to" ? categoryId : null,
          everyTag: side === "from" || !categoryId,
          blocked: function () {
            return row[other];
          },
          onChange: function (tags) {
            row[side] = tags;
            fieldSet(name);
          },
        }),
        { full: true, error: errors[name] }
      );
    }

    function replaceSection() {
      var container = el(
        '<div class="form-kit__container manage-tags-form__replace --full"><span class="form-kit__container-title"></span><p class="form-kit-text form-kit__container-subtitle"></p><div class="form-kit__container-content --full"></div></div>'
      );
      container.children[0].textContent = key("replace.title");
      container.children[1].textContent = key("replace.description");
      var content = container.lastChild;
      if (data.replace_rows.length) {
        var collection = el('<div class="form-kit__collection"></div>');
        data.replace_rows.forEach(function (row, index) {
          var line = el(
            '<div class="form-kit__row"><div class="form-kit__col --col-5"></div><div class="form-kit__col --col-1 manage-tags-form__replace-arrow">' +
              icon("arrow-right") +
              '</div><div class="form-kit__col --col-5"></div><div class="form-kit__col --col-1"><button class="btn no-text btn-icon form-kit__button manage-tags-form__replace-row-remove" type="button">' +
              icon("xmark") +
              '<span aria-hidden="true">&#8203;</span></button></div></div>'
          );
          line.children[0].appendChild(replaceField(row, index, "from"));
          line.children[2].appendChild(replaceField(row, index, "to"));
          var remove = line.querySelector(".manage-tags-form__replace-row-remove");
          remove.title = key("replace.remove_replacement");
          // removeReplaceRow: the errors are not re-keyed, so all go.
          remove.addEventListener("click", function () {
            data.replace_rows.splice(index, 1);
            errors = {};
            api.setSubmitDisabled(isEmpty());
            draw();
          });
          collection.appendChild(line);
        });
        content.appendChild(collection);
      }
      var add = el(
        '<button class="btn btn-icon-text form-kit__button btn-default manage-tags-form__replace-row-add" type="button">' +
          icon("plus") +
          '<span class="d-button-label"></span></button>'
      );
      add.lastChild.textContent = key("replace.add_replacement");
      // addItemToCollection
      add.addEventListener("click", function () {
        data.replace_rows.push({ from: [], to: [] });
        draw();
      });
      content.appendChild(add);
      return container;
    }

    function draw() {
      form.textContent = "";
      if (Object.keys(errors).length) {
        form.appendChild(errorsSummary());
      }
      form.appendChild(removeSection());
      form.appendChild(
        fkField(
          "add_tags",
          "tag-chooser",
          tagChooser({
            value: data.add_tags,
            categoryId: categoryId,
            everyTag: !categoryId,
            onChange: function (tags) {
              data.add_tags = tags;
              fieldSet("add_tags");
            },
          }),
          { title: key("add.title"), description: key("add.description"), full: true }
        )
      );
      form.appendChild(replaceSection());
    }

    function ids(tags) {
      return tags.map(function (tag) {
        return tag.id;
      });
    }

    // triggerSubmit, onSubmit and #buildPayload
    function submit() {
      if (!validate()) {
        draw();
        return;
      }
      api.onPerform({
        type: "manage_tags",
        add_tag_ids: ids(data.add_tags),
        remove_tag_ids: data.remove_all_tags ? [] : ids(data.remove_tags),
        remove_all_tags: data.remove_all_tags,
        replace_tags: data.replace_rows
          .filter(function (row) {
            return status(row) === "valid";
          })
          .map(function (row) {
            return { from_tag_id: row.from[0].id, to_tag_id: row.to[0].id };
          }),
      });
    }

    form.addEventListener("submit", function (event) {
      event.preventDefault();
      submit();
    });
    draw();
    return { el: form, submit: submit };
  }

  // BulkPinOptions: PinOptionSection (topic-admin-modals.js) for the
  // topics' categories and, as the menu is a topic manager's
  // (canManageTopic), for every list; `category` the selection's sole
  // one, whose pins loadFeatureStats counts.
  function bulkPinOptions(section, category, onPin) {
    var modals = Discourse.topicAdminModals;
    var carrier = { strings: config().strings };
    var stats = null;
    var paragraphs = [];

    // DConditionalLoadingSpinner (small) until the counts arrive.
    function statsParagraph(render) {
      var p = document.createElement("p");
      p.innerHTML = '<div class="spinner small"></div>';
      paragraphs.push([p, render]);
      return p;
    }

    function pinnedInCategoryStatsMessage() {
      var count = stats.pinned_in_category_count || 0;
      var text = count === 0 ? t("topic.feature_topic.not_pinned") : t("topic.feature_topic.already_pinned", count);
      return text.split("%{categoryLink}").join(category.link);
    }

    function pinnedGloballyStatsMessage() {
      if (stats.pinned_globally_count) {
        return t("topic.feature_topic.already_pinned_globally", stats.pinned_globally_count);
      }
      return t("topic.feature_topic.not_pinned_globally");
    }

    section.appendChild(
      modals.pinOptionSection(carrier, {
        buttonLabel: t("topic.bulk_feature_topic.pin_button"),
        note: t("topic.bulk_feature_topic.pin_note"),
        pinMessage: t("topic.bulk_feature_topic.pin"),
        stats: category ? statsParagraph(pinnedInCategoryStatsMessage) : null,
        onPin: function (until) {
          onPin({ pinned_globally: false, pinned_until: until });
        },
      })
    );
    section.appendChild(document.createElement("hr"));
    section.appendChild(
      modals.pinOptionSection(carrier, {
        buttonLabel: t("topic.bulk_feature_topic.pin_globally_button"),
        note: t("topic.bulk_feature_topic.pin_note"),
        pinMessage: t("topic.bulk_feature_topic.pin_globally"),
        stats: statsParagraph(pinnedGloballyStatsMessage),
        confirmMessage: function () {
          if (stats && stats.pinned_globally_count >= modals.MAX_GLOBALLY_PINNED_TOPICS) {
            return t("topic.feature_topic.confirm_pin_globally", stats.pinned_globally_count);
          }
          return null;
        },
        onPin: function (until) {
          onPin({ pinned_globally: true, pinned_until: until });
        },
      })
    );

    // loadFeatureStats
    fetch(basePath() + "/topics/feature_stats.json" + (category ? "?category_id=" + category.id : ""), {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        return r.ok ? r.json() : null;
      })
      .catch(function () {
        return null;
      })
      .then(function (result) {
        stats = result || { pinned_in_category_count: 0, pinned_globally_count: 0 };
        paragraphs.forEach(function (pair) {
          pair[0].innerHTML = pair[1]();
        });
      });
  }

  function simple(type) {
    return function () {
      return { type: type };
    };
  }

  // BulkSelectTopicsDropdown#deferTopics
  function deferTopics() {
    bulkOperation(selected.slice(), { type: "destroy_post_timing" }).then(
      function () {
        refresh("success", t("topics.bulk.completed"));
      },
      function () {
        toast("error", t("generic_error"));
      }
    );
  }

  // BulkSelectTopicsDropdown#onSelect
  function onSelect(id) {
    var description = function (name) {
      return "topic_bulk_actions." + name + ".description";
    };
    switch (id) {
      case "dismiss-unread":
        if (Discourse.topicDismiss) {
          Discourse.topicDismiss.dismissRead();
        }
        break;
      case "dismiss-new":
        if (Discourse.topicDismiss) {
          Discourse.topicDismiss.resetNew();
        }
        break;
      case "update-category":
        showModal({
          title: "topics.bulk.change_category",
          description: description("update_category"),
          confirm: "topics.bulk.confirm_update_topics",
          allowSilent: true,
          category: true,
          operation: function (state) {
            return { type: "change_category", category_id: state.categoryId };
          },
        });
        break;
      case "update-notifications":
        showModal({
          title: "topics.bulk.notification_level",
          description: description("update_notifications"),
          confirm: "topics.bulk.confirm_update_topics",
          notifications: true,
          operation: function (state) {
            return { type: "change_notification_level", notification_level_id: state.notificationLevelId };
          },
        });
        break;
      case "close-topics":
        showModal({
          title: "topics.bulk.close_topics",
          description: description("close_topics"),
          confirm: "topics.bulk.confirm_close_topics",
          closeNote: true,
          operation: simple("close"),
        });
        break;
      case "archive-topics":
        showModal({
          title: "topics.bulk.archive_topics",
          description: description("archive_topics"),
          confirm: "topics.bulk.confirm_archive_topics",
          operation: simple("archive"),
        });
        break;
      case "convert-to-private-message":
        showModal({
          title: "topics.bulk.convert_to_private_message",
          description: description("convert_to_private_message"),
          confirm: "topics.bulk.confirm_update_topics",
          allowSilent: true,
          operation: simple("convert_to_private_message"),
        });
        break;
      case "unlist-topics":
        showModal({
          title: "topics.bulk.unlist_topics",
          description: description("unlist_topics"),
          confirm: "topics.bulk.confirm_unlist_topics",
          operation: simple("unlist"),
        });
        break;
      case "relist-topics":
        showModal({
          title: "topics.bulk.relist_topics",
          confirm: "topics.bulk.confirm_relist_topics",
          operation: simple("relist"),
        });
        break;
      case "delete-topics":
        showModal({
          title: "topics.bulk.delete",
          description: description("delete_topics"),
          confirm: "topics.bulk.confirm_delete_topics",
          operation: simple("delete"),
        });
        break;
      case "reset-bump-dates":
        showModal({
          title: "topics.bulk.reset_bump_dates",
          description: description("reset_bump_dates"),
          confirm: "topics.bulk.confirm_update_topics",
          operation: simple("reset_bump_dates"),
        });
        break;
      case "unpin-topics":
        showModal({
          title: "topics.bulk.unpin_topics",
          description: description("unpin_topics"),
          confirm: "topics.bulk.confirm_unpin_topics",
          operation: simple("unpin"),
        });
        break;
      case "defer":
        deferTopics();
        break;
      case "manage-tags":
        // The choosers are copies of the composer's.
        if (!Discourse.miniTagChooser || !Discourse.miniTagChooser.copy()) {
          unported("the bulk actions' Manage tags without the composer's tag chooser");
          break;
        }
        showModal({
          title: "topics.bulk.manage_tags",
          confirm: "topics.bulk.confirm_apply_to_topics",
          allowSilent: true,
          manageTags: true,
        });
        break;
      case "pin-topics":
        // DFutureDateInput is penalize-user.js', moment loaded first.
        if (!Discourse.futureDateInput || !Discourse.topicAdminModals) {
          unported("the bulk actions' Pin without its date input");
          break;
        }
        Discourse.futureDateInput.load().then(function () {
          showModal({ title: "topics.bulk.pin_topics", pin: true, showFooter: false });
        });
        break;
      case "enable-nested-replies":
        unported("the bulk actions' Enable nested replies (NestedTopic::Toggle)");
        break;
    }
  }

  // For topic-dismiss.js: the selected topics its buttons count and send.
  Discourse.bulkSelectTopics = {
    selected: function () {
      return enabled ? selected.slice() : [];
    },
  };

  // Before the rows' links and their boosts: in the mode a click on a row
  // is a selection (Item#click, Item#onBulkSelectToggle).
  document.addEventListener(
    "click",
    function (event) {
      if (!enabled || !(event.target instanceof Element)) {
        return;
      }
      var row = event.target.closest("#list-area tbody.topic-list-body > tr.topic-list-item.bulk-selecting");
      if (!row) {
        return;
      }
      if (event.target.matches("input.bulk-select")) {
        event.stopPropagation();
        if (event.target.checked) {
          selectTopic(row, event.shiftKey);
        } else {
          unselectTopic(row);
        }
        return;
      }
      if (wantsNewWindow(event)) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      if (selected.indexOf(Number(row.dataset.topicId)) !== -1) {
        unselectTopic(row);
      } else {
        selectTopic(row, event.shiftKey);
      }
    },
    true
  );

  document.addEventListener("click", function (event) {
    if (!(event.target instanceof Element) || !config()) {
      return;
    }
    if (
      event.target.closest(
        "#list-area thead.topic-list-header button.bulk-select, .list-controls .navigation-controls > button.bulk-select"
      )
    ) {
      event.preventDefault();
      toggle();
      return;
    }
    if (!enabled) {
      return;
    }
    var trigger = event.target.closest("#list-area thead .bulk-select-topics-dropdown-trigger");
    if (trigger) {
      openMenu(trigger);
    } else if (event.target.closest("#list-area thead .bulk-select-all")) {
      // SortableColumn#bulkSelectAll
      document.querySelectorAll("input.bulk-select:not(:checked)").forEach(function (input) {
        input.click();
      });
    } else if (event.target.closest("#list-area thead .bulk-clear-all")) {
      // SortableColumn#bulkClearAll
      document.querySelectorAll("input.bulk-select:checked").forEach(function (input) {
        input.click();
      });
    }
  });

  document.addEventListener("discourse:before-redraw", function () {
    redrawing = true;
  });
  document.addEventListener("discourse:redrawn", function () {
    redrawing = false;
    var done = refreshing;
    refreshing = null;
    if (done && done.toast) {
      toast(done.toast[0], done.toast[1]);
    }
  });

  // The mode is the page's: gone with the list it was turned on for. One
  // turned on while an action's refresh was on its way goes back on over
  // the list drawn again, nothing selected: the BulkSelectHelper outlives
  // router.refresh, whose setupController clears its selection.
  Discourse.onPage(function () {
    if (resume) {
      resume = false;
      enable();
    }
    return function () {
      resume = redrawing && !!refreshing && enabled;
      // Another page: the refresh was this list's.
      if (!redrawing) {
        refreshing = null;
      }
      enabled = false;
      selected = [];
      lastChecked = null;
      headerStash = [];
      closeMenu();
      if (observer) {
        observer.disconnect();
        observer = null;
      }
    };
  });
})();
