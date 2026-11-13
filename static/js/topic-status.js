// TopicStatus' pin toggle for a member (components/topic-status.gjs,
// a.pin-toggle-button): togglePinned sends Topic#togglePinnedForUser,
//
//   pinned:   clearPin, PUT /t/:id/clear-pin, back to pinned on an error
//   unpinned: rePin, PUT /t/:id/re-pin
//
// with the status drawn for the new state at once, wherever the page
// shows the topic. As in Ember a list keeps its order until it loads
// again; the row follows `pinned` in its class and its pinned excerpt
// (TopicListItem#expandPinned).
//
// The links sit under hx-boost with an empty href, so the click is taken
// in the capture phase, before htmx's own listener on the link.
(function () {
  "use strict";

  if (!Discourse.once("topic-status")) {
    return;
  }

  var script = document.currentScript;
  var titles = {
    pinned: script.dataset.pinned || "",
    unpinned: script.dataset.unpinned || "",
  };
  // A row's excerpt while its topic is unpinned: [node, parent, next].
  var excerpts = new WeakMap();

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  function headers() {
    var h = {};
    try {
      h = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (_) {}
    h["X-Requested-With"] = "XMLHttpRequest";
    return h;
  }

  // The topic a status belongs to: its row's or heading's, else the
  // title link's beside it (the header).
  function topicId(link) {
    var statuses = link.closest(".topic-statuses");
    if (!statuses) {
      return null;
    }
    var holder = statuses.closest("[data-topic-id]");
    if (!holder && statuses.parentNode) {
      holder = statuses.parentNode.querySelector("[data-topic-id]");
    }
    return holder ? holder.getAttribute("data-topic-id") : null;
  }

  function setLink(link, pinned) {
    link.className = "topic-status " + (pinned ? "--pinned" : "--unpinned") + " pin-toggle-button";
    link.title = pinned ? titles.pinned : titles.unpinned;
    var icon = link.querySelector(".d-icon-thumbtack");
    if (icon) {
      icon.classList.toggle("unpinned", !pinned);
    }
  }
  // For the docked header's copy (header-topic-info.js).
  Discourse.setPinToggle = setLink;

  function setRow(row, pinned) {
    row.classList.toggle("pinned", pinned);
    var kept = excerpts.get(row);
    if (pinned && kept) {
      excerpts.delete(row);
      if (kept[1].isConnected) {
        kept[1].insertBefore(kept[0], kept[2] && kept[2].parentNode === kept[1] ? kept[2] : null);
        row.classList.add("excerpt-expanded");
      }
    } else if (!pinned) {
      var excerpt = row.querySelector(".topic-excerpt");
      if (excerpt) {
        excerpts.set(row, [excerpt, excerpt.parentNode, excerpt.nextSibling]);
        excerpt.remove();
        row.classList.remove("excerpt-expanded");
      }
    }
  }

  function draw(id, pinned) {
    document.querySelectorAll("a.pin-toggle-button").forEach(function (link) {
      if (topicId(link) !== id) {
        return;
      }
      setLink(link, pinned);
      var row = link.closest(".topic-list-item, .latest-topic-list-item");
      if (row) {
        setRow(row, pinned);
      }
    });
  }

  document.addEventListener(
    "click",
    function (event) {
      var link = event.target.closest && event.target.closest("a.pin-toggle-button");
      var id = link && topicId(link);
      if (!id) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();
      var pinned = link.classList.contains("--pinned");
      draw(id, !pinned);
      fetch(basePath() + "/t/" + id + (pinned ? "/clear-pin" : "/re-pin"), {
        method: "PUT",
        headers: headers(),
        credentials: "same-origin",
      })
        .then(function (r) {
          if (!r.ok) {
            throw new Error(String(r.status));
          }
        })
        .catch(function () {
          // On an error either way Ember's topic is pinned again.
          draw(id, true);
        });
    },
    true
  );
})();
