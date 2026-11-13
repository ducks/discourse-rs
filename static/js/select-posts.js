// The topic's multi-select mode (controllers/topic.js: multiSelect,
// selectedPostIds and the can* rules over them), turned on and off from
// the topic admin menu's Select Posts (topic-admin-menu.js):
//
//   - each post's select controls (post/meta-data/select-post.gjs) in its
//     .post-infos: select / selected, and on an unselected reply select
//     +below and, when it has replies, select +replies. A selected post's
//     .topic-post is post--selected selected. Posts drawn while the mode
//     is on (loaded on scroll, redrawn live) get theirs as they arrive.
//   - SelectedPosts (components/selected-posts.gjs), .selected-posts at
//     the top of .container.posts: the count, select all / deselect all
//     and the buttons the rules allow.
//   - ChangeOwner (components/modal/change-owner.gjs) with its
//     EmailGroupUserChooser (email-group-user-chooser.js), POSTing
//     /t/:id/change-owner as Topic.changeOwners does; the page is then
//     drawn again, where Ember routes to the topic. The post admin menu's
//     Change Ownership opens it for its one post.
//
// Delete selected asks as Ember does and, with every post selected,
// deletes the topic (the admin menu's delete); otherwise it is
// Post.deleteMany (DELETE /posts/destroy_many). Merge selected posts is
// Post.mergePosts (PUT /posts/merge_posts), select +replies the post's
// GET /posts/:id/reply-ids. Both writes turn the mode off as Ember does
// and then draw the page again, where Ember marks the posts in place or
// waits for the message bus; what the server refuses is shown as
// popupAjaxError shows it. Move to opens MoveToTopic
// (components/modal/move-to-topic.gjs) with its ChooseTopic and
// ChooseMessage, POSTing /t/:id/move-posts or, with every post selected,
// /t/:id/merge-topic, and then goes to the destination topic.
//
// The posts' state is the server's: the stream's ids on #posts, and on a
// post's article data-username, data-can-delete and data-has-replies.
// The viewer's and the strings are #select-posts-config's.
(function () {
  "use strict";

  if (!Discourse.once("select-posts")) {
    return;
  }

  function base() {
    return (document.querySelector('meta[name="discourse-base-uri"]') || {}).content || "";
  }

  function memberHeaders() {
    try {
      return JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      return {};
    }
  }

  function config() {
    var el = document.getElementById("select-posts-config");
    try {
      return el ? JSON.parse(el.textContent) : null;
    } catch (e) {
      return null;
    }
  }

  function escapeHtml(s) {
    return String(s == null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function icon(name) {
    return (
      '<svg class="fa d-icon d-icon-' +
      name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name +
      '"></use></svg>'
    );
  }

  // ajax with jQuery's form encoding: `ids` as post_ids[].
  function request(method, path, ids, extra) {
    var headers = memberHeaders();
    headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
    var form = new URLSearchParams();
    ids.forEach(function (id) {
      form.append("post_ids[]", String(id));
    });
    Object.keys(extra || {}).forEach(function (key) {
      form.append(key, extra[key]);
    });
    return fetch(base() + path, {
      method: method,
      headers: headers,
      credentials: "same-origin",
      body: form.toString(),
    }).then(function (r) {
      return r.ok ? r : Promise.reject(r);
    });
  }

  // popupAjaxError: the errors the server gave, in a dialog.
  function popupError(r) {
    if (!(r instanceof Response)) {
      console.error(r);
      return;
    }
    r.json()
      .catch(function () {
        return {};
      })
      .then(function (body) {
        var text = ((body && body.errors) || [r.statusText || "HTTP " + r.status]).join("\n");
        Discourse.dialog.popupAjaxError(text);
      });
  }

  function plural(cfg, key, count) {
    return String(cfg.strings[key + (count === 1 ? ".one" : ".other")]).replace("%{count}", count);
  }

  // The mode while it is on: its panel, the selected post ids in the
  // order chosen, the observer drawing the controls of posts as they
  // arrive, and the admin menu's topic and delete.
  var mode = null;

  // Leaving the topic turns the mode off (the topic route's
  // deactivate): its panel went with the page.
  function current() {
    if (mode && !mode.panel.isConnected) {
      mode.observer.disconnect();
      mode = null;
    }
    return mode;
  }

  function streamEl() {
    return document.getElementById("posts");
  }

  // postStream.stream: every post id of the topic, in order. The page's
  // is as it was loaded; posts that arrived since are drawn after it.
  function stream() {
    var el = streamEl();
    var ids = [];
    try {
      ids = ((el && JSON.parse(el.dataset.stream)) || []).slice();
    } catch (e) {}
    if (el) {
      el.querySelectorAll(":scope > [data-post-number] > article[data-post-id]").forEach(function (article) {
        var id = parseInt(article.dataset.postId, 10);
        if (ids.indexOf(id) === -1) {
          ids.push(id);
        }
      });
    }
    return ids;
  }

  // postStream.posts: the posts drawn, by id.
  function loadedPosts() {
    var posts = {};
    var el = streamEl();
    if (!el) {
      return posts;
    }
    el.querySelectorAll(":scope > [data-post-number] > article[data-post-id]").forEach(function (article) {
      var id = parseInt(article.dataset.postId, 10);
      posts[id] = {
        id: id,
        article: article,
        wrapper: article.parentNode,
        smallAction: article.classList.contains("small-action"),
        firstPost: article.parentNode.dataset.postNumber === "1",
        number: parseInt(article.parentNode.dataset.postNumber, 10),
        // post_type regular: neither a whisper nor a moderator action.
        regular:
          !article.classList.contains("small-action") &&
          !article.parentNode.classList.contains("whisper") &&
          !article.parentNode.classList.contains("moderator"),
        username: article.dataset.username,
        canDelete: article.hasAttribute("data-can-delete"),
        hasReplies: article.hasAttribute("data-has-replies"),
      };
    });
    return posts;
  }

  // _smallActionPostIds, of the posts drawn.
  function withoutSmallActions(ids, posts) {
    return ids.filter(function (id) {
      return !(posts[id] && posts[id].smallAction);
    });
  }

  function union(a, b) {
    var out = a.slice();
    b.forEach(function (id) {
      if (out.indexOf(id) === -1) {
        out.push(id);
      }
    });
    return out;
  }

  // The controller's rules over the selection.
  function rules(cfg) {
    var ids = mode.ids;
    var posts = loadedPosts();
    var count = ids.length;
    // selectedPosts: those of the selection that are drawn.
    var selected = ids
      .map(function (id) {
        return posts[id];
      })
      .filter(Boolean);
    // selectedPostsUsername: the one author of the selection, all drawn.
    var username;
    if (selected.length >= 1 && count <= selected.length) {
      username = selected.every(function (p) {
        return p.username === selected[0].username;
      })
        ? selected[0].username
        : undefined;
    }
    var all = count >= stream().length;
    var everyDeletable = selected.every(function (p) {
      return p.canDelete;
    });
    return {
      count: count,
      username: username,
      selectedAll: all,
      canSelectAll: !all,
      canDeselectAll: all,
      canDeleteSelected: count > 0 && ((all && cfg.staff) || everyDeletable),
      canMergeTopic: cfg.can_move_posts && count > 0,
      // MoveToTopic's canSplitTopic: not every post, and the first of
      // those drawn a regular post.
      canSplitTopic:
        !all &&
        selected.length > 0 &&
        selected.slice().sort(function (a, b) {
          return a.number - b.number;
        })[0].regular,
      canChangeOwner: cfg.can_change_post_owner && count > 0 && username !== undefined,
      canMergePosts: count > 1 && username !== undefined && everyDeletable,
    };
  }

  function button(className, label, title, iconName) {
    return (
      '<button class="btn ' +
      (iconName ? "btn-icon-text " : "btn-text ") +
      className +
      '"' +
      (title ? ' title="' + escapeHtml(title) + '"' : "") +
      ' type="button">' +
      (iconName ? icon(iconName) : "") +
      '<span class="d-button-label">' +
      escapeHtml(label) +
      "</span></button>"
    );
  }

  // PostMetaDataSelectPost
  function controls(post, selected, s) {
    var m = "topic.multi_select.";
    var html = "";
    if (!selected && !post.firstPost) {
      if (post.hasReplies) {
        html += button("btn-flat select-replies", s[m + "select_replies.label"], s[m + "select_replies.title"]);
      }
      html += button("btn-flat select-below", s[m + "select_below.label"], s[m + "select_below.title"]);
    }
    var key = m + (selected ? "selected_post" : "select_post");
    return html + button("btn-flat select-post", s[key + ".label"], s[key + ".title"]);
  }

  // The posts' controls and selected classes as the selection has them;
  // with the mode off, gone. Only what differs is written, so the
  // observer settles.
  function drawPosts(cfg) {
    var el = streamEl();
    if (!el) {
      return;
    }
    var on = !!mode;
    var posts = loadedPosts();
    Object.keys(posts).forEach(function (id) {
      var post = posts[id];
      if (post.smallAction) {
        return;
      }
      var selected = on && mode.ids.indexOf(post.id) !== -1;
      ["post--selected", "selected"].forEach(function (name) {
        if (post.wrapper.classList.contains(name) !== selected) {
          post.wrapper.classList.toggle(name, selected);
        }
      });
      var infos = post.article.querySelector(".topic-meta-data .post-infos");
      if (!infos) {
        return;
      }
      var box = infos.querySelector(":scope > .select-posts");
      if (!on) {
        if (box) {
          box.remove();
        }
        return;
      }
      var html = controls(post, selected, cfg.strings);
      if (!box) {
        box = document.createElement("div");
        box.className = "select-posts";
        // Before the reply-to tab, the date and the read state, after
        // the indicators (meta-data.gjs' order).
        infos.insertBefore(box, infos.querySelector(":scope > .reply-to-tab, :scope > .post-date, :scope > .read-state"));
      }
      if (box.dataset.html !== html) {
        box.dataset.html = html;
        box.innerHTML = html;
      }
    });
  }

  function link(className, label, action) {
    var p = document.createElement("p");
    var a = document.createElement("a");
    if (className) {
      a.className = className;
    }
    a.setAttribute("href", "");
    a.textContent = label;
    a.addEventListener("click", function (event) {
      event.preventDefault();
      action();
    });
    p.appendChild(a);
    return p;
  }

  // SelectedPosts
  function drawPanel(cfg) {
    var s = cfg.strings;
    var r = rules(cfg);
    var root = mode.panel.firstElementChild;
    root.textContent = "";
    var count = document.createElement("p");
    count.innerHTML =
      "<span>" +
      String(s["topic.multi_select.description." + (r.count === 1 ? "one" : "other")]).replace("%{count}", r.count) +
      "</span>";
    root.appendChild(count);
    if (r.canSelectAll) {
      root.appendChild(link("select-all", s["topic.multi_select.select_all"], selectAll));
    }
    if (r.canDeselectAll) {
      root.appendChild(link("", s["topic.multi_select.deselect_all"], deselectAll));
    }
    var add = function (show, className, iconName, label, action) {
      if (!show) {
        return;
      }
      var holder = document.createElement("div");
      holder.innerHTML = button(className, label, null, iconName);
      holder.firstChild.addEventListener("click", action);
      root.appendChild(holder.firstChild);
    };
    add(r.canDeleteSelected, "btn-danger", "trash-can", s["topic.multi_select.delete"], function () {
      deleteSelected(cfg);
    });
    add(r.canMergeTopic, "btn-primary move-to-topic", "right-from-bracket", s["topic.move_to.action"], function () {
      var now = rules(cfg);
      moveToTopic(cfg, { ids: mode.ids.slice(), count: now.count, selectedAll: now.selectedAll, canSplitTopic: now.canSplitTopic });
    });
    add(r.canChangeOwner, "btn-primary", "user", s["topic.change_owner.action"], function () {
      var now = rules(cfg);
      changeOwner(cfg, { ids: mode.ids.slice(), count: now.count, username: now.username });
    });
    add(r.canMergePosts, "btn-primary", "up-down", s["topic.merge_posts.action"], function () {
      mergePosts(cfg);
    });
    var cancel = link("", s["topic.multi_select.cancel"], toggle);
    cancel.className = "cancel";
    root.appendChild(cancel);
  }

  function draw() {
    var cfg = config();
    if (!cfg) {
      return;
    }
    drawPosts(cfg);
    if (mode) {
      drawPanel(cfg);
    }
  }

  function selectAll() {
    mode.ids = withoutSmallActions(stream(), loadedPosts());
    draw();
  }

  function deselectAll() {
    mode.ids = [];
    draw();
  }

  // toggleMultiSelect; either way the selection starts empty
  // (_multiSelectChanged).
  function toggle(topic, helpers) {
    if (current()) {
      mode.observer.disconnect();
      mode.panel.remove();
      mode = null;
      draw();
      return;
    }
    var container = document.querySelector(".container.posts");
    if (!container || !streamEl() || !config()) {
      return;
    }
    var panel = document.createElement("div");
    panel.className = "selected-posts";
    panel.appendChild(document.createElement("div"));
    container.insertBefore(panel, container.firstChild);
    mode = {
      panel: panel,
      ids: [],
      topic: topic,
      helpers: helpers || {},
      // Posts loaded or drawn again: their controls, and the rules over
      // what is now drawn.
      observer: new MutationObserver(function (records) {
        var posts = records.some(function (record) {
          return !record.target.closest(".select-posts");
        });
        if (posts && current()) {
          draw();
        }
      }),
    };
    mode.observer.observe(streamEl(), { childList: true, subtree: true });
    draw();
  }

  // deleteSelected: every post selected is the topic deleted, otherwise
  // Post.deleteMany. Ember marks the posts deleted without waiting and
  // leaves a failure unhandled; here the page is drawn again, or the
  // server's errors are shown.
  function deleteSelected(cfg) {
    Discourse.dialog.yesNoConfirm({
      message: plural(cfg, "post.delete.confirm", rules(cfg).count),
      didConfirm: function () {
        if (!current()) {
          return;
        }
        if (rules(cfg).selectedAll && mode.helpers.deleteTopic) {
          mode.helpers.deleteTopic(mode.topic);
          return;
        }
        var ids = mode.ids.slice();
        toggle();
        request("DELETE", "/posts/destroy_many", ids, { agree_with_first_reply_flag: "true" })
          .then(function () {
            location.reload();
          })
          .catch(popupError);
      },
    });
  }

  // mergePosts: Post.mergePosts, its failure popupAjaxError's.
  function mergePosts(cfg) {
    Discourse.dialog.yesNoConfirm({
      message: plural(cfg, "post.merge.confirm", rules(cfg).count),
      didConfirm: function () {
        if (!current()) {
          return;
        }
        var ids = mode.ids.slice();
        toggle();
        request("PUT", "/posts/merge_posts", ids)
          .then(function () {
            location.reload();
          })
          .catch(popupError);
      },
    });
  }

  // selectReplies: the post and its replies (GET /posts/:id/reply-ids)
  // join the selection, drawn or not. Ember leaves a failure unhandled.
  function selectReplies(id) {
    fetch(base() + "/posts/" + id + "/reply-ids.json", { headers: memberHeaders(), credentials: "same-origin" })
      .then(function (r) {
        return r.ok ? r.json() : Promise.reject(r);
      })
      .then(function (replies) {
        if (!current()) {
          return;
        }
        mode.ids = union(
          mode.ids,
          [id].concat(
            replies.map(function (reply) {
              return reply.id;
            })
          )
        );
        draw();
      })
      .catch(popupError);
  }

  // ChangeOwnerModal for `model`: { ids, count, username }, the posts'
  // one author when they have one.
  function changeOwner(cfg, model) {
    if (!Discourse.modal || !Discourse.emailGroupUserChooser) {
      return;
    }
    var s = cfg.strings;
    var newOwner = null;
    var saving = false;
    var submit;
    var close;

    function sync() {
      submit.disabled = saving || !newOwner;
      submit.firstChild.textContent = saving ? s.saving : s["topic.change_owner.action"];
    }

    // Topic.changeOwners
    function save() {
      saving = true;
      sync();
      var headers = memberHeaders();
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      var form = new URLSearchParams();
      model.ids.forEach(function (id) {
        form.append("post_ids[]", String(id));
      });
      form.append("username", newOwner);
      fetch(base() + "/t/" + cfg.topic_id + "/change-owner", {
        method: "POST",
        headers: headers,
        credentials: "same-origin",
        body: form.toString(),
      })
        .then(function (r) {
          return r.ok ? r.json() : Promise.reject(r);
        })
        .then(function (result) {
          if (!result || !result.success) {
            return Promise.reject(result);
          }
          close();
          location.reload();
        })
        .catch(function () {
          modalFlash(submit, s["topic.change_owner.error"]);
          saving = false;
          sync();
        });
    }

    close = Discourse.modal.show({
      className: "change-ownership-modal",
      title: s["topic.change_owner.title"],
      body: function (body) {
        body.classList.add("change-ownership");
        var key =
          "topic.change_owner." +
          (model.username ? "instructions" : "instructions_without_old_user") +
          (model.count === 1 ? ".one" : ".other");
        var instructions = document.createElement("span");
        instructions.innerHTML = String(s[key])
          .replace("%{count}", model.count)
          .replace("%{old_user}", escapeHtml(model.username));
        body.appendChild(instructions);
        var holder = document.createElement("div");
        holder.innerHTML =
          '<details class="select-kit multi-select user-chooser email-group-user-chooser"><summary data-name="" data-value="" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div></details>';
        var details = holder.firstChild;
        body.appendChild(details);
        Discourse.emailGroupUserChooser.attach(details, {
          none: s["select_kit.default_header_text"],
          placeholder: s["topic.change_owner.placeholder"],
          noContent: s["select_kit.no_content"],
          resultsOne: s["select_kit.results_count.one"],
          resultsOther: s["select_kit.results_count.other"],
          deleteItem: s["select_kit.delete_item"],
          allowEmails: false,
          includeMessageableGroups: false,
          maximum: 1,
          onChange: function (values) {
            newOwner = values[0] || null;
            sync();
          },
        });
      },
      footer: function (footer) {
        var holder = document.createElement("div");
        holder.innerHTML = button("btn-primary", s["topic.change_owner.action"]);
        submit = holder.firstChild;
        footer.appendChild(submit);
        submit.addEventListener("click", save);
        sync();
      },
    });
  }

  // DModal's @flash (error): `text` above the modal's body, or none.
  function modalFlash(inside, text) {
    var modal = inside.closest(".d-modal");
    var alert = modal.querySelector("#modal-alert");
    if (!text) {
      if (alert) {
        alert.remove();
      }
      return;
    }
    if (!alert) {
      alert = document.createElement("div");
      alert.id = "modal-alert";
      alert.setAttribute("role", "alert");
      alert.className = "alert alert-error";
      var body = modal.querySelector(".d-modal__body");
      body.parentNode.insertBefore(alert, body);
    }
    alert.textContent = text;
  }

  // extractError over a failed response's body, else `fallback`.
  function extractError(s, r, body, fallback) {
    var errors = body && Array.isArray(body.errors) ? body.errors : [];
    if (errors.length > 1) {
      return String(s.multiple_errors).replace(
        "%{errors}",
        errors
          .map(function (e, i) {
            return i + 1 + ") " + e;
          })
          .join(" ")
      );
    }
    if (errors.length === 1) {
      return String(s.generic_error_with_reason).replace("%{error}", errors[0]);
    }
    if (body && (body.error || body.message)) {
      return body.error || body.message;
    }
    if (r && r.status >= 400) {
      return r.status + " " + r.statusText;
    }
    return fallback;
  }

  // ChooseTopic and ChooseMessage (components/choose-topic.gjs,
  // choose-message.gjs): the topics, or the messages, found by title, url
  // or id through /search/query as searchForTerm asks, the current topic
  // left out; one result alone is chosen. `chosen(topic)` hears the
  // choice, null when a search starts. TopicStatus and the title's emoji
  // are not drawn.
  function chooseTopic(cfg, kind, chosen) {
    var s = cfg.strings;
    var message = kind === "message";
    var name = message ? "choose-message" : "choose-topic";
    var strings = message ? "choose_message." : "choose_topic.";
    var root = document.createElement("div");
    root.className = name;
    var label = escapeHtml(s[strings + "title.search"]);
    root.innerHTML =
      '<label for="' +
      name +
      '-title">' +
      (message ? label : "<span>" + label + "</span>") +
      '</label><input id="' +
      name +
      '-title" placeholder="' +
      escapeHtml(s[strings + "title.placeholder"]) +
      '" type="text"><div class="' +
      name +
      '__search-results">&nbsp;</div>';
    var input = root.querySelector("input");
    var results = root.querySelector("." + name + "__search-results");
    var timer = null;
    var searches = 0;
    var found = [];

    // dBoundCategoryLink, as the composer's category chooser draws the
    // category.
    function category(id) {
      var row = id && document.querySelector('#reply-control details.category-chooser .category-row[data-value="' + Number(id) + '"]');
      var badge = row && row.querySelector(".category-status > .badge-category__wrapper");
      if (!badge) {
        return "";
      }
      badge = badge.cloneNode(true);
      badge.querySelectorAll(".topic-count").forEach(function (count) {
        count.remove();
      });
      return badge.outerHTML;
    }

    function draw() {
      if (!found.length) {
        results.textContent = s[strings + "none_found"];
        return;
      }
      var rows = found
        .map(function (t) {
          return message
            ? '<div class="controls existing-message"><label class="radio"><input name="choose_message_id" type="radio" value="' +
                t.id +
                '"><span class="message-title">' +
                escapeHtml(t.title) +
                "</span></label></div>"
            : '<div class="controls existing-topic"><label class="radio"><input id="choose-topic-' +
                t.id +
                '" name="choose_topic_id" type="radio" value="' +
                t.id +
                '"><span class="topic-title">' +
                escapeHtml(t.title) +
                '</span><span class="topic-categories">' +
                category(t.category_id) +
                "</span></label></div>";
        })
        .join("");
      results.innerHTML = message ? rows : '<div class="choose-topic-list" role="radiogroup">' + rows + "</div>";
      if (found.length === 1) {
        results.querySelector("input").checked = true;
        chosen(found[0]);
      }
    }

    function search(term) {
      var params = new URLSearchParams({
        term: term,
        type_filter: message ? "private_messages" : "topic",
        search_for_id: "true",
        restrict_to_archetype: message ? "private_message" : "regular",
      });
      var mine = searches;
      var headers = memberHeaders();
      headers.Accept = "application/json";
      fetch(base() + "/search/query?" + params.toString(), { credentials: "same-origin", headers: headers })
        .then(function (r) {
          return r.ok ? r.json() : {};
        })
        .catch(function () {
          return {};
        })
        .then(function (body) {
          // The term changed since: these results are ignored.
          if (mine !== searches || !root.isConnected) {
            return;
          }
          // results.posts.map((item) => item.topic)
          var topics = (body && body.topics) || [];
          found = [];
          ((body && body.posts) || []).forEach(function (post) {
            var topic = topics.find(function (candidate) {
              return candidate.id === post.topic_id;
            });
            if (topic && topic.id !== cfg.topic_id && found.indexOf(topic) === -1) {
              found.push(topic);
            }
          });
          draw();
        });
    }

    input.addEventListener("keydown", function (event) {
      // ignoreEnter
      if (event.key === "Enter") {
        event.preventDefault();
        event.stopPropagation();
      }
    });
    input.addEventListener("input", function () {
      clearTimeout(timer);
      searches++;
      chosen(null);
      var term = input.value;
      if (!term) {
        results.innerHTML = "&nbsp;";
        return;
      }
      results.textContent = s.loading;
      // DAsyncContent's @debounce: INPUT_DELAY.
      timer = setTimeout(function () {
        search(term);
      }, 250);
    });
    results.addEventListener("change", function (event) {
      var id = Number(event.target.value);
      chosen(
        found.find(function (t) {
          return t.id === id;
        }) || null
      );
    });
    return root;
  }

  // MoveToTopic (components/modal/move-to-topic.gjs) for `model`:
  // { ids, count, selectedAll, canSplitTopic }. A topic offers a new
  // topic, an existing topic and (an admin's) a new message; a message a
  // new message (an admin's) and an existing message; with every post
  // selected, or a first post that is not a regular one, only the
  // existing one. Topic.movePosts, or with every post selected
  // mergeTopic, then the destination topic.
  //
  // Not as Ember: the button waits for what its destination needs (a
  // title, or a chosen topic), where Ember's is enabled by either under
  // any destination and then fails; the category chooser is the
  // composer's (topic-admin-modals.js) and the tags the composer's
  // MiniTagChooser, offering existing tags only (tag_ids carries no new
  // tag), where Ember has TagChooser.
  function moveToTopic(cfg, model) {
    if (!Discourse.modal) {
      return;
    }
    var s = cfg.strings;
    var canSplitTopic = model.canSplitTopic;
    var canSplitToPM = canSplitTopic && cfg.admin;
    var state = {
      selection: "new_topic",
      topicName: "",
      categoryId: null,
      tags: [],
      participants: [],
      chronologicalOrder: false,
      selectedTopic: null,
      saving: false,
    };
    if (cfg.private_message) {
      state.selection = canSplitToPM ? "new_message" : "existing_message";
    } else if (!canSplitTopic) {
      state.selection = "existing_topic";
    }
    var TITLES = {
      new_topic: "topic.split_topic.title",
      existing_topic: "topic.merge_topic.title",
      new_message: "topic.move_to_new_message.title",
      existing_message: "topic.move_to_existing_message.title",
    };
    var bodyEl;
    var submit;
    var close;
    var order;

    function existing() {
      return state.selection === "existing_topic" || state.selection === "existing_message";
    }

    // buttonDisabled and buttonTitle
    function sync() {
      submit.disabled = state.saving || (existing() ? !state.selectedTopic : !state.topicName);
      submit.querySelector(".d-button-label").textContent = s[TITLES[state.selection]];
      // The chronological order checkbox, once a topic is chosen.
      if (order) {
        order.parts.forEach(function (part) {
          if (state.selectedTopic) {
            order.form.appendChild(part);
          } else {
            part.remove();
          }
        });
      }
    }

    function el(html) {
      var holder = document.createElement("div");
      holder.innerHTML = html;
      return holder.firstChild;
    }

    function instructions(key) {
      var p = document.createElement("p");
      p.innerHTML = plural(cfg, key, model.count);
      return p;
    }

    function titleField(form) {
      var input = el('<input class="ember-text-field" id="split-topic-name" type="text">');
      input.placeholder = s["composer.title_placeholder"];
      input.value = state.topicName;
      input.addEventListener("input", function () {
        state.topicName = input.value;
        sync();
      });
      form.appendChild(input);
    }

    // The composer's tag chooser, copied; nothing where the viewer has
    // none.
    function tagField(form, withCategory) {
      var source = document.querySelector("#reply-control details.mini-tag-chooser");
      if (!source || !Discourse.miniTagChooser) {
        return;
      }
      form.appendChild(el("<label>" + escapeHtml(s["tagging.tags"]) + "</label>"));
      var details = source.cloneNode(true);
      details.open = false;
      details.classList.remove("is-expanded");
      details.removeAttribute("category-id");
      var options = {};
      try {
        options = JSON.parse(details.dataset.options || "{}");
      } catch (e) {}
      options.allowAny = false;
      details.dataset.options = JSON.stringify(options);
      var body = details.querySelector(".select-kit-body");
      body.innerHTML = "";
      body.removeAttribute("style");
      var chooser = Discourse.miniTagChooser(details, {
        categoryId: function () {
          return withCategory && state.categoryId ? String(state.categoryId) : "";
        },
        changed: function () {
          state.tags = chooser.selected().slice();
        },
        opened: function () {
          if (Discourse.topicAdminModals) {
            Discourse.topicAdminModals.floatBody(details);
          }
        },
      });
      chooser.set(state.tags);
      form.appendChild(details);
    }

    function chooseField(form, kind) {
      form.appendChild(
        chooseTopic(cfg, kind, function (topic) {
          state.selectedTopic = topic;
          sync();
        })
      );
    }

    function orderField(form) {
      var label = el(
        '<label class="checkbox-label" for="chronological-order"><input id="chronological-order" class="ember-checkbox" type="checkbox">' +
          escapeHtml(s["topic.merge_topic.chronological_order"]) +
          "</label>"
      );
      var box = label.querySelector("input");
      box.checked = state.chronologicalOrder;
      box.addEventListener("change", function () {
        state.chronologicalOrder = box.checked;
      });
      order = { form: form, parts: [document.createElement("hr"), label] };
    }

    function newMessageForm() {
      var form = document.createElement("form");
      form.appendChild(el("<label>" + escapeHtml(s["topic.move_to_new_message.message_title"]) + "</label>"));
      titleField(form);
      if (cfg.can_tag_pms) {
        tagField(form, false);
      }
      return form;
    }

    // The destination's instructions and form, as the template's ifs.
    function draw() {
      while (bodyEl.lastChild && !bodyEl.lastChild.classList.contains("radios")) {
        bodyEl.lastChild.remove();
      }
      // A ChooseTopic drawn again starts with nothing chosen.
      state.selectedTopic = null;
      order = null;
      var form;
      if (state.selection === "existing_topic") {
        bodyEl.appendChild(instructions("topic.merge_topic.instructions"));
        form = document.createElement("form");
        chooseField(form, "topic");
        orderField(form);
      } else if (state.selection === "new_topic") {
        bodyEl.appendChild(instructions("topic.split_topic.instructions"));
        form = document.createElement("form");
        form.className = "split-new-topic-form";
        var title = el('<div class="control-group"><label></label></div>');
        title.firstChild.textContent = s["topic.split_topic.topic_name"];
        titleField(title);
        form.appendChild(title);
        var chooser =
          Discourse.topicAdminModals &&
          Discourse.topicAdminModals.categoryChooser(
            mode ? mode.topic : {},
            function (id) {
              state.categoryId = id;
            },
            { value: state.categoryId }
          );
        if (chooser) {
          var category = el('<div class="control-group"><label></label></div>');
          category.firstChild.textContent = s["categories.category"];
          chooser.classList.add("small");
          category.appendChild(chooser);
          form.appendChild(category);
        }
        if (cfg.can_create_tag) {
          var tags = document.createElement("div");
          tags.className = "control-group";
          tagField(tags, true);
          if (tags.firstChild) {
            form.appendChild(tags);
          }
        }
      } else if (state.selection === "new_message") {
        bodyEl.appendChild(instructions("topic.move_to_new_message.instructions"));
        form = newMessageForm();
      } else {
        bodyEl.appendChild(instructions("topic.move_to_existing_message.instructions"));
        form = document.createElement("form");
        chooseField(form, "message");
        form.appendChild(el("<label>" + escapeHtml(s["topic.move_to_new_message.participants"]) + "</label>"));
        if (Discourse.emailGroupUserChooser) {
          var details = el(
            '<details class="select-kit multi-select user-chooser email-group-user-chooser participant-selector"><summary data-name="" data-value="" tabindex="0" class="select-kit-header multi-select-header"><div class="select-kit-header-wrapper"></div></summary><div class="select-kit-body"></div></details>'
          );
          form.appendChild(details);
          Discourse.emailGroupUserChooser
            .attach(details, {
              none: s["select_kit.default_header_text"],
              placeholder: "",
              noContent: s["select_kit.no_content"],
              resultsOne: s["select_kit.results_count.one"],
              resultsOther: s["select_kit.results_count.other"],
              deleteItem: s["select_kit.delete_item"],
              allowEmails: false,
              includeMessageableGroups: false,
              onChange: function (values) {
                state.participants = values.slice();
              },
            })
            .set(state.participants);
        }
        orderField(form);
      }
      form.addEventListener("submit", function (event) {
        event.preventDefault();
      });
      bodyEl.appendChild(form);
      sync();
    }

    // movePostsTo: the params as jQuery encodes them.
    function params() {
      var form = new URLSearchParams();
      function ids() {
        model.ids.forEach(function (id) {
          form.append("post_ids[]", String(id));
        });
      }
      function tagIds() {
        var tagged = state.tags.filter(function (t) {
          return t.id;
        });
        if (!tagged.length) {
          return;
        }
        tagged.forEach(function (t) {
          form.append("tag_ids[]", String(t.id));
        });
      }
      if (existing()) {
        if (!model.selectedAll) {
          ids();
        }
        form.append("destination_topic_id", String(state.selectedTopic.id));
        if (state.selection === "existing_message") {
          form.append("participants", state.participants.join(","));
          form.append("archetype", "private_message");
        }
        form.append("chronological_order", String(state.chronologicalOrder));
      } else if (!model.selectedAll) {
        form.append("title", state.topicName);
        ids();
        if (state.selection === "new_topic") {
          form.append("category_id", state.categoryId ? String(state.categoryId) : "");
          tagIds();
        } else {
          tagIds();
          form.append("archetype", "private_message");
        }
      }
      return form;
    }

    function performMove() {
      state.saving = true;
      modalFlash(submit, null);
      sync();
      var headers = memberHeaders();
      headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      var response = null;
      fetch(base() + "/t/" + cfg.topic_id + (model.selectedAll ? "/merge-topic" : "/move-posts"), {
        method: "POST",
        headers: headers,
        credentials: "same-origin",
        body: params().toString(),
      })
        .then(function (r) {
          response = r;
          return r.json().catch(function () {
            return null;
          });
        })
        .then(function (result) {
          // moveResult: anything but success is an error.
          if (!response.ok || !result || !result.success) {
            return Promise.reject(response.ok ? null : result);
          }
          close();
          if (current()) {
            toggle();
          }
          // DiscourseURL.routeTo(result.url)
          window.location.assign(result.url);
        })
        .catch(function (body) {
          if (body instanceof Error) {
            console.error(body);
            body = null;
            response = null;
          }
          modalFlash(submit, extractError(s, response && !response.ok ? response : null, body, s["topic.move_to.error"]));
          state.saving = false;
          sync();
        });
    }

    function radio(value, id, key) {
      return (
        '<label class="radio-label" for="' +
        id +
        '"><input id="' +
        id +
        '" name="move-to-entity" type="radio" value="' +
        value +
        '"' +
        (state.selection === value ? " checked" : "") +
        ">" +
        escapeHtml(s[key]) +
        "</label>"
      );
    }

    close = Discourse.modal.show({
      className: "choose-topic-modal",
      attributes: { id: "choosing-topic" },
      title: s["topic.move_to.title"],
      body: function (body) {
        bodyEl = body;
        var radios = document.createElement("div");
        radios.className = "radios";
        var newMessage = canSplitToPM ? radio("new_message", "move-to-new-message", "topic.move_to_new_message.radio_label") : "";
        radios.innerHTML = cfg.private_message
          ? newMessage + radio("existing_message", "move-to-existing-message", "topic.move_to_existing_message.radio_label")
          : (canSplitTopic ? radio("new_topic", "move-to-new-topic", "topic.split_topic.radio_label") : "") +
            radio("existing_topic", "move-to-existing-topic", "topic.merge_topic.radio_label") +
            newMessage;
        radios.addEventListener("change", function (event) {
          state.selection = event.target.value;
          draw();
        });
        body.appendChild(radios);
      },
      footer: function (footer) {
        submit = el(button("btn-primary", "", null, "right-from-bracket"));
        footer.appendChild(submit);
        submit.addEventListener("click", performMove);
        draw();
      },
    });
  }

  // The posts' select controls.
  document.addEventListener("click", function (event) {
    var clicked = event.target.closest(".select-posts button");
    if (!clicked || !current()) {
      return;
    }
    var article = clicked.closest("article[data-post-id]");
    var id = article && parseInt(article.dataset.postId, 10);
    if (!id) {
      return;
    }
    if (clicked.classList.contains("select-post")) {
      // togglePostSelection
      var at = mode.ids.indexOf(id);
      if (at === -1) {
        mode.ids.push(id);
      } else {
        mode.ids.splice(at, 1);
      }
    } else if (clicked.classList.contains("select-below")) {
      // selectBelow: the post and every one after it in the stream.
      var all = stream();
      var from = all.indexOf(id);
      mode.ids = union(mode.ids, withoutSmallActions(from === -1 ? [] : all.slice(from), loadedPosts()));
    } else if (clicked.classList.contains("select-replies")) {
      selectReplies(id);
      return;
    }
    draw();
  });

  Discourse.selectPosts = {
    // The topic admin menu's Select Posts: `topic` its topic, `helpers`
    // its { deleteTopic(topic) }.
    toggle: toggle,
    // The post admin menu's Change Ownership, for its `post`
    // ({ id, username }).
    changeOwner: function (post) {
      var cfg = config();
      if (cfg) {
        changeOwner(cfg, { ids: [post.id], count: 1, username: post.username });
      }
    },
  };
})();
