// discourse-topic-voting's vote box beside a topic's title (VoteBox,
// VoteButton, VoteCount): the button votes and opens its menu (votes left,
// remove the vote, watch the topic), the count opens the voters, and an
// anonymous reader is sent to log in.
(function () {
  "use strict";

  var open = null;

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    headers["X-Requested-With"] = "XMLHttpRequest";
    return headers;
  }

  function escapeHtml(s) {
    var div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" ' +
      'xmlns="http://www.w3.org/2000/svg"><use href="#' + name + '"></use></svg>'
    );
  }

  function state(box) {
    var d = box.dataset;
    return {
      topicId: d.topicId,
      base: d.basePath || "",
      voted: d.userVoted === "true",
      closed: d.closed === "true",
      exceeded: d.votesExceeded === "true",
      limit: d.voteLimit === "" ? null : Number(d.voteLimit),
      left: d.votesLeft === "" ? null : Number(d.votesLeft),
      watching: d.watching === "true",
      count: Number(d.voteCount),
      labels: JSON.parse(d.labels || "{}"),
    };
  }

  function post(box, path, params) {
    var s = state(box);
    return fetch(s.base + path, {
      method: "POST",
      credentials: "same-origin",
      headers: csrfHeaders(),
      body: new URLSearchParams(params).toString(),
    }).then(function (r) {
      return r.json().then(function (json) {
        return { ok: r.ok, status: r.status, json: json };
      });
    });
  }

  // VoteBox#sendVote's bookkeeping, and the button and count redrawn.
  function applyResult(box, result, voted) {
    var d = box.dataset;
    d.userVoted = String(voted);
    d.voteCount = String(result.vote_count);
    d.votesExceeded = String(!result.can_vote);
    d.voteLimit = result.vote_limit == null ? "" : String(result.vote_limit);
    d.votesLeft = result.votes_left == null ? "" : String(result.votes_left);
    var s = state(box);
    var button = box.querySelector(".voting-wrapper__button");
    var locked = s.limit === 0;
    button.classList.toggle("btn-success", voted && !locked);
    button.classList.toggle("btn-default", !voted || locked);
    var label = voted ? s.labels.remove_vote : s.labels.vote_title;
    button.setAttribute("aria-label", label);
    button.setAttribute("title", label);
    button.innerHTML = icon(voted ? "vote-up-filled" : "vote-up");
    var count = box.querySelector(".voting-wrapper__count");
    count.classList.toggle("no-votes", s.count === 0);
    count.querySelector(".voting-wrapper__count-text").textContent = String(s.count);
  }

  function close() {
    if (!open) {
      return;
    }
    open.trigger.setAttribute("aria-expanded", "false");
    open.trigger.classList.remove("-expanded");
    open.content.remove();
    open = null;
  }

  // A DMenu's content, floated to the right of its trigger.
  function openMenu(trigger, identifier, html) {
    close();
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu " + identifier + "-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.identifier = identifier;
    content.dataset.content = "";
    content.dataset.strategy = "absolute";
    content.dataset.placement = "right";
    content.innerHTML = '<div class="fk-d-menu__inner-content">' + html + "</div>";
    portals.appendChild(content);
    var rect = trigger.getBoundingClientRect();
    content.style.position = "absolute";
    content.style.left = rect.right + window.scrollX + 10 + "px";
    content.style.top = rect.top + window.scrollY + "px";
    trigger.setAttribute("aria-expanded", "true");
    trigger.classList.add("-expanded");
    open = { trigger: trigger, content: content };
    return content;
  }

  function item(cls, inner) {
    return '<li class="dropdown-menu__item ' + cls + '">' + inner + "</li>";
  }

  function rowButton(cls, iconName, label, href) {
    var body = icon(iconName) + '<span class="d-button-label">' + escapeHtml(label) + "</span>";
    var classes = "btn btn-icon-text btn-transparent " + cls + " topic-voting-menu__row-btn";
    if (href) {
      return '<a class="' + classes + '" href="' + escapeHtml(href) + '">' + body + "</a>";
    }
    return '<button class="' + classes + '" type="button">' + body + "</button>";
  }

  // VoteButton's menu.
  function showVoteMenu(box, trigger, justVoted) {
    var s = state(box);
    var items = [];
    if (s.limit === 0) {
      items.push(
        item(
          "topic-voting-menu__title --locked",
          icon("lock") + "<span>" + escapeHtml(s.labels.locked_description) + "</span>"
        )
      );
    } else {
      if (s.limit !== null) {
        var seeVotes = s.labels.see_votes
          .replace("%{count}", String(s.left))
          .replace("%{max}", String(s.limit));
        items.push(
          item(
            "topic-voting-menu__votes-left",
            rowButton("see-votes", "check-to-slot", seeVotes, s.base + "/my/activity/votes")
          )
        );
      }
      if (justVoted || s.voted) {
        items.push(
          item("topic-voting-menu__row", rowButton("remove-vote", "arrow-rotate-left", s.labels.remove_vote))
        );
        items.push(
          item(
            "topic-voting-menu__watch-toggle",
            rowButton("watch-toggle", s.watching ? "toggle-on" : "toggle-off", s.labels.watch_topic)
          )
        );
      }
    }
    var content = openMenu(trigger, "topic-voting-menu", '<ul class="dropdown-menu">' + items.join("") + "</ul>");
    content.addEventListener("click", function (event) {
      if (event.target.closest(".remove-vote")) {
        post(box, "/voting/unvote", { topic_id: s.topicId }).then(function (r) {
          if (r.ok) {
            applyResult(box, r.json, false);
          }
        });
        close();
      } else if (event.target.closest(".watch-toggle")) {
        var watching = state(box).watching;
        post(box, "/t/" + s.topicId + "/notifications", {
          notification_level: watching ? 1 : 3,
        }).then(function (r) {
          if (!r.ok) {
            return;
          }
          box.dataset.watching = String(!watching);
          var toggle = content.querySelector(".watch-toggle .d-icon");
          if (toggle) {
            toggle.outerHTML = icon(!watching ? "toggle-on" : "toggle-off");
          }
        });
      }
    });
  }

  // VoteCount's menu: the voters, fetched when it opens.
  function showVoters(box, trigger) {
    var s = state(box);
    var content = openMenu(
      trigger,
      "vote-count-voters",
      '<div class="voting-voters__loading">' + escapeHtml(s.labels.loading) + "</div>"
    );
    var inner = content.querySelector(".fk-d-menu__inner-content");
    if (s.count === 0) {
      inner.innerHTML = '<div class="voting-voters__empty">' + escapeHtml(s.labels.no_votes_yet) + "</div>";
      return;
    }
    fetch(s.base + "/voting/who.json?topic_id=" + encodeURIComponent(s.topicId), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        return r.json();
      })
      .then(function (users) {
        var html = (users || [])
          .map(function (u) {
            var src = s.base + u.avatar_template.replace("{size}", "24");
            return (
              '<a class="voting-voters__avatar trigger-user-card" data-user-card="' +
              escapeHtml(u.username) + '" title="' + escapeHtml(u.username) +
              '" href="' + s.base + "/u/" + encodeURIComponent(u.username.toLowerCase()) +
              '"><img loading="lazy" alt="" width="24" height="24" src="' + escapeHtml(src) +
              '" class="avatar"></a>'
            );
          })
          .join("");
        var more = s.count - (users || []).length;
        if (more > 0) {
          html +=
            '<div class="voting-voters__overflow">' +
            escapeHtml(s.labels.and_more_voters.replace("%{count}", String(more))) +
            "</div>";
        }
        inner.innerHTML = '<div class="voting-voters__list">' + html + "</div>";
      });
  }

  document.addEventListener("click", function (event) {
    var login = event.target.closest(".voting-wrapper__button[data-login-url]");
    if (login) {
      location.href = login.dataset.loginUrl;
      return;
    }
    var trigger = event.target.closest(".topic-voting-menu-trigger, .vote-count-voters-trigger");
    if (!trigger) {
      if (open && !event.target.closest(".fk-d-menu")) {
        close();
      }
      return;
    }
    var box = trigger.closest(".voting-wrapper");
    if (open && open.trigger === trigger) {
      close();
      return;
    }
    if (trigger.classList.contains("vote-count-voters-trigger")) {
      showVoters(box, trigger);
      return;
    }
    // VoteButton#onShowMenu: vote first when the viewer can.
    var s = state(box);
    if (s.limit !== 0 && !s.closed && !s.voted && !s.exceeded) {
      post(box, "/voting/vote", { topic_id: s.topicId }).then(function (r) {
        // 403 with the votes: out of votes after all.
        if (r.json && r.json.vote_count !== undefined) {
          applyResult(box, r.json, r.ok);
        }
        showVoteMenu(box, trigger, r.ok);
      });
      return;
    }
    showVoteMenu(box, trigger, false);
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      close();
    }
  });
})();
