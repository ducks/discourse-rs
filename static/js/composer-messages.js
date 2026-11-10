// components/composer-messages.gjs and composer-message.gjs: the popups
// above the composer (.composer-popup-container, drawn by composer_view;
// none show in the mobile view, as Ember has no ComposerMessages there).
// As the composer opens it asks GET /composer_messages for its action,
// topic and post (ComposerMessagesFinder); those waiting for
// typing show once a key is released, the others at once. Typing a
// message to oneself, or to someone not seen in a while, warns too.
// composer-messages:create puts up one message in place of the others
// (the mention and duplicate link warnings), and each is dismissed by its
// close button or Esc. The education message shows once until a post is
// saved. Also here, composer-editor's preview half: the preview's mentions
// linked once /composer/mentions answers (lib/link-mentions), with their
// warnings, and the composer service's duplicate link check
// (afterRefresh, lib/link-lookup).
//
//   Discourse.composerMessages.attach(container, hooks)
//       container  the .composer-popup-container
//       hooks.model()  the composer's state: { action, topicId, postId,
//                  topic: { url, postsCount, privateMessage }, post:
//                  { postNumber, createdAt }, whisper, whispering,
//                  privateMessage, recipients: [{ name, type }],
//                  targetRecipients, draftKey }, or null when closed
//       hooks.ready()  a promise for the topic and post loading
//       hooks.t(key, params)  the composer's strings
//       hooks.siteSettings, hooks.username, hooks.basePath,
//       hooks.canInviteToForum
//       hooks.switchPM(username), hooks.share()  get a room's and the
//                  dominating topic's buttons
//       hooks.mobileView()  the page is in the mobile layout
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (Discourse.composerMessages) {
    return;
  }

  // INPUT_DELAY, and composer-editor's debounces.
  var INPUT_DELAY = 250;
  var DEBOUNCE_FETCH_MS = 450;
  var DEBOUNCE_JIT_MS = 2000;
  var NEW_PRIVATE_MESSAGE_KEY = "new_private_message";

  var messagesCache = {};
  var educationMessageShown = false;

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");
  }

  function svg(name) {
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  function getJson(url) {
    return fetch(url, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(url + " " + r.status);
      }
      return r.json();
    });
  }

  function query(params) {
    var parts = [];
    Object.keys(params).forEach(function (key) {
      var value = params[key];
      if (value === undefined || value === null) {
        return;
      }
      if (Array.isArray(value)) {
        value.forEach(function (v) {
          parts.push(encodeURIComponent(key) + "[]=" + encodeURIComponent(v));
        });
      } else {
        parts.push(encodeURIComponent(key) + "=" + encodeURIComponent(value));
      }
    });
    return parts.join("&");
  }

  // lib/link-lookup: links of the topic already posted, each warned of
  // once a page.
  var warned = {};
  var NO_RESULT = [false, null];

  function LinkLookup(links) {
    this.links = links;
  }

  LinkLookup.prototype.check = function (post, href) {
    if (warned[href]) {
      return NO_RESULT;
    }
    var normalized = href
      .toLowerCase()
      .replace(/^https?:\/\//, "")
      .replace(/\/$/, "");
    if (warned[normalized]) {
      return NO_RESULT;
    }
    var linkInfo = this.links[normalized];
    if (linkInfo) {
      if (post) {
        // Skip edits to the OP
        if (post.postNumber === 1 || post.postNumber === linkInfo.post_number) {
          return NO_RESULT;
        }
        // Don't warn on older posts
        if (post.createdAt && new Date(post.createdAt) < new Date(Date.now() - 14 * 24 * 3600 * 1000)) {
          return NO_RESULT;
        }
      }
      warned[href] = true;
      warned[normalized] = true;
      return [true, linkInfo];
    }
    return NO_RESULT;
  };

  // lib/link-mentions: what /composer/mentions said of each name.
  var checked = {};
  var foundUsers = {};
  var userReasons = {};
  var foundGroups = {};
  var groupReasons = {};
  var maxGroupMention = null;

  function replaceSpan(element, name, opts, basePath) {
    var a = document.createElement("a");
    if (opts.group) {
      a.href = basePath + "/g/" + name;
      a.innerText = "@" + name;
      a.classList.add("mention-group");
      if (!opts.reason && opts.details) {
        a.dataset.mentionableUserCount = opts.details.user_count;
        a.dataset.maxMentions = maxGroupMention;
      }
    } else {
      a.href = basePath + "/u/" + name.toLowerCase();
      a.innerText = "@" + name;
      a.classList.add("mention");
    }
    a.dataset.name = name;
    if (opts.reason) {
      a.dataset.reason = opts.reason;
      if (opts.details) {
        a.dataset.notifiedUserCount = opts.details.notified_count;
      }
    }
    element.replaceWith(a);
  }

  // linkSeenMentions: the names not looked up yet.
  function linkSeenMentions(element, siteSettings, basePath) {
    var mentions = Array.prototype.slice.call(element.querySelectorAll("span.mention:not(.mention-tested)"));
    if (mentions.length === 0) {
      return [];
    }
    var names = mentions.map(function (mention) {
      return mention.innerText.slice(1);
    });
    mentions.forEach(function (mention, index) {
      var name = names[index];
      var lower = name.toLowerCase();
      if (foundUsers[lower]) {
        replaceSpan(mention, name, { reason: userReasons[lower] }, basePath);
      } else if (foundGroups[lower]) {
        replaceSpan(mention, name, { group: true, details: foundGroups[lower], reason: groupReasons[lower] }, basePath);
      } else if (checked[lower]) {
        mention.classList.add("mention-tested");
      }
    });
    var min = parseInt(siteSettings.min_username_length, 10) || 0;
    return names.filter(function (name, index) {
      return names.indexOf(name) === index && !checked[name.toLowerCase()] && name.length >= min;
    });
  }

  function fetchUnseenMentions(basePath, names, topicId, allowedNames) {
    return getJson(
      basePath + "/composer/mentions?" + query({ names: names, topic_id: topicId, allowed_names: allowedNames })
    ).then(function (response) {
      names.forEach(function (name) {
        checked[name.toLowerCase()] = true;
      });
      response.users.forEach(function (username) {
        foundUsers[username.toLowerCase()] = true;
      });
      Object.keys(response.user_reasons).forEach(function (username) {
        userReasons[username.toLowerCase()] = response.user_reasons[username];
      });
      Object.keys(response.groups).forEach(function (name) {
        foundGroups[name.toLowerCase()] = response.groups[name];
      });
      Object.keys(response.group_reasons).forEach(function (name) {
        groupReasons[name.toLowerCase()] = response.group_reasons[name];
      });
      maxGroupMention = response.max_users_notified_per_group_mention;
      return response;
    });
  }

  // lib/formatter shortDate: moment's "MMM D, YYYY" (dates.medium.date_year).
  function shortDate(value, format) {
    var date = new Date(value);
    var month = new Intl.DateTimeFormat(document.documentElement.lang || "en", { month: "short" }).format(date);
    return (format || "MMM D, YYYY")
      .replace("YYYY", String(date.getFullYear()))
      .replace("MMM", month)
      .replace(/\bD\b/, String(date.getDate()));
  }

  // @debounce: the last call, `wait` ms after calls stop.
  function debounced(fn, wait) {
    var timer = null;
    var run = function () {
      var args = arguments;
      clearTimeout(timer);
      timer = setTimeout(function () {
        timer = null;
        fn.apply(null, args);
      }, wait);
    };
    run.cancel = function () {
      clearTimeout(timer);
      timer = null;
    };
    return run;
  }

  function attach(container, hooks) {
    var t = hooks.t;
    var basePath = hooks.basePath || "";
    var messages = [];
    var messagesByTemplate = {};
    var queuedForTyping = [];
    var checkedMessages = false;
    var usersNotSeen = null;
    var recipientNames = [];
    var linkLookup = null;
    // composer-editor's, for the composer it belongs to.
    var warnedCannotSeeMentions = [];
    var warnedGroupMentions = [];
    var generation = 0;

    function closeButton() {
      return (
        '<button class="btn btn-icon-text close btn-default" aria-label="' + escapeHtml(t("composer.esc_label")) +
        '" type="button">' + svg("xmark") + '<span class="d-button-label">' + escapeHtml(t("composer.esc")) +
        "</span></button>"
      );
    }

    function primaryButton(action, icon, label) {
      return (
        '<button class="btn btn-icon-text btn-primary" data-action="' + action + '" type="button">' + svg(icon) +
        '<span class="d-button-label">' + escapeHtml(t(label)) + "</span></button>"
      );
    }

    // ComposerMessage and the templates it picks by templateName.
    function messageHtml(message) {
      var body = message.body || "";
      var content;
      switch (message.templateName) {
        case "education":
          content = (message.title ? "<h3>" + escapeHtml(message.title) + "</h3>" : "") + body;
          break;
        case "dominating-topic":
          content = body + (hooks.canInviteToForum ? primaryButton("share", "link", "footer_nav.share") : "");
          break;
        case "get-a-room":
          content = body + primaryButton("switch-pm", "envelope", "user.private_message");
          break;
        case "group-mentioned":
          content = "<p>" + body + "</p>";
          break;
        default:
          // eslint-disable-next-line no-console
          console.error("Composer message component not found for template name: " + message.templateName);
          return '<div class="composer-popup' + (message.extraClass ? " " + escapeHtml(message.extraClass) : "") + '"></div>';
      }
      return (
        '<div class="composer-popup' + (message.extraClass ? " " + escapeHtml(message.extraClass) : "") + '">' +
        closeButton() + '<div class="composer-popup__content">' + content + "</div></div>"
      );
    }

    // ComposerMessages is rendered: not in the mobile view.
    function active() {
      return !!container && !hooks.mobileView();
    }

    function render() {
      if (!container) {
        return;
      }
      container.innerHTML = (active() ? messages : []).map(messageHtml).join("");
      Array.prototype.forEach.call(container.querySelectorAll(".composer-popup"), function (el, index) {
        el.composerMessage = messages[index];
      });
    }

    function removeMessage(message) {
      var index = messages.indexOf(message);
      if (index !== -1) {
        messages.splice(index, 1);
      }
      render();
    }

    function popup(message) {
      if (messages.length) {
        return;
      }
      if (!messagesByTemplate[message.templateName]) {
        messages.push(message);
        messagesByTemplate[message.templateName] = message;
        render();
      }
    }

    // reset: the messages and what was checked, for a new composer.
    function reset() {
      messages = [];
      messagesByTemplate = {};
      queuedForTyping = [];
      checkedMessages = false;
      render();
    }

    if (container) {
      container.addEventListener("click", function (event) {
        var el = event.target.closest(".composer-popup");
        var message = el && el.composerMessage;
        if (!message) {
          return;
        }
        if (event.target.closest(".btn.close")) {
          event.preventDefault();
          removeMessage(message);
        } else if (event.target.closest('[data-action="switch-pm"]')) {
          hooks.switchPM(message.reply_username);
          removeMessage(message);
        } else if (event.target.closest('[data-action="share"]')) {
          hooks.share();
        }
      });
    }

    // _findMessages, on composer:opened.
    function opened() {
      if (!active() || checkedMessages) {
        return;
      }
      var mine = generation;
      Promise.resolve(hooks.ready())
        .then(function () {
          var model = hooks.model();
          if (mine !== generation || !model || checkedMessages) {
            return null;
          }
          var args = { composer_action: model.action };
          if (model.topicId) {
            args.topic_id = model.topicId;
          }
          if (model.postId) {
            args.post_id = model.postId;
          }
          var cacheKey = String(args.composer_action) + args.topic_id + args.post_id;
          var found;
          if (messagesCache.cacheKey === cacheKey) {
            found = Promise.resolve(messagesCache.messages);
          } else {
            found = getJson(basePath + "/composer_messages?" + query(args)).then(function (json) {
              messagesCache = { messages: json, cacheKey: cacheKey };
              return json;
            });
          }
          return found.then(function (json) {
            if (mine !== generation || checkedMessages) {
              return;
            }
            // Checking composer messages on replies can give us a list of
            // links to check for duplicates
            if (json.extras && json.extras.duplicate_lookup) {
              linkLookup = new LinkLookup(json.extras.duplicate_lookup);
            }
            checkedMessages = true;
            (json.composer_messages || [])
              .filter(function (msg) {
                return !educationMessageShown || msg.id !== "education";
              })
              .forEach(function (msg) {
                if (msg.wait_for_typing) {
                  if (queuedForTyping.indexOf(msg) === -1) {
                    queuedForTyping.push(msg);
                  }
                } else {
                  popup(msg);
                  if (msg.id === "education") {
                    educationMessageShown = true;
                  }
                }
              });
          });
        })
        .catch(function (e) {
          // eslint-disable-next-line no-console
          console.error(e);
        });
    }

    // _typedReply: some messages only show once a reply is typed.
    var typedReply = debounced(function () {
      var model = hooks.model();
      if (!model) {
        return;
      }
      var queued = queuedForTyping.slice();
      queuedForTyping.length = 0;
      queued.forEach(function (msg) {
        if (model.whisper && msg.hide_if_whisper) {
          return;
        }
        popup(msg);
        if (msg.id === "education") {
          educationMessageShown = true;
        }
      });

      if (!model.privateMessage) {
        return;
      }
      var recipients = model.recipients || [];
      if (
        recipients.length > 0 &&
        recipients.every(function (r) {
          return r.name === hooks.username;
        })
      ) {
        popup({
          id: "yourself_confirm",
          templateName: "education",
          title: t("composer.yourself_confirm.title"),
          body: t("composer.yourself_confirm.body"),
        });
      }

      var names = recipients
        .filter(function (r) {
          return r.type === "user";
        })
        .map(function (r) {
          return r.name;
        });
      if (
        names.length > 0 &&
        names.length !== recipientNames.length &&
        !names.every(function (v, i) {
          return v === recipientNames[i];
        })
      ) {
        recipientNames = names;
        var mine = generation;
        getJson(basePath + "/composer_messages/user_not_seen_in_a_while?" + query({ usernames: names })).then(
          function (response) {
            if (mine !== generation) {
              return;
            }
            if (response.user_count > 0 && usersNotSeen !== response.usernames.join("-")) {
              usersNotSeen = response.usernames.join("-");
              messagesByTemplate.education = undefined;
              var usernames = response.usernames.map(function (username) {
                return "<a class='mention' href='" + basePath + "/u/" + username + "'>@" + username + "</a>";
              });
              var key =
                response.user_count === 1
                  ? "composer.user_not_seen_in_a_while.single"
                  : "composer.user_not_seen_in_a_while.multiple";
              popup({
                id: "user-not-seen",
                templateName: "education",
                body: t(key, { usernames: usernames.join(", "), time_ago: response.time_ago }),
              });
            }
          },
          function (e) {
            // eslint-disable-next-line no-console
            console.error(e);
          }
        );
      }
    }, INPUT_DELAY);

    // _create: a message in place of the others.
    function create(info) {
      if (!active()) {
        return;
      }
      setTimeout(function () {
        reset();
        popup(Object.assign({}, info));
      }, 0);
    }

    // The composer service's groupsMentioned, cannotSeeMention and
    // hereMention.
    function groupsMentioned(name, userCount, maxMentions) {
      var model = hooks.model();
      if (!model || model.privateMessage || (model.topic && model.topic.privateMessage)) {
        return;
      }
      maxMentions = parseInt(maxMentions, 10);
      userCount = parseInt(userCount, 10);
      var body;
      var groupLink = basePath + "/g/" + name + "/members";
      if (userCount > maxMentions) {
        body = t("composer.group_mentioned_limit", { group: "@" + name, count: maxMentions, group_link: groupLink });
      } else if (userCount > 0) {
        // Louder warning for a larger group.
        var key = userCount >= 5 ? "composer.larger_group_mentioned" : "composer.group_mentioned";
        body = t(key, { group: "@" + name, count: userCount, group_link: groupLink });
      }
      if (body) {
        create({ extraClass: "custom-body", templateName: "education", body: body });
      }
    }

    function cannotSeeMention(name, reason, notifiedCount, isGroup) {
      notifiedCount = parseInt(notifiedCount, 10);
      var body = isGroup
        ? t("composer.cannot_see_group_mention." + reason, { group: name, count: notifiedCount })
        : t("composer.cannot_see_mention." + reason, { username: name });
      create({ extraClass: "custom-body", templateName: "education", body: body });
    }

    function hereMention(count) {
      create({
        extraClass: "custom-body",
        templateName: "education",
        body: t("composer.here_mention", { here: hooks.siteSettings.here_mention, count: count }),
      });
    }

    function isInQuote(element) {
      var parent = element.parentElement;
      while (parent && !(parent.tagName === "DIV" && parent.classList.contains("d-editor-preview"))) {
        if (parent.tagName === "ASIDE" && parent.classList.contains("quote")) {
          return true;
        }
        parent = parent.parentElement;
      }
      return false;
    }

    // _warnMentionedGroups
    var warnMentionedGroups = debounced(function (preview) {
      preview.querySelectorAll(".mention-group[data-mentionable-user-count]").forEach(function (mention) {
        var name = mention.dataset.name;
        if (warnedGroupMentions.indexOf(name) !== -1 || isInQuote(mention)) {
          return;
        }
        warnedGroupMentions.push(name);
        groupsMentioned(name, mention.dataset.mentionableUserCount, mention.dataset.maxMentions);
      });
    }, DEBOUNCE_JIT_MS);

    // _warnCannotSeeMention: after a pause, so @bob is not warned of
    // while @bob2 is being typed.
    var warnCannotSeeMention = debounced(function (preview) {
      var model = hooks.model();
      if (model && model.draftKey === NEW_PRIVATE_MESSAGE_KEY) {
        return;
      }
      preview.querySelectorAll(".mention[data-reason]").forEach(function (mention) {
        var name = mention.dataset.name;
        if (warnedCannotSeeMentions.indexOf(name) !== -1) {
          return;
        }
        warnedCannotSeeMentions.push(name);
        cannotSeeMention(name, mention.dataset.reason);
      });
      preview.querySelectorAll(".mention-group[data-reason]").forEach(function (mention) {
        var name = mention.dataset.name;
        if (warnedCannotSeeMentions.indexOf(name) !== -1) {
          return;
        }
        warnedCannotSeeMentions.push(name);
        cannotSeeMention(name, mention.dataset.reason, mention.dataset.notifiedUserCount, true);
      });
    }, DEBOUNCE_JIT_MS);

    // _renderUnseenMentions
    var renderUnseenMentions = debounced(function (preview, unseen) {
      var model = hooks.model();
      if (!model) {
        return;
      }
      var allowed = model.targetRecipients ? model.targetRecipients.split(",") : undefined;
      fetchUnseenMentions(basePath, unseen, model.topicId || undefined, allowed).then(
        function (response) {
          linkSeenMentions(preview, hooks.siteSettings, basePath);
          warnMentionedGroups(preview);
          warnCannotSeeMention(preview);
          if (response.here_count) {
            hereMention(response.here_count);
          }
        },
        function (e) {
          // eslint-disable-next-line no-console
          console.error(e);
        }
      );
    }, DEBOUNCE_FETCH_MS);

    // _renderMentions
    function renderMentions(preview) {
      var unseen = linkSeenMentions(preview, hooks.siteSettings, basePath);
      if (unseen.length > 0) {
        renderUnseenMentions(preview, unseen);
      } else {
        warnMentionedGroups(preview);
        warnCannotSeeMention(preview);
      }
    }

    // The composer service's afterRefresh: a link the topic already has.
    function afterRefresh(preview) {
      var model = hooks.model();
      var topic = model && model.topic;
      if (!topic || !linkLookup) {
        return;
      }
      // Don't check if there's only one post
      if (topic.postsCount === 1) {
        return;
      }
      preview.querySelectorAll("a[href]").forEach(function (l) {
        var href = l.href;
        if (!href || !href.length) {
          return;
        }
        // skip links added by watched words
        if (l.dataset.word !== undefined) {
          return;
        }
        // skip links in quotes and oneboxes
        for (var element = l; element; element = element.parentElement) {
          if (element.tagName === "DIV" && element.classList.contains("d-editor-preview")) {
            break;
          }
          if (element.tagName === "ASIDE" && element.classList.contains("quote")) {
            return;
          }
          // Ember reads dataset["onebox-src"], which is never set: any
          // link in a onebox is skipped.
          if (element.tagName === "ASIDE" && element.classList.contains("onebox")) {
            return;
          }
        }
        var result = linkLookup.check(model.post, href);
        var linkInfo = result[1];
        if (result[0] && !model.whispering) {
          var postUrl = topic.url + (linkInfo.post_number > 0 ? "/" + linkInfo.post_number : "");
          var ago = shortDate(linkInfo.posted_at, t("dates.medium.date_year"));
          if (linkInfo.username === hooks.username) {
            create({
              extraClass: "custom-body",
              templateName: "education",
              body: t("composer.duplicate_link_same_user", { domain: linkInfo.domain, post_url: postUrl, ago: ago }),
            });
          } else {
            create({
              extraClass: "custom-body duplicate-link-message",
              templateName: "education",
              body: t("composer.duplicate_link", {
                domain: linkInfo.domain,
                username: linkInfo.username,
                post_url: postUrl,
                ago: ago,
              }),
            });
          }
        }
      });
    }

    return {
      // The composer opened from closed: what the last one checked goes.
      reset: function () {
        generation++;
        typedReply.cancel();
        warnMentionedGroups.cancel();
        warnCannotSeeMention.cancel();
        renderUnseenMentions.cancel();
        warnedCannotSeeMentions = [];
        warnedGroupMentions = [];
        linkLookup = null;
        reset();
      },
      opened: opened,
      typedReply: function () {
        if (active()) {
          typedReply();
        }
      },
      // composer:saved
      saved: function () {
        educationMessageShown = false;
      },
      create: create,
      // composer-messages:close
      closeTop: function () {
        messages.pop();
        render();
      },
      count: function () {
        return messages.length;
      },
      // hidden: the composer neither open nor fullscreen.
      setHidden: function (hidden) {
        if (container) {
          container.classList.toggle("hidden", hidden);
        }
      },
      // composer-editor's previewUpdated: the mentions, then afterRefresh.
      previewUpdated: function (preview) {
        renderMentions(preview);
        afterRefresh(preview);
      },
    };
  }

  Discourse.composerMessages = {
    attach: attach,
    // resetComposerMessagesCache and the lookups' resets, for tests.
    resetCaches: function () {
      messagesCache = {};
      educationMessageShown = false;
      warned = {};
      checked = {};
      foundUsers = {};
      userReasons = {};
      foundGroups = {};
      groupReasons = {};
      maxGroupMention = null;
    },
  };
})();
