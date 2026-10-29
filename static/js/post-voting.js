// discourse-post-voting on the topic page. PostVotingVoteControls: the up
// and down buttons vote and take the vote back (optimistically, as the
// component does), the count opens the voters (PostVotingWhoVotedList).
// PostVotingComments: a comment's upvote, edit (PostVotingCommentEditor)
// and delete; the comments menu's Add a comment (with the comments not
// drawn yet loaded first, as expandComposer does), its composer
// (PostVotingCommentComposer) and Show more. Comments loaded here are
// drawn from their JSON the way the server draws them, and kept, with the
// answers header, when the live stream redraws their post. An anonymous
// reader is sent to log in.
(function () {
  "use strict";

  if (!Discourse.once("post-voting")) {
    return;
  }

  var template = document.getElementById("post-voting-labels");
  if (!template) {
    return;
  }
  var labels = {};
  try {
    labels = JSON.parse(template.dataset.labels || "{}");
  } catch (e) {
    return;
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    headers["X-Requested-With"] = "XMLHttpRequest";
    headers.Accept = "application/json";
    return headers;
  }

  function basePath() {
    var timeline = document.querySelector(".timeline-container[data-topic-url]");
    var url = timeline ? timeline.dataset.topicUrl : "";
    var at = url.indexOf("/t/");
    return at > 0 ? url.slice(0, at) : "";
  }

  // ajax: the JSON, or the errors as popupAjaxError shows them.
  function request(method, path, params) {
    var query = new URLSearchParams(params).toString();
    var options = { method: method, credentials: "same-origin", headers: csrfHeaders() };
    var url = basePath() + path;
    if (method === "GET") {
      url += "?" + query;
    } else {
      options.body = query;
    }
    return fetch(url, options).then(function (r) {
      return r
        .json()
        .catch(function () {
          return {};
        })
        .then(function (json) {
          if (!r.ok) {
            var message = (json.errors || [r.statusText]).join("\n");
            if (Discourse.dialog) {
              Discourse.dialog.alert(message);
            } else {
              window.alert(message);
            }
            throw new Error(message);
          }
          return json;
        });
    });
  }

  function toLogin() {
    location.href = labels.login_url;
  }

  // For text and attribute values alike.
  function escapeHtml(s) {
    var div = document.createElement("div");
    div.textContent = s == null ? "" : String(s);
    return div.innerHTML.replace(/"/g, "&quot;").replace(/'/g, "&#39;");
  }

  function plural(key, count) {
    return (labels[key + (count === 1 ? "_one" : "_other")] || "").replace(
      "%{count}",
      String(count)
    );
  }

  // PostVotingButton, as view::vote_button draws it.
  function voteButton(direction, voted, disabled) {
    var key = direction === "up" ? "upvote" : "downvote";
    var label = voted ? labels["remove_" + key + "_label"] : labels[key + "_label"];
    return (
      '<button aria-label="' + escapeHtml(label) +
      '" class="btn no-text btn-icon btn-transparent post-voting-button --' + key +
      (voted ? " --voted" : "") + '" data-direction="' + direction +
      '" data-voted="' + voted + '"' + (disabled ? ' disabled=""' : "") +
      ' title="' + escapeHtml(label) + '" type="button">' +
      (voted ? labels.vote_icon : labels.unvoted_icon) +
      '<span aria-hidden="true">&#8203;</span></button>'
    );
  }

  function setVoted(button, voted) {
    var up = button.dataset.direction === "up";
    var key = up ? "upvote" : "downvote";
    var label = voted ? labels["remove_" + key + "_label"] : labels[key + "_label"];
    button.dataset.voted = String(voted);
    button.classList.toggle("--voted", voted);
    button.setAttribute("aria-label", label || "");
    button.setAttribute("title", label || "");
    button.innerHTML =
      (voted ? labels.vote_icon : labels.unvoted_icon) + '<span aria-hidden="true">&#8203;</span>';
  }

  // ---- votes on a post ----

  function voteState(controls) {
    var up = controls.querySelector(".--upvote");
    var down = controls.querySelector(".--downvote");
    var direction = up.dataset.voted === "true" ? "up" : down.dataset.voted === "true" ? "down" : null;
    return {
      direction: direction,
      count: Number(controls.querySelector(".post-voting-post__toggle-voters").textContent.trim()),
    };
  }

  function drawVotes(controls, direction, count) {
    setVoted(controls.querySelector(".--upvote"), direction === "up");
    setVoted(controls.querySelector(".--downvote"), direction === "down");
    controls.querySelector(".post-voting-post__toggle-voters").textContent = String(count);
  }

  function vote(controls, button) {
    if (!labels.viewer) {
      toLogin();
      return;
    }
    var before = voteState(controls);
    var direction = button.dataset.direction;
    var removing = button.dataset.voted === "true";
    var change;
    var after;
    if (removing) {
      change = direction === "up" ? -1 : 1;
      after = null;
    } else {
      change = before.direction ? 2 : 1;
      if (direction === "down") {
        change *= -1;
      }
      after = direction;
    }
    drawVotes(controls, after, before.count + change);
    var buttons = controls.querySelectorAll(".post-voting-button");
    buttons.forEach(function (b) {
      b.disabled = true;
    });
    var postId = controls.dataset.postId;
    var call = removing
      ? request("DELETE", "/post_voting/vote", { post_id: postId })
      : request("POST", "/post_voting/vote", { post_id: postId, direction: direction });
    call
      .catch(function () {
        drawVotes(controls, before.direction, before.count);
      })
      .finally(function () {
        buttons.forEach(function (b) {
          b.disabled = false;
        });
      });
  }

  // ---- the voters menu ----

  var open = null;

  function closeMenu() {
    if (!open) {
      return;
    }
    open.trigger.setAttribute("aria-expanded", "false");
    open.content.remove();
    open = null;
  }

  function openMenu(trigger, html) {
    closeMenu();
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu post-voting-popup-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.identifier = "post-voting-popup";
    content.innerHTML = '<div class="fk-d-menu__inner-content">' + html + "</div>";
    portals.appendChild(content);
    var rect = trigger.getBoundingClientRect();
    content.style.position = "absolute";
    content.style.left = rect.right + window.scrollX + 10 + "px";
    content.style.top = rect.top + window.scrollY + "px";
    trigger.setAttribute("aria-expanded", "true");
    open = { trigger: trigger, content: content };
    return content;
  }

  function voterList(direction, voters) {
    if (!voters.length) {
      return "";
    }
    var users = voters
      .map(function (v) {
        var src = (v.avatar_template || "").replace("{size}", "24");
        return (
          '<a class="trigger-user-card" data-user-card="' + escapeHtml(v.username) +
          '" href="' + basePath() + "/u/" + escapeHtml(String(v.username).toLowerCase()) +
          '" aria-label="' + escapeHtml(v.username) + '"><img alt="" width="24" height="24" src="' +
          escapeHtml(src) + '" class="avatar" title="' + escapeHtml(v.username) + '"></a>'
        );
      })
      .join("");
    return (
      '<div class="post-voting-popup-content__wrapper"><span aria-label="' +
      escapeHtml(direction === "up" ? labels.upvotes : labels.downvotes) +
      '" class="post-voting-popup-content__icon --' + (direction === "up" ? "upvote" : "downvote") +
      '">' + labels.vote_icon + '</span><span class="post-voting-popup-content__count">' +
      voters.length + '</span><div class="small-user-list post-voting-popup-content__voters">' +
      '<span class="small-user-list-content" role="list">' + users + "</span></div></div>"
    );
  }

  function showVoters(trigger) {
    if (open && open.trigger === trigger) {
      closeMenu();
      return;
    }
    var content = openMenu(trigger, escapeHtml(labels.loading));
    var postId = trigger.closest(".post-voting-post").dataset.postId;
    request("GET", "/post_voting/voters", { post_id: postId })
      .then(function (result) {
        if (!open || open.content !== content) {
          return;
        }
        var voters = result.voters || [];
        var html =
          voterList(
            "up",
            voters.filter(function (v) {
              return v.direction === "up";
            })
          ) +
          voterList(
            "down",
            voters.filter(function (v) {
              return v.direction === "down";
            })
          );
        var remaining = Math.max(0, (result.total_voters_count || 0) - voters.length);
        if (remaining) {
          html += "<span>" + escapeHtml(plural("voters_more", remaining)) + "</span>";
        }
        content.querySelector(".fk-d-menu__inner-content").innerHTML = html;
      })
      .catch(closeMenu);
  }

  // ---- comments ----

  function commentsOf(el) {
    return el.closest(".post-voting-comments");
  }

  function disabled(container) {
    return container.dataset.disabled === "true";
  }

  function tinyDate(iso) {
    var ms = Date.parse(iso);
    return (
      '<span class="relative-date" data-time="' + ms + '" data-format="tiny"></span>'
    );
  }

  // PostVotingCommentActions, as view::comment_actions draws them.
  function actionsHtml(comment, isDisabled) {
    if (!labels.viewer || isDisabled) {
      return "";
    }
    var mine = comment.username === labels.viewer;
    if (!mine && !labels.staff) {
      return "";
    }
    function action(cls, label, iconHtml) {
      return (
        '<button aria-label="' + escapeHtml(label) + '" class="btn no-text btn-icon ' + cls +
        ' btn-transparent" title="' + escapeHtml(label) + '" type="button">' + iconHtml +
        '<span aria-hidden="true">&#8203;</span></button>'
      );
    }
    return (
      '<span class="post-voting-comments__comment-actions">' +
      action("post-voting-comments__comment-actions-edit-link", labels.edit_label, labels.pencil_icon) +
      action("post-voting-comments__comment-actions-flag-link", labels.flag_label, labels.flag_icon) +
      action("post-voting-comments__comment-actions-delete-link", labels.delete_label, labels.trash_icon) +
      "</span>"
    );
  }

  // PostVotingComment from its serializer's JSON (view::comment).
  function commentHtml(comment, isDisabled) {
    var count = comment.post_voting_vote_count || 0;
    var countHtml = count
      ? '<span class="post-voting-comments__vote-count">' + count + "</span>"
      : '<span class="post-voting-comments__vote-count --none">0</span>';
    var author = comment.username
      ? '<a class="post-voting-comments__username" data-user-card="' + escapeHtml(comment.username) +
        '" href="' + basePath() + "/u/" + escapeHtml(String(comment.username).toLowerCase()) + '">' +
        escapeHtml(comment.username) + "</a>"
      : '<span class="post-voting-comments__username --deleted">' +
        escapeHtml(labels.deleted_user) + "</span>";
    return (
      '<div class="post-voting-comments__comment" id="post-voting-comment-' + comment.id +
      '" data-comment-id="' + comment.id + '" data-user-id="' + comment.user_id +
      '" data-raw="' + escapeHtml(comment.raw) + '" data-client=""><div class="post-voting-comments__vote">' +
      countHtml + voteButton("up", comment.user_voted === true, isDisabled) +
      '</div><div class="post-voting-comments__comment-post"><span class="post-voting-comments__comment-cooked">' +
      (comment.cooked || "") + '</span><span class="post-voting-comments__separator">–</span>' + author +
      '<span class="post-voting-comments__created">' + tinyDate(comment.created_at) + "</span>" +
      actionsHtml(comment, isDisabled) + "</div></div>"
    );
  }

  function lastCommentId(container) {
    var all = container.querySelectorAll(".post-voting-comments__comment");
    return all.length ? all[all.length - 1].dataset.commentId : "0";
  }

  // The comments menu's Show more, as many as are not drawn.
  function drawShowMore(container) {
    var actions = container.querySelector(".post-voting-comments__actions");
    if (!actions) {
      return;
    }
    var more = Number(container.dataset.commentsCount) - container.querySelectorAll(".post-voting-comments__comment").length;
    var button = actions.querySelector(".post-voting-comments__actions-show-more");
    if (more <= 0) {
      if (button) {
        button.remove();
      }
      return;
    }
    if (!button) {
      button = document.createElement("button");
      button.className =
        "btn btn-text btn-transparent --primary btn-small post-voting-comments__actions-show-more";
      button.type = "button";
      actions.appendChild(button);
    }
    button.innerHTML = '<span class="d-button-label">' + escapeHtml(plural("show_more", more)) + "</span>";
  }

  function appendComments(container, comments) {
    var footer = container.querySelector(".post-voting-comments__footer");
    comments.forEach(function (comment) {
      if (container.querySelector('[data-comment-id="' + comment.id + '"]')) {
        return;
      }
      var holder = document.createElement("div");
      holder.innerHTML = commentHtml(comment, disabled(container));
      var el = holder.firstChild;
      if (footer) {
        container.insertBefore(el, footer);
      } else {
        container.appendChild(el);
      }
    });
    container.dataset.lastCommentId = lastCommentId(container);
    drawShowMore(container);
  }

  // fetchComments: those after the last one drawn.
  function fetchComments(container) {
    return request("GET", "/post_voting/comments", {
      post_id: container.dataset.postId,
      last_comment_id: lastCommentId(container),
    }).then(function (result) {
      appendComments(container, result.comments || []);
    });
  }

  // PostVotingCommentComposer: the textarea and its length line.
  function composerHtml(container, raw) {
    return (
      '<div class="post-voting-comments__composer"><textarea class="post-voting-comments__composer-textarea" maxlength="' +
      container.dataset.maxLength + '">' + escapeHtml(raw || "") + "</textarea></div>"
    );
  }

  function wireComposer(container, root, submit) {
    var textarea = root.querySelector("textarea");
    var min = Number(container.dataset.minLength);
    var max = Number(container.dataset.maxLength);
    function update() {
      var value = textarea.value;
      var composer = textarea.parentNode;
      var flash = composer.querySelector(".post-voting-comments__composer-flash");
      if (!value.length) {
        if (flash) {
          flash.remove();
        }
      } else {
        if (!flash) {
          flash = document.createElement("div");
          composer.appendChild(flash);
        }
        if (value.length < min) {
          flash.className = "post-voting-comments__composer-flash has-error";
          flash.textContent = plural("too_short", min);
        } else if (value.length > max) {
          flash.className = "post-voting-comments__composer-flash has-error";
          flash.textContent = plural("too_long", max);
        } else {
          flash.className = "post-voting-comments__composer-flash";
          flash.innerHTML = value.length + "/<span>" + max + "</span>";
        }
      }
      submit.disabled = value.length < min || value.length > max;
    }
    textarea.addEventListener("input", update);
    textarea.addEventListener("keydown", function (event) {
      if (event.key === "Enter" && (event.ctrlKey || event.metaKey) && !submit.disabled) {
        submit.click();
      }
    });
    submit.disabled = true;
    return textarea;
  }

  // PostVotingCommentsMenu#expandComposer, then PostVotingCommentsMenuComposer.
  function openComposer(container) {
    var footer = container.querySelector(".post-voting-comments__footer");
    fetchComments(container).then(function () {
      footer.innerHTML =
        composerHtml(container, "") +
        '<button class="btn btn-icon-text btn-primary btn-small post-voting-comments__composer-submit" type="button">' +
        labels.reply_icon + '<span class="d-button-label">' + escapeHtml(labels.submit) + "</span></button>" +
        '<button class="btn btn-text btn-transparent btn-small post-voting-comments__composer-cancel" type="button">' +
        '<span class="d-button-label">' + escapeHtml(labels.cancel) + "</span></button>";
      var submit = footer.querySelector(".post-voting-comments__composer-submit");
      var textarea = wireComposer(container, footer, submit);
      textarea.focus();
      textarea.select();
    });
  }

  function closeComposer(container) {
    var footer = container.querySelector(".post-voting-comments__footer");
    var addLabel = footer.dataset.addLabel || "";
    footer.innerHTML =
      '<div class="post-voting-comments__actions"><button class="btn btn-text btn-transparent --primary btn-small post-voting-comments__actions-add" type="button"><span class="d-button-label">' +
      escapeHtml(addLabel) + "</span></button></div>";
    drawShowMore(container);
  }

  function saveComment(container, submit) {
    var textarea = container.querySelector(".post-voting-comments__composer-textarea");
    submit.disabled = true;
    request("POST", "/post_voting/comments", {
      raw: textarea.value,
      post_id: container.dataset.postId,
    })
      .then(function (comment) {
        container.dataset.commentsCount = String(Number(container.dataset.commentsCount) + 1);
        closeComposer(container);
        appendComments(container, [comment]);
      })
      .catch(function () {
        submit.disabled = false;
      });
  }

  // PostVotingCommentEditor in the comment's place.
  function editComment(container, el) {
    var raw = el.dataset.raw || "";
    el.dataset.saved = el.innerHTML;
    el.innerHTML =
      '<div class="post-voting-comments__comment-editor post-voting-comments__comment-editor-' +
      el.dataset.commentId + '">' + composerHtml(container, raw) +
      '<button class="btn btn-icon-text btn-primary post-voting-comments__comment-editor-submit" type="button">' +
      labels.pencil_icon + '<span class="d-button-label">' + escapeHtml(labels.edit) + "</span></button>" +
      '<button class="btn btn-text btn-link post-voting-comments__comment-editor-cancel" type="button">' +
      '<span class="d-button-label">' + escapeHtml(labels.cancel) + "</span></button></div>";
    var submit = el.querySelector(".post-voting-comments__comment-editor-submit");
    wireComposer(container, el, submit).focus();
  }

  function saveEdit(container, el, submit) {
    submit.disabled = true;
    request("PUT", "/post_voting/comments", {
      comment_id: el.dataset.commentId,
      raw: el.querySelector("textarea").value,
    })
      .then(function (comment) {
        var holder = document.createElement("div");
        holder.innerHTML = commentHtml(comment, disabled(container));
        el.replaceWith(holder.firstChild);
      })
      .catch(function () {
        submit.disabled = false;
      });
  }

  function cancelEdit(el) {
    el.innerHTML = el.dataset.saved || "";
    delete el.dataset.saved;
  }

  function deleteComment(container, el) {
    function destroy() {
      request("DELETE", "/post_voting/comments", { comment_id: el.dataset.commentId }).then(function () {
        el.classList.add("--deleted");
        container.dataset.commentsCount = String(Math.max(0, Number(container.dataset.commentsCount) - 1));
      });
    }
    if (Discourse.dialog && Discourse.dialog.deleteConfirm) {
      Discourse.dialog.deleteConfirm({ message: labels.delete_confirm, didConfirm: destroy });
    } else if (window.confirm(labels.delete_confirm)) {
      destroy();
    }
  }

  // A comment's upvote, optimistic.
  function voteComment(container, el, button) {
    if (!labels.viewer) {
      toLogin();
      return;
    }
    var countEl = el.querySelector(".post-voting-comments__vote-count");
    var count = Number(countEl.textContent);
    var removing = button.dataset.voted === "true";
    function draw(n, voted) {
      countEl.textContent = String(n);
      countEl.classList.toggle("--none", !n);
      setVoted(button, voted);
    }
    draw(count + (removing ? -1 : 1), !removing);
    button.disabled = true;
    request(removing ? "DELETE" : "POST", "/post_voting/vote/comment", {
      comment_id: el.dataset.commentId,
    })
      .catch(function () {
        draw(count, removing);
      })
      .finally(function () {
        button.disabled = false;
      });
  }

  document.addEventListener("click", function (event) {
    var target = event.target;
    var controls = target.closest(".post-voting-post");
    if (controls) {
      var voteBtn = target.closest(".post-voting-button");
      if (voteBtn) {
        vote(controls, voteBtn);
        return;
      }
      var toggle = target.closest("button.post-voting-post__toggle-voters");
      if (toggle) {
        showVoters(toggle);
        return;
      }
    }
    if (open && !target.closest(".post-voting-popup-content")) {
      closeMenu();
    }
    var answer = target.closest(".post-action-menu__post-voting-answer");
    if (answer && Discourse.composerOpen) {
      var topic = document.getElementById("topic");
      Discourse.composerOpen({
        mode: "reply",
        topicId: topic ? topic.dataset.topicId : null,
        replyTo: answer.dataset.postNumber,
        replyToUsername: answer.dataset.username,
      });
      return;
    }
    var container = commentsOf(target);
    if (!container) {
      return;
    }
    var comment = target.closest(".post-voting-comments__comment");
    if (target.closest(".post-voting-comments__actions-add")) {
      var footer = container.querySelector(".post-voting-comments__footer");
      footer.dataset.addLabel = target.closest("button").textContent.trim();
      if (!labels.viewer) {
        toLogin();
      } else {
        openComposer(container);
      }
    } else if (target.closest(".post-voting-comments__actions-show-more")) {
      fetchComments(container);
    } else if (target.closest(".post-voting-comments__composer-submit")) {
      saveComment(container, target.closest("button"));
    } else if (target.closest(".post-voting-comments__composer-cancel")) {
      closeComposer(container);
    } else if (comment && target.closest(".post-voting-comments__comment-editor-submit")) {
      saveEdit(container, comment, target.closest("button"));
    } else if (comment && target.closest(".post-voting-comments__comment-editor-cancel")) {
      cancelEdit(comment);
    } else if (comment && target.closest(".post-voting-comments__comment-actions-edit-link")) {
      editComment(container, comment);
    } else if (comment && target.closest(".post-voting-comments__comment-actions-delete-link")) {
      deleteComment(container, comment);
    } else if (comment && target.closest(".post-voting-comments__comment-actions-flag-link")) {
      if (Discourse.postVotingFlag) {
        Discourse.postVotingFlag(comment.dataset.commentId);
      }
    } else if (comment && target.closest(".post-voting-button")) {
      voteComment(container, comment, target.closest(".post-voting-button"));
    }
  });

  document.addEventListener("keydown", function (event) {
    if (event.key === "Escape") {
      closeMenu();
    }
  });

  // The live stream redraws a post from the server's view of it: the
  // comments loaded here and the answers header stay as the page has them.
  var kept = null;
  document.addEventListener("htmx:oobBeforeSwap", function (event) {
    var target = event.detail && event.detail.target;
    if (!target || !target.matches || !target.matches("#posts > [data-post-number]")) {
      return;
    }
    var container = target.querySelector(".post-voting-comments");
    var extras = container ? Array.prototype.slice.call(container.querySelectorAll("[data-client]")) : [];
    kept = {
      number: target.dataset.postNumber,
      header: target.querySelector(".post-voting-answers-header"),
      extras: extras,
    };
  });
  document.addEventListener("htmx:oobAfterSwap", function () {
    var k = kept;
    kept = null;
    if (!k) {
      return;
    }
    var wrapper = document.querySelector('#posts > [data-post-number="' + k.number + '"]');
    if (!wrapper) {
      return;
    }
    if (k.header && !wrapper.querySelector(".post-voting-answers-header")) {
      var article = wrapper.querySelector("article");
      if (article) {
        wrapper.insertBefore(k.header, article);
      }
    }
    var container = wrapper.querySelector(".post-voting-comments");
    if (!container || !k.extras.length) {
      return;
    }
    var footer = container.querySelector(".post-voting-comments__footer");
    k.extras.forEach(function (el) {
      if (!container.querySelector('[data-comment-id="' + el.dataset.commentId + '"]')) {
        container.insertBefore(el, footer);
      }
    });
    container.dataset.lastCommentId = lastCommentId(container);
    drawShowMore(container);
  });
})();
