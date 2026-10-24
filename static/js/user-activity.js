// The activity tabs drawn by user_activity_view.rs:
//
//   drafts     PostList's bulk select (PostListBulkControls, the bulk
//              actions DMenu with Delete selected), resuming a draft
//              (its post, or the composer for a new topic) and removing
//              one, as UserStream does
//   pending    deleting one's queued post (DELETE /review/:id)
//   bookmarks  the search, BookmarkActionsDropdown (delete, clear the
//              reminder, pin; editing is BookmarkModal's), BookmarkList's
//              bulk select and BulkSelectBookmarksDropdown with its toasts
//
// What the route would reload (router.refresh) is fetched again and
// swapped in. Strings come from #user-activity-strings.
(function () {
  "use strict";

  if (!Discourse.once("user-activity")) {
    return;
  }

  function strings() {
    var el = document.getElementById("user-activity-strings");
    try {
      return el ? JSON.parse(el.textContent) : {};
    } catch (_) {
      return {};
    }
  }

  function meta(name) {
    return (document.querySelector('meta[name="' + name + '"]') || {}).content || "";
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

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function counted(key, count) {
    var s = strings();
    var text = count === 1 ? s[key + "_one"] : s[key + "_other"];
    return String(text || "").replace("%{count}", count);
  }

  // ajax: form data, or JSON when `json` is given.
  function request(method, url, form, json) {
    var headers = {
      "X-CSRF-Token": meta("csrf-token"),
      "X-Requested-With": "XMLHttpRequest",
      Accept: "application/json",
    };
    var body;
    if (json) {
      headers["Content-Type"] = "application/json";
      body = JSON.stringify(json);
    } else if (form) {
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      body = form;
    }
    return fetch(meta("discourse-base-uri") + url, {
      method: method,
      credentials: "same-origin",
      headers: headers,
      body: body,
    }).then(function (response) {
      return response.text().then(function (text) {
        var parsed = null;
        try {
          parsed = text ? JSON.parse(text) : null;
        } catch (_) {
          parsed = null;
        }
        if (!response.ok) {
          var errors = parsed && parsed.errors;
          throw new Error(
            Array.isArray(errors) ? errors.join(" ") : errors || response.status + " " + response.statusText
          );
        }
        return parsed;
      });
    });
  }

  function popupAjaxError(e) {
    Discourse.dialog.popupAjaxError(e.message);
  }

  function unported(what) {
    Discourse.dialog.alert(escapeHtml("Not ported yet: " + what + "."));
  }

  // router.refresh: the route's content drawn again.
  function refresh() {
    return fetch(location.href, { credentials: "same-origin" })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var fresh = doc.getElementById("user-content");
        var current = document.getElementById("user-content");
        if (fresh && current) {
          current.replaceWith(document.adoptNode(fresh));
        }
        var nav = doc.querySelector(".user-navigation-secondary");
        var currentNav = document.querySelector(".user-navigation-secondary");
        if (nav && currentNav) {
          currentNav.replaceWith(document.adoptNode(nav));
        }
        if (bookmarksBulk) {
          enableBookmarksBulk();
        }
        watchSentinel();
      });
  }

  // toasts.success({ duration: "short" })
  function toast(text) {
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
      icon("check") +
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

  function float(anchor, body, placement) {
    var ui = window.FloatingUIDOM;
    if (!ui) {
      return;
    }
    ui.computePosition(anchor, body, {
      placement: placement,
      strategy: body.style.position === "fixed" ? "fixed" : "absolute",
      middleware: [ui.offset(placement.indexOf("bottom") === 0 ? 3 : 10), ui.flip({ padding: 3 }), ui.shift({ padding: 10 })],
    }).then(function (result) {
      if (body.style.position === "fixed") {
        body.style.transform = "translate(" + Math.round(result.x) + "px," + Math.round(result.y) + "px)";
      } else {
        body.style.left = result.x + "px";
        body.style.top = result.y + "px";
      }
      body.dataset.placement = result.placement;
      body.style.visibility = "visible";
    });
  }

  // --- topic lists and bookmarks: sorting and the next page ---------------

  // UserTopicsListController#changeSort: the order again flips ascending,
  // another starts descending (the query params in Ember's order).
  function changeSort(th) {
    var current = new URLSearchParams(location.search);
    var order = current.get("order") || "activity";
    var ascending = current.get("ascending") === "true";
    var sortBy = th.dataset.sortOrder;
    var next = new URLSearchParams();
    next.set("ascending", String(sortBy === order ? !ascending : false));
    next.set("order", sortBy);
    current.forEach(function (value, key) {
      if (key !== "order" && key !== "ascending") {
        next.append(key, value);
      }
    });
    location.href = location.pathname + "?" + next.toString();
  }

  var loadingMore = false;
  var observer = null;

  function watchSentinel() {
    if (observer) {
      observer.disconnect();
      observer = null;
    }
    var sentinel = document.querySelector("#user-content .load-more-sentinel");
    if (!sentinel || !window.IntersectionObserver) {
      return;
    }
    observer = new IntersectionObserver(function (entries) {
      var seen = entries.some(function (e) {
        return e.isIntersecting;
      });
      if (seen) {
        loadMore();
      }
    });
    observer.observe(sentinel);
  }

  // DLoadMore: the next page's rows appended once the sentinel shows.
  function loadMore() {
    var sentinel = document.querySelector("#user-content .load-more-sentinel[data-next-page]");
    if (!sentinel || loadingMore) {
      return;
    }
    loadingMore = true;
    var params = new URLSearchParams(location.search);
    params.set("page", sentinel.dataset.nextPage);
    fetch(location.pathname + "?" + params.toString(), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        var doc = new DOMParser().parseFromString(html, "text/html");
        var body = document.querySelector("#user-content tbody.topic-list-body");
        if (body) {
          doc.querySelectorAll("#user-content tbody.topic-list-body > tr").forEach(function (row) {
            body.appendChild(document.adoptNode(row));
          });
        }
        if (bookmarksBulk) {
          enableBookmarksBulk();
        }
        var fresh = doc.querySelector("#user-content .load-more-sentinel[data-next-page]");
        if (fresh) {
          sentinel.dataset.nextPage = fresh.dataset.nextPage;
        } else {
          delete sentinel.dataset.nextPage;
        }
      })
      .catch(popupAjaxError)
      .finally(function () {
        loadingMore = false;
        var rect = sentinel.getBoundingClientRect();
        if (sentinel.dataset.nextPage && rect.top < window.innerHeight) {
          loadMore();
        }
      });
  }

  // --- drafts -------------------------------------------------------------

  var lastDraft = null;

  function draftItems() {
    return Array.prototype.slice.call(document.querySelectorAll(".post-list--bulk-select > .post-list-item"));
  }

  function selectedDrafts() {
    return draftItems().filter(function (item) {
      return item.classList.contains("post-list-item--selected");
    });
  }

  function setDraftSelected(item, on) {
    item.classList.toggle("post-list-item--selected", on);
    item.querySelector(".bulk-select-checkbox").checked = on;
  }

  function closeDraftsMenu() {
    var content = document.querySelector(".post-list-bulk-actions-dropdown-content");
    if (content) {
      content.remove();
    }
    var trigger = document.querySelector(".post-list-bulk-controls .bulk-actions-dropdown");
    if (trigger) {
      trigger.setAttribute("aria-expanded", "false");
      trigger.classList.remove("-expanded");
    }
  }

  function syncDraftControls() {
    var list = document.querySelector(".post-list--bulk-select");
    if (!list) {
      return;
    }
    var count = selectedDrafts().length;
    var controls = document.querySelector(".post-list-bulk-controls");
    if (!count) {
      closeDraftsMenu();
      if (controls) {
        controls.remove();
      }
      return;
    }
    var s = strings();
    if (!controls) {
      controls = document.createElement("div");
      controls.className = "post-list-bulk-controls";
      controls.innerHTML =
        '<div class="post-list-bulk-controls__selection"><div class="post-list-bulk-controls__count"></div><div class="post-list-bulk-controls__actions">' +
        '<button class="btn btn-transparent bulk-clear-all" type="button"><span class="d-button-label">' +
        escapeHtml(s.post_list_clear_all) +
        '</span></button><button class="btn btn-transparent bulk-select-all" type="button"><span class="d-button-label">' +
        escapeHtml(s.post_list_select_all) +
        '</span></button><button aria-expanded="false" class="btn btn-icon-text fk-d-menu__trigger post-list-bulk-actions-dropdown-trigger bulk-actions-dropdown" data-identifier="post-list-bulk-actions-dropdown" data-trigger="" type="button">' +
        icon("chevron-down") +
        '<span class="d-button-label">' +
        escapeHtml(s.post_list_actions) +
        "</span></button></div></div>";
      list.parentNode.insertBefore(controls, list);
    }
    controls.querySelector(".post-list-bulk-controls__count").textContent = counted("post_list_selected", count);
  }

  function openDraftsMenu(trigger) {
    var s = strings();
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu post-list-bulk-actions-dropdown-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.content = "";
    content.dataset.identifier = "post-list-bulk-actions-dropdown";
    content.dataset.strategy = "absolute";
    content.style.position = "absolute";
    content.style.maxWidth = "min(400px, -20px + 100dvw)";
    content.style.visibility = "hidden";
    content.innerHTML =
      '<div class="fk-d-menu__inner-content"><ul class="dropdown-menu"><li class="dropdown-menu__item"><button class="btn btn-icon-text btn-transparent btn-danger" type="button">' +
      icon("trash-can") +
      '<span class="d-button-label">' +
      escapeHtml(s.drafts_bulk_delete) +
      "</span></button></li></ul></div>";
    portals.appendChild(content);
    trigger.setAttribute("aria-expanded", "true");
    trigger.classList.add("-expanded");
    float(trigger, content, "top-end");
  }

  function draftCountChanged(removed) {
    var s = strings();
    var label = document.querySelector(".user-nav__activity-drafts a span");
    if (!label) {
      return;
    }
    var count = (parseInt((label.textContent.match(/\d+/) || ["0"])[0], 10) || 0) - removed;
    label.textContent =
      count > 0 ? String(s.drafts_label_with_count).replace("%{count}", count) : s.drafts_label;
  }

  function draftsRemoved(items) {
    items.forEach(function (item) {
      item.remove();
    });
    draftCountChanged(items.length);
    syncDraftControls();
    if (!draftItems().length) {
      refresh();
    }
  }

  function bulkDeleteDrafts() {
    var s = strings();
    var items = selectedDrafts();
    closeDraftsMenu();
    Discourse.dialog.deleteConfirm({
      title: s.drafts_bulk_delete_confirmation,
      message: escapeHtml(counted("drafts_bulk_delete_message", items.length)),
      didConfirm: function () {
        var form = [];
        items.forEach(function (item) {
          var key = item.dataset.draftKey;
          form.push("draft_keys%5B%5D=" + encodeURIComponent(key));
          form.push(
            "sequences%5B" + encodeURIComponent(key) + "%5D=" + encodeURIComponent(item.dataset.draftSequence)
          );
        });
        request("DELETE", "/drafts/bulk_destroy.json", form.join("&"))
          .then(function () {
            draftsRemoved(items);
          })
          .catch(popupAjaxError);
      },
    });
  }

  function removeDraft(item) {
    var s = strings();
    Discourse.dialog.deleteConfirm({
      title: s.drafts_remove_confirmation,
      didConfirm: function () {
        var key = item.dataset.draftKey;
        request(
          "DELETE",
          "/drafts/" + encodeURIComponent(key) + ".json",
          "draft_key=" + encodeURIComponent(key) + "&sequence=" + encodeURIComponent(item.dataset.draftSequence)
        )
          .then(function () {
            draftsRemoved([item]);
          })
          .catch(popupAjaxError);
      },
    });
  }

  function resumeDraft(item) {
    var link = item.querySelector(".stream-topic-title .title a");
    if (link) {
      location.href = link.href;
      return;
    }
    var key = item.dataset.draftKey;
    if (!window.composer) {
      unported("the composer on this page");
      return;
    }
    window.composer.open({
      mode: key.indexOf("new_private_message") === 0 ? "private-message" : "create-topic",
      draftKey: key,
      draftSequence: parseInt(item.dataset.draftSequence, 10) || 0,
    });
  }

  function onDraftsClick(event) {
    var box = event.target.closest(".post-list--bulk-select .bulk-select-checkbox");
    if (box) {
      var item = box.closest(".post-list-item");
      var on = box.checked;
      if (event.shiftKey && lastDraft && lastDraft.isConnected) {
        var items = draftItems();
        var from = items.indexOf(lastDraft);
        var to = items.indexOf(item);
        items.slice(Math.min(from, to), Math.max(from, to) + 1).forEach(function (i) {
          setDraftSelected(i, true);
        });
      } else {
        setDraftSelected(item, on);
      }
      lastDraft = item;
      syncDraftControls();
      return true;
    }
    if (event.target.closest(".post-list-bulk-controls .bulk-clear-all")) {
      draftItems().forEach(function (i) {
        setDraftSelected(i, false);
      });
      lastDraft = null;
      syncDraftControls();
      return true;
    }
    if (event.target.closest(".post-list-bulk-controls .bulk-select-all")) {
      draftItems().forEach(function (i) {
        setDraftSelected(i, true);
      });
      syncDraftControls();
      return true;
    }
    var trigger = event.target.closest(".post-list-bulk-controls .bulk-actions-dropdown");
    if (trigger) {
      if (document.querySelector(".post-list-bulk-actions-dropdown-content")) {
        closeDraftsMenu();
      } else {
        openDraftsMenu(trigger);
      }
      return true;
    }
    if (event.target.closest(".post-list-bulk-actions-dropdown-content .btn-danger")) {
      bulkDeleteDrafts();
      return true;
    }
    if (!event.target.closest(".post-list-bulk-actions-dropdown-content")) {
      closeDraftsMenu();
    }
    var resume = event.target.closest(".post-list-item .resume-draft, .post-list-item .draft-title");
    if (resume) {
      resumeDraft(resume.closest(".post-list-item"));
      return true;
    }
    var remove = event.target.closest(".post-list-item .remove-draft");
    if (remove) {
      removeDraft(remove.closest(".post-list-item"));
      return true;
    }
    return false;
  }

  // --- pending ------------------------------------------------------------

  function onPendingClick(event) {
    var button = event.target.closest(".post-list-item[data-pending-id] .reviewable-actions .btn-danger");
    if (!button) {
      return false;
    }
    var item = button.closest(".post-list-item");
    Discourse.dialog.deleteConfirm({
      message: escapeHtml(strings().review_delete_confirm),
      didConfirm: function () {
        request("DELETE", "/review/" + encodeURIComponent(item.dataset.pendingId))
          .then(function () {
            var list = item.parentNode;
            item.remove();
            if (!list.querySelector(".post-list-item")) {
              location.href = location.pathname.replace(/\/pending\/?$/, "");
            }
          })
          .catch(popupAjaxError);
      },
    });
    return true;
  }

  // --- bookmarks ----------------------------------------------------------

  var bookmarksBulk = false;
  var lastBookmark = null;

  function bookmarkId(el) {
    return el.closest("tr.bookmark-list-item").dataset.bookmarkId;
  }

  function search() {
    var input = document.getElementById("bookmark-search");
    var q = input ? input.value : "";
    location.href = location.pathname + (q ? "?q=" + encodeURIComponent(q) : "");
  }

  function selectKitRow(index, value, name, iconName, description) {
    return (
      '<li aria-checked="false" role="menuitemradio" data-index="' +
      index +
      '" data-name="' +
      escapeHtml(name) +
      '" data-value="' +
      value +
      '" title="' +
      escapeHtml(name) +
      '" tabindex="0" class="select-kit-row dropdown-select-box-row"><div class="icons"><span class="selection-indicator"></span>' +
      icon(iconName) +
      '</div><div class="texts"><span class="name">' +
      escapeHtml(name) +
      "</span>" +
      (description ? '<span class="desc">' + escapeHtml(description) + "</span>" : "") +
      "</div></li>"
    );
  }

  function bulkHeader() {
    var s = strings();
    return (
      '<span class="bulk-select-topics"><div class="bulk-select-bookmarks-dropdown" hidden><span class="bulk-select-bookmark-dropdown__count"></span> ' +
      '<details class="select-kit single-select dropdown-select-box bulk-select-bookmarks-dropdown"><summary aria-label="Filter by: ' +
      escapeHtml(s.bookmarks_bulk_title) +
      '" name="Filter by: ' +
      escapeHtml(s.bookmarks_bulk_title) +
      '" data-name="' +
      escapeHtml(s.bookmarks_bulk_title) +
      '" tabindex="0" class="select-kit-header single-select-header dropdown-select-box-header btn btn-icon-text btn-default"><div class="select-kit-header-wrapper"><div class="select-kit-selected-name selected-name choice" data-name="' +
      escapeHtml(s.bookmarks_bulk_title) +
      '" title="' +
      escapeHtml(s.bookmarks_bulk_title) +
      '"><span class="name">' +
      escapeHtml(s.bookmarks_bulk_title) +
      "</span></div>" +
      icon("angle-down", "angle-icon") +
      ' &#8203;</div></summary><div class="select-kit-body"><div class="select-kit-filter"></div><ul aria-live="polite" class="select-kit-collection" role="menu">' +
      selectKitRow(0, "clear-reminders", s.bookmarks_clear_reminders, "tag") +
      selectKitRow(1, "delete-bookmarks", s.bookmarks_delete_bookmarks, "trash-can") +
      '</ul></div></details></div><button class="btn btn btn-default bulk-select-all" type="button"><span class="d-button-label">' +
      escapeHtml(s.bookmarks_select_all) +
      '</span></button><button class="btn btn btn-default bulk-clear-all" type="button"><span class="d-button-label">' +
      escapeHtml(s.bookmarks_clear_all) +
      "</span></button></span>"
    );
  }

  function enableBookmarksBulk() {
    var table = document.querySelector("table.bookmark-list");
    if (!table) {
      return;
    }
    var head = table.querySelector("thead tr");
    var first = head && head.querySelector("th");
    if (first && !first.classList.contains("bulk-select")) {
      var toggle = first.querySelector("button.bulk-select");
      toggle.className = "btn no-text btn-icon bulk-select btn-flat";
      var th = document.createElement("th");
      th.className = "bulk-select topic-list-data";
      th.appendChild(toggle);
      first.innerHTML = bulkHeader();
      head.insertBefore(th, first);
    }
    table.querySelectorAll("tbody tr.bookmark-list-item").forEach(function (row) {
      if (row.querySelector("td.bulk-select")) {
        return;
      }
      var id = row.dataset.bookmarkId;
      var td = document.createElement("td");
      td.className = "bulk-select bookmark-list-data";
      td.innerHTML =
        '<label for="bulk-select-' +
        id +
        '"><input class="bulk-select" data-id="' +
        id +
        '" id="bulk-select-' +
        id +
        '" type="checkbox"></label>';
      row.insertBefore(td, row.firstChild);
    });
    syncBookmarksCount();
  }

  function disableBookmarksBulk() {
    var table = document.querySelector("table.bookmark-list");
    if (!table) {
      return;
    }
    var s = strings();
    var th = table.querySelector("thead th.bulk-select");
    if (th) {
      var toggle = th.querySelector("button.bulk-select");
      toggle.className = "btn no-text btn-icon btn-flat bulk-select";
      var next = th.nextElementSibling;
      next.innerHTML = "";
      next.appendChild(toggle);
      next.appendChild(document.createTextNode(s.topic_title));
      th.remove();
    }
    table.querySelectorAll("td.bulk-select").forEach(function (td) {
      td.remove();
    });
  }

  function selectedBookmarks() {
    return Array.prototype.slice.call(document.querySelectorAll("table.bookmark-list input.bulk-select:checked"));
  }

  function syncBookmarksCount() {
    var box = document.querySelector("th .bulk-select-topics > .bulk-select-bookmarks-dropdown");
    if (!box) {
      return;
    }
    var count = selectedBookmarks().length;
    box.hidden = !count;
    box.querySelector(".bulk-select-bookmark-dropdown__count").textContent = counted("bookmarks_selected", count);
    if (!count) {
      box.querySelector("details").open = false;
    }
  }

  function bulkBookmarks(type, confirmKey, toastKey, dialog) {
    var ids = selectedBookmarks().map(function (input) {
      return parseInt(input.dataset.id, 10);
    });
    dialog({
      message: counted(confirmKey, ids.length),
      didConfirm: function () {
        request("PUT", "/bookmarks/bulk", null, { bookmark_ids: ids, operation: { type: type } })
          .then(function () {
            return refresh();
          })
          .then(function () {
            toast(strings()[toastKey]);
          })
          .catch(popupAjaxError);
      },
    });
  }

  function bookmarkAction(row, value) {
    var id = bookmarkId(row);
    var tr = row.closest("tr.bookmark-list-item");
    if (value === "remove") {
      var destroy = function () {
        request("DELETE", "/bookmarks/" + encodeURIComponent(id) + ".json")
          .then(refresh)
          .catch(popupAjaxError);
      };
      if (tr.dataset.hasReminder !== "true") {
        destroy();
        return;
      }
      Discourse.dialog.deleteConfirm({
        message: escapeHtml(strings().bookmarks_confirm_delete),
        didConfirm: destroy,
      });
    } else if (value === "edit") {
      unported("editing a bookmark (BookmarkModal, 196)");
    } else if (value === "clear_reminder") {
      request("PUT", "/bookmarks/" + encodeURIComponent(id), "reminder_at=")
        .then(refresh)
        .catch(popupAjaxError);
    } else if (value === "pin") {
      request("PUT", "/bookmarks/" + encodeURIComponent(id) + "/toggle_pin")
        .then(refresh)
        .catch(popupAjaxError);
    }
  }

  function onBookmarksClick(event) {
    if (event.target.closest(".bookmark-search-form .btn-primary")) {
      search();
      return true;
    }
    var row = event.target.closest(".bookmark-list .select-kit-row");
    document.querySelectorAll(".bookmark-list details.select-kit[open]").forEach(function (d) {
      if (!d.contains(event.target) || row) {
        d.open = false;
      }
    });
    if (row) {
      var chooser = row.closest("details");
      if (chooser.classList.contains("bookmark-actions-dropdown")) {
        bookmarkAction(chooser, row.dataset.value);
      } else if (row.dataset.value === "clear-reminders") {
        bulkBookmarks("clear_reminder", "bookmarks_clear_reminders", "bookmarks_reminders_cleared", Discourse.dialog.yesNoConfirm);
      } else if (row.dataset.value === "delete-bookmarks") {
        bulkBookmarks("delete", "bookmarks_delete_bookmarks", "bookmarks_delete_completed", Discourse.dialog.deleteConfirm);
      }
      return true;
    }
    if (event.target.closest(".bookmark-list button.bulk-select")) {
      bookmarksBulk = !bookmarksBulk;
      lastBookmark = null;
      if (bookmarksBulk) {
        enableBookmarksBulk();
      } else {
        disableBookmarksBulk();
      }
      return true;
    }
    var box = event.target.closest(".bookmark-list input.bulk-select");
    if (box) {
      if (box.checked && event.shiftKey && lastBookmark && lastBookmark.isConnected) {
        var boxes = Array.prototype.slice.call(document.querySelectorAll(".bookmark-list input.bulk-select"));
        var from = boxes.indexOf(box);
        var to = boxes.indexOf(lastBookmark);
        boxes.slice(Math.min(from, to), Math.max(from, to)).forEach(function (b) {
          b.checked = true;
        });
      }
      lastBookmark = box.checked ? box : null;
      syncBookmarksCount();
      return true;
    }
    if (event.target.closest(".bookmark-list .bulk-select-all")) {
      document.querySelectorAll(".bookmark-list input.bulk-select").forEach(function (b) {
        b.checked = true;
      });
      syncBookmarksCount();
      return true;
    }
    if (event.target.closest(".bookmark-list .bulk-clear-all")) {
      document.querySelectorAll(".bookmark-list input.bulk-select").forEach(function (b) {
        b.checked = false;
      });
      lastBookmark = null;
      syncBookmarksCount();
      return true;
    }
    return false;
  }

  // The select-kit bodies float under their headers.
  document.addEventListener(
    "toggle",
    function (event) {
      var chooser = event.target;
      if (!chooser.matches || !chooser.matches(".bookmark-list details.select-kit")) {
        return;
      }
      var open = chooser.open;
      chooser.classList.toggle("is-expanded", open);
      var caret = chooser.querySelector(".angle-icon");
      if (caret) {
        caret.outerHTML = icon(open ? "angle-up" : "angle-down", "angle-icon");
      }
      var body = chooser.querySelector(".select-kit-body");
      if (!open || !body) {
        return;
      }
      body.style.position = "fixed";
      body.style.top = "0";
      body.style.left = "0";
      body.style.minWidth = "220px";
      float(chooser.querySelector("summary"), body, "bottom-start");
    },
    true
  );

  document.addEventListener("click", function (event) {
    if (!document.getElementById("user-activity-strings")) {
      return;
    }
    var th = event.target.closest("#user-content .topic-list th.sortable");
    if (th) {
      event.preventDefault();
      changeSort(th);
      return;
    }
    onDraftsClick(event) || onPendingClick(event) || onBookmarksClick(event);
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Enter" && event.target.id === "bookmark-search") {
      event.preventDefault();
      search();
    } else if (event.key === "Escape") {
      closeDraftsMenu();
    }
  });

  Discourse.onPage(function () {
    bookmarksBulk = false;
    lastBookmark = null;
    lastDraft = null;
    watchSentinel();
    return function () {
      if (observer) {
        observer.disconnect();
        observer = null;
      }
    };
  });
})();
