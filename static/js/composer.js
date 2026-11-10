// The composer (#reply-control, rendered by composer_view): opens to
// reply to a topic or a post, create a topic or edit a post, and posts
// through /posts as Ember's composer does. Its height, sizing, minimizing
// and fullscreen follow services/composer.js and composer-container; the
// toolbar (editor-toolbar.js, shipped in front of this file) acts through
// the open editor as d-editor's buttons do.
(function () {
  "use strict";

  var control = document.getElementById("reply-control");
  // Bound once per composer: it stays across pages (hx-preserve, page.js),
  // and its script tag comes back with each. A page without one drops it;
  // the next composer is bound afresh and this one's listeners go quiet.
  if (!control || control.dataset.bound) {
    return;
  }
  control.dataset.bound = "true";
  var root = document.documentElement;
  var textarea = control.querySelector(".d-editor-input");
  var title = control.querySelector("#reply-title");
  var fields = control.querySelector(".title-and-category");
  var chooser = control.querySelector(".category-chooser");
  var actionButton = control.querySelector(".composer-actions");
  var submit = control.querySelector(".save-or-cancel .create");
  var error = control.querySelector(".composer-error");
  var data = control.dataset;
  var state = null;
  var base = control.dataset.basePath || "";
  // The message composer's recipients (PmUserSelector) and the staff's
  // official warning toggle.
  var userSelector = control.querySelector(".user-selector");
  var warning = control.querySelector(".add-warning");
  var titlePlaceholder = title.placeholder;
  var recipients = null;

  // ComposerMessages (composer-messages.js, shipped in front of this
  // file): the popups above the composer, from its state.
  var composerMessages = Discourse.composerMessages.attach(control.querySelector(".composer-popup-container"), {
    model: messagesModel,
    ready: function () {
      var s = state;
      return Promise.all(
        s
          ? [s.actionContext, s.editPostLoad, s.editTopicLoad].map(function (p) {
              return p && p.catch(function () {});
            })
          : []
      );
    },
    t: function (key, params) {
      return richI18n(key, params);
    },
    get siteSettings() {
      return richContextSettings();
    },
    username: data.username,
    basePath: base,
    get canInviteToForum() {
      return !!(richContext().currentUser || {}).can_invite_to_forum;
    },
    switchPM: switchPM,
    share: shareTopic,
    mobileView: mobileView,
  });

  // The composer's action as Ember names it (Composer.REPLY, ...).
  var EMBER_ACTIONS = {
    reply: "reply",
    "create-topic": "createTopic",
    "private-message": "privateMessage",
    edit: "edit",
  };

  // The composer model as ComposerMessages reads it: its action, the topic
  // and post (replied to, or edited), the whisper toggle and
  // isWhispering, a message's recipients (targetRecipientsArray), and
  // what _findSimilar reads of a new topic: its title, its text and
  // whether its category has form templates (form_template_ids, the
  // chooser row's data-form-template-ids).
  function messagesModel() {
    var s = state;
    if (!s) {
      return null;
    }
    var topic = s.actionTopic || s.editTopic || null;
    var post = null;
    if (s.mode === "edit" && s.editPost) {
      post = { postNumber: s.editPost.post_number, createdAt: s.editPost.created_at };
    } else if (s.mode !== "edit" && s.actionPost) {
      post = { postNumber: s.actionPost.postNumber, createdAt: s.actionPost.createdAt };
    }
    var message = s.mode === "private-message";
    var creatingTopic = s.mode === "create-topic";
    var categoryId = creatingTopic ? chooser.querySelector("summary").dataset.value : null;
    var categoryRow = categoryId ? chooser.querySelector('.category-row[data-value="' + categoryId + '"]') : null;
    return {
      action: EMBER_ACTIONS[s.mode],
      topicId: s.mode === "reply" || s.mode === "edit" ? s.topicId || null : null,
      postId: s.mode === "edit" ? s.postId || null : s.actionPost ? s.actionPost.id : null,
      topic: topic ? { url: topic.url, postsCount: topic.postsCount, privateMessage: topic.privateMessage } : null,
      post: post,
      whisper: !!s.whisper,
      whispering: isWhispering(s),
      privateMessage: message,
      recipients: message && recipients && recipients.types ? recipients.types() : [],
      targetRecipients: message ? recipientsValue().join(",") : null,
      draftKey: s.draftKey,
      creatingTopic: creatingTopic,
      title: title.value,
      reply: textarea.value,
      categoryHasFormTemplates: !!(categoryRow && categoryRow.dataset.formTemplateIds),
    };
  }

  // ComposerMessages' switchPM: get a room's reply goes to its user as a
  // message, with what was written.
  function switchPM(username) {
    var s = state;
    if (!s) {
      return;
    }
    switchComposer(
      {
        mode: "private-message",
        carry: true,
        draftKey: s.draftKey,
        draftSequence: s.draftSequence,
        reply: textarea.value,
        recipients: username ? [username] : [],
      },
      s
    );
  }

  // The dominating topic's Share: ShareTopic (share-topic.js, a topic
  // page's) for the page's topic.
  function shareTopic() {
    if (Discourse.shareTopic) {
      Discourse.shareTopic.show();
    }
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["Content-Type"] = "application/x-www-form-urlencoded";
    return headers;
  }

  function getItem(key) {
    try {
      return window.localStorage.getItem("discourse_" + key);
    } catch (e) {
      return null;
    }
  }

  function setItem(key, value) {
    try {
      window.localStorage.setItem("discourse_" + key, value);
    } catch (e) {
      // Storage blocked: the height lasts for this page.
    }
  }

  // _getDefaultComposerHeight
  function defaultHeight(mode) {
    return (
      getItem("composerHeight") ||
      (mode === "reply"
        ? "var(--reply-composer-height, 255px)"
        : "var(--new-topic-composer-height, 400px)")
    );
  }

  // canEditTitle: creating a topic, or editing a first post (with its
  // title, category and tags above the editor).
  // A message (creatingPrivateMessage) also edits its title, with its
  // recipients above it and no category; tags only where messages may
  // take them (site.can_tag_pms).
  function setMode(mode, firstPost) {
    var pm = mode === "private-message";
    var titled = mode === "create-topic" || pm || (mode === "edit" && !!firstPost);
    ["reply", "create-topic", "edit", "private-message"].forEach(function (m) {
      control.classList.toggle("composer-action-" + m, m === mode);
    });
    control.classList.toggle("private-message", pm);
    control.classList.toggle("edit-title", titled);
    control.classList.toggle("topic", mode === "create-topic");
    fields.hidden = !titled;
    title.disabled = false;
    chooser.closest(".category-input").hidden = pm;
    replyArea.classList.toggle("with-category", !pm);
    var tagsInput = control.querySelector(".tags-input");
    if (tagsInput) {
      tagsInput.hidden = pm && !data.canTagPms;
      replyArea.classList.toggle("with-tags", !tagsInput.hidden);
    }
    if (userSelector) {
      userSelector.hidden = !pm;
    }
    title.placeholder = pm ? data.labelTitleMessage : titlePlaceholder;
    title.setAttribute("aria-label", title.placeholder);
    // The actions trigger and the save button (saveLabel and saveIcon).
    renderActions();
  }

  // The mode as the data attributes name it.
  function modeName(mode) {
    return { reply: "Reply", "create-topic": "CreateTopic", edit: "Edit", "private-message": "PrivateMessage" }[mode];
  }

  function showError(messages) {
    error.textContent = messages;
    error.hidden = !messages;
  }

  // Drafts, as models/composer's saveDraft and services/composer's
  // _saveDraft keep them: saved two seconds after typing stops (at once
  // when the last save is fifteen seconds old), on minimizing, and by
  // beacon when the page is left with a save pending; restored when the
  // composer opens on the same key. A reply and an edit share the topic's
  // key (`topic_<id>`), a new topic has its own (`new_topic_<time>`).
  // The owner is this page, as MessageBus's client id is in Ember.
  var clientId = Array.from(crypto.getRandomValues(new Uint8Array(16)), function (b) {
    return b.toString(16).padStart(2, "0");
  }).join("");
  var draftStatus = control.querySelector("#draft-status");
  var draftTimer = null;
  var lastDraftSaved = null;
  var draftSaving = null;

  // The composer's draftStatus: an error while drafts cannot be saved.
  function showDraftStatus(text) {
    var span = draftStatus.querySelector(".draft-error");
    span.title = text || "";
    while (span.childNodes.length > 1) {
      span.removeChild(span.lastChild);
    }
    if (text) {
      span.appendChild(document.createTextNode(text));
    }
    draftStatus.hidden = !text;
  }

  // The composer's action as a draft names it.
  function draftAction(mode) {
    return { reply: "reply", "create-topic": "createTopic", edit: "edit", "private-message": "privateMessage" }[mode];
  }

  // _draft_serializer's fields the composer has.
  function draftData(s) {
    var d = { reply: textarea.value, action: draftAction(s.mode), archetypeId: "regular" };
    d.composerTime = composerTime(s);
    d.typingTime = s.typingTime;
    d.original_text = s.originalText === undefined ? null : s.originalText;
    if (s.mode === "create-topic") {
      d.title = title.value;
      var category = chooser.querySelector("summary").dataset.value;
      d.categoryId = category ? parseInt(category, 10) : null;
      if (tagChooser) {
        d.tags = serializedTags();
      }
    } else if (s.mode === "private-message") {
      d.title = title.value;
      d.categoryId = null;
      d.tags = data.canTagPms ? serializedTags() : [];
      d.archetypeId = "private_message";
      d.recipients = recipientsValue().join(",");
    } else if (s.mode === "edit") {
      d.postId = parseInt(s.postId, 10);
      // _draft_serializer's reply target: one changed and not yet saved
      // survives the draft.
      if (s.editReplyTo !== undefined) {
        d.reply_to_post_number = s.editReplyTo;
        d.reply_to_user = s.editReplyToUser || null;
      }
      // A first post's draft keeps the topic fields being edited and what
      // they were.
      if (s.topic) {
        d.title = title.value;
        var editCategory = chooser.querySelector("summary").dataset.value;
        d.categoryId = editCategory ? parseInt(editCategory, 10) : null;
        if (tagChooser) {
          d.tags = serializedTags();
        }
        d.original_title = s.topic.title;
        d.original_tags = s.topic.tags;
      }
    } else {
      d.reply_to_post_number = s.replyTo ? parseInt(s.replyTo, 10) : null;
      if (s.whisper !== undefined) {
        d.whisper = !!s.whisper;
      }
    }
    return d;
  }

  // canSaveDraft
  function canSaveDraft() {
    if (!state || state.loading || draftSaving) {
      return false;
    }
    if (state.mode === "create-topic" || state.mode === "private-message") {
      return textarea.value !== "" || title.value !== "";
    }
    return textarea.value !== "";
  }

  function saveDraft() {
    clearTimeout(draftTimer);
    draftTimer = null;
    if (draftSaving) {
      draftTimer = setTimeout(saveDraft, 2000);
      return draftSaving;
    }
    if (!canSaveDraft()) {
      return Promise.resolve();
    }
    var s = state;
    var sequence = s.draftSequence;
    s.draftSequence += 1;
    draftSaving = fetch(base + "/drafts.json", {
      method: "POST",
      headers: csrfHeaders(),
      credentials: "same-origin",
      body: form([
        ["draft_key", s.draftKey],
        ["sequence", sequence],
        ["data", JSON.stringify(draftData(s))],
        ["owner", clientId],
        ["force_save", s.forceSave ? "true" : "false"],
      ]),
    })
      .then(function (r) {
        return r
          .json()
          .catch(function () {
            return null;
          })
          .then(function (json) {
            if (r.ok && json) {
              if ("draft_sequence" in json) {
                s.draftSequence = json.draft_sequence;
              }
              s.forceSave = false;
              showDraftStatus(json.conflict_user ? data.labelEditConflict : "");
              return;
            }
            var message = null;
            if (r.status === 409 && json && json.errors && json.errors.length) {
              message = json.errors[0];
              // Ember's dialog offers to reload or to ignore the newer
              // draft (forceSave on the next save).
              if (json.extras && json.extras.description) {
                if (window.confirm(json.extras.description)) {
                  window.location.reload();
                } else {
                  s.forceSave = true;
                }
              }
            }
            showDraftStatus(message || data.labelDraftsOffline);
          });
      })
      .catch(function () {
        showDraftStatus(data.labelDraftsOffline);
      })
      .finally(function () {
        draftSaving = null;
        lastDraftSaved = Date.now();
      });
    return draftSaving;
  }

  // _shouldSaveDraft, on the reply and the title changing.
  function scheduleDraft() {
    if (!state || state.loading) {
      return;
    }
    if (!lastDraftSaved) {
      lastDraftSaved = Date.now();
    }
    if (Date.now() - lastDraftSaved > 15000) {
      saveDraft();
    } else {
      clearTimeout(draftTimer);
      draftTimer = setTimeout(saveDraft, 2000);
    }
  }

  // destroyDraft: after a save in flight.
  function destroyDraft(s) {
    clearTimeout(draftTimer);
    draftTimer = null;
    return (draftSaving || Promise.resolve()).then(function () {
      return fetch(base + "/drafts/" + encodeURIComponent(s.draftKey) + ".json", {
        method: "DELETE",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form([
          ["draft_key", s.draftKey],
          ["sequence", s.draftSequence],
        ]),
      });
    });
  }

  // Draft.get: the draft (parsed) and the key's sequence.
  function fetchDraft(key) {
    return fetch(base + "/drafts/" + encodeURIComponent(key) + ".json", {
      credentials: "same-origin",
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (json) {
        var draft = null;
        try {
          draft = json.draft ? JSON.parse(json.draft) : null;
        } catch (e) {
          // loadDraft: a draft that does not parse is dropped.
        }
        return { draft: draft, sequence: json.draft_sequence || 0 };
      });
  }

  // _beaconSaveDraft
  window.addEventListener("beforeunload", function () {
    if (!control.isConnected || !draftTimer || !canSaveDraft()) {
      return;
    }
    clearTimeout(draftTimer);
    draftTimer = null;
    var sequence = state.draftSequence;
    state.draftSequence += 1;
    // Form-encoded rather than Ember's FormData: the server does not
    // parse multipart bodies.
    navigator.sendBeacon(
      base + "/drafts.json",
      new URLSearchParams([
        ["draft_key", state.draftKey],
        ["sequence", sequence],
        ["data", JSON.stringify(draftData(state))],
        ["owner", clientId],
        ["authenticity_token", csrfHeaders()["X-CSRF-Token"] || ""],
      ])
    );
  });

  // The topic of a first post being edited: what the composer shows and
  // what Topic.update sends against (original_title, original_tags).
  function loadTopic(s) {
    return fetch(base + "/t/" + s.topicId + ".json", {
      credentials: "same-origin",
      headers: { Accept: "application/json" },
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (topic) {
        // The topic's tags as it serializes them ({id, name, slug}),
        // which original_tags sends back.
        var tags = topic.tags || [];
        var details = topic.details || {};
        s.topic = {
          id: topic.id,
          slug: topic.slug,
          title: topic.title,
          categoryId: topic.category_id,
          tags: tags,
          privateMessage: topic.archetype === "private_message",
          canEdit: !!details.can_edit,
          canEditTags: !!details.can_edit_tags,
          pmWithNonHumanUser: !!topic.pm_with_non_human_user,
        };
        if (state !== s) {
          return;
        }
        title.value = topic.title;
        // disableTitleInput: editing without the right to edit the topic.
        title.disabled = !s.topic.canEdit;
        // canCategorize: never for a message.
        chooser.closest(".category-input").hidden = s.topic.privateMessage;
        if (topic.category_id) {
          selectCategory(topic.category_id);
          keepCategory(topic.category_id);
        }
        setTags(
          (topic.tags || []).map(function (t) {
            return typeof t === "string" ? { name: t, id: null } : t;
          })
        );
      });
  }

  function rawToEdit(s) {
    return fetch(base + "/raw/" + s.topicId + "/" + s.postNumber, {
      credentials: "same-origin",
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(r.status);
      }
      return r.text();
    });
  }

  // What the composer opens with: the key's draft when it is for this
  // composer, an edit's raw markdown otherwise. A reply opened over an
  // edit's draft opens that edit (loadDraft takes the draft's action).
  function initialText(s, found) {
    var draft = found && found.draft;
    if (found) {
      s.draftSequence = found.sequence;
    }
    // loadDraft: the draft's composerTime and typingTime carry on.
    if (draft) {
      s.composerTotalOpened = draft.composerTime || 0;
      s.typingTime = draft.typingTime || 0;
    }
    var editDraft = draft && draft.action === "edit";
    // selectText: a quote opens the reply with it, after the key's draft
    // when there is one (composerOpts.reply), else as the text it
    // started from (composerOpts.quote).
    if (s.mode === "reply" && s.quote) {
      s.originalText = draft ? null : s.quote;
      return Promise.resolve(draft ? (draft.reply || "") + "\n" + s.quote : s.quote);
    }
    if (s.mode === "reply" && editDraft && draft.postId) {
      s.mode = "edit";
      s.postId = draft.postId;
      setMode("edit");
      loadEditPost(s);
    }
    // originalText: what the composer started from, as a draft keeps it
    // (original_text); an edit's is the post's raw, a new reply's none.
    s.originalText = draft && "original_text" in draft ? draft.original_text : null;
    if (s.mode === "edit") {
      if (editDraft && String(draft.postId) === String(s.postId)) {
        // A null in the draft is no target, kept over the post's.
        if ("reply_to_post_number" in draft) {
          s.editReplyTo = draft.reply_to_post_number;
          s.editReplyToUser = draft.reply_to_user || null;
          renderEditReplyTo(s);
        }
        if (s.topic && typeof draft.title === "string") {
          title.value = draft.title;
          if (draft.categoryId) {
            selectCategory(draft.categoryId);
          }
          if (draft.tags) {
            setTags(draft.tags);
          }
        }
        return Promise.resolve(draft.reply || "");
      }
      return rawToEdit(s).then(function (raw) {
        s.originalText = raw;
        return raw;
      });
    }
    if (draft && !editDraft) {
      if (s.mode === "private-message") {
        title.value = draft.title || "";
        setRecipients(draft.recipients);
        if (draft.tags && data.canTagPms) {
          setTags(draft.tags);
        }
      } else if (s.mode === "create-topic") {
        title.value = draft.title || "";
        if (draft.categoryId) {
          selectCategory(draft.categoryId);
        }
        if (draft.tags) {
          setTags(draft.tags);
        }
      } else if ("reply_to_post_number" in draft) {
        s.replyTo = draft.reply_to_post_number;
      }
      if (s.mode === "reply" && typeof draft.whisper === "boolean") {
        s.whisper = draft.whisper;
        renderActions();
      }
      return Promise.resolve(draft.reply || "");
    }
    return Promise.resolve("");
  }

  // open({mode, topicId, replyTo, replyToUsername, postId, postNumber,
  // category, draftKey, draftSequence}): to reply, create a topic or edit
  // a post; a draft key from the drafts menu resumes that draft.
  // A closed composer holds only its grippie, as Ember renders the reply
  // area once the composer is visible.
  var replyArea = control.querySelector(".reply-area");

  // The mobile view's icon discard: cancelIcon.
  function applyDiscardIcon(mode) {
    var discardIcon = control.querySelector(".discard-button.no-text use");
    if (discardIcon) {
      discardIcon.setAttribute("href", mode === "edit" ? "#xmark" : "#trash-can");
      discardIcon.parentNode.setAttribute(
        "class",
        "fa d-icon d-icon-" + (mode === "edit" ? "xmark" : "trash-can") + " svg-icon fa-width-auto svg-string"
      );
    }
  }

  function open(opts) {
    replyArea.hidden = false;
    if (state && state.mode === opts.mode && control.classList.contains("draft")) {
      expand();
      return;
    }
    state = opts;
    var s = state;
    // clearState's timings.
    s.typingTime = 0;
    s.composerOpened = null;
    s.composerTotalOpened = 0;
    // The topic as the reply-where dialog shows it, should the reply be
    // saved from another topic's page (composer.topic).
    if (opts.mode === "reply") {
      var where = document.querySelector('template.reply-where-topic[data-topic-id="' + opts.topicId + '"]');
      s.whereHtml = where ? where.innerHTML : null;
    }
    // The actions menu's model: the topic and post (loaded for a reply),
    // what the composer was switched from (the snapshot) and the toggles.
    s.actionTopic = opts.actionTopic || null;
    s.actionPost = opts.actionPost || null;
    s.snapshot = opts.snapshot || null;
    s.noBump = !!opts.noBump;
    s.unlistTopic = !!opts.unlistTopic;
    // skipJumpOnSave and onSaved: the review queue's edit stays on its
    // page, then performs once this session is saved.
    s.skipJumpOnSave = !!opts.skipJumpOnSave;
    s.onSaved = opts.onSaved || null;
    setMode(opts.mode);
    applyDiscardIcon(opts.mode);
    textarea.value = "";
    syncRichEditor();
    title.value = "";
    showError("");
    showDraftStatus("");
    lastDraftSaved = null;
    if (opts.mode === "create-topic") {
      selectCategory(opts.category || data.defaultCategory);
      // The new-topic route's topicTitle and topicTags.
      title.value = opts.title || "";
      setTags(opts.tags || []);
    }
    if (opts.mode === "private-message") {
      // composer.open: the recipients, title and tags it was opened with.
      title.value = opts.title || "";
      setRecipients(opts.recipients);
      setTags(opts.tags || []);
      if (warning) {
        warning.querySelector("input").checked = false;
      }
    }
    s.firstPost = opts.mode === "edit" && String(opts.postNumber) === "1";
    if (s.firstPost) {
      setMode("edit", true);
      title.disabled = true;
    }
    control.classList.remove("closed", "draft", "fullscreen");
    control.classList.add("open");
    composerHeight = defaultHeight(opts.mode);
    root.style.setProperty("--composer-height", composerHeight);
    composeStateChanged();
    updatePreview();
    if (richMode) {
      openRichEditor();
    }

    var resuming = Boolean(opts.draftKey);
    var creating = opts.mode === "create-topic" || opts.mode === "private-message";
    if (opts.mode === "create-topic") {
      s.draftKey = opts.draftKey || "new_topic_" + Date.now();
    } else if (opts.mode === "private-message") {
      s.draftKey = opts.draftKey || "new_private_message_" + Date.now();
    } else {
      s.draftKey = (opts.carry && opts.draftKey) || "topic_" + opts.topicId;
    }
    s.draftSequence = opts.draftSequence || 0;
    s.loading = true;
    textarea.disabled = true;
    announce();
    loadActionContext(s);
    loadEditTopic(s);
    loadEditPost(s);
    // composer:opened
    composerMessages.opened();
    // A new topic's key is new: there is no draft to look up. A composer
    // switched from another (the actions menu) carries its text, and
    // takes only the key's sequence.
    var lookup =
      creating && !resuming
        ? Promise.resolve(null)
        : fetchDraft(s.draftKey).catch(function (e) {
            if (opts.carry) {
              return null;
            }
            throw e;
          });
    var prefilled = creating && !resuming && Boolean(opts.title || opts.body);
    // Editing a first post: the topic's title, category and tags, as
    // the topic model holds them in Ember.
    var topicLoad = s.firstPost ? loadTopic(s) : Promise.resolve();
    s.topicLoad = topicLoad;
    Promise.all([lookup, topicLoad])
      .then(function (results) {
        if (opts.carry) {
          if (results[0]) {
            s.draftSequence = results[0].sequence;
          }
          return opts.reply || "";
        }
        // The new-topic route's topicBody.
        if (creating && !resuming && opts.body) {
          return opts.body;
        }
        return initialText(s, results[0]);
      })
      .then(function (text) {
        if (state !== s) {
          return;
        }
        textarea.value = text;
        syncRichEditor();
        updatePreview();
        announce();
      })
      .catch(function () {
        if (state === s) {
          showError(
            s.mode === "edit" ? "Could not load the post to edit." : "Could not load the draft."
          );
        }
      })
      .finally(function () {
        if (state !== s) {
          return;
        }
        s.loading = false;
        textarea.disabled = false;
        // A prefilled composer opened empty, then took the route's title
        // and body: the title has the focus it took then, and what was
        // filled in is saved as a draft once it settles.
        // focusTarget: a message's recipients while it has none but the
        // member, else its title.
        var to = recipientsValue().join(",");
        if (s.mode === "private-message" && recipients && (to === "" || to === data.username)) {
          recipients.focus();
        } else if (s.mode === "private-message" || (s.mode === "create-topic" && (title.value === "" || prefilled))) {
          title.focus();
        } else {
          focusEditor();
        }
        if (prefilled) {
          scheduleDraft();
        }
      });
  }

  function close() {
    if (recipients) {
      recipients.close();
    }
    // ComposerMessages goes with the composer.
    composerMessages.reset();
    clearTimeout(lastKeyTimeout);
    lastKeyTimeout = null;
    cancelUploads();
    clearTimeout(draftTimer);
    draftTimer = null;
    control.classList.remove("open", "draft", "fullscreen");
    control.classList.add("closed");
    replyArea.hidden = true;
    // The composer service's close().
    root.classList.remove("fullscreen-composer", "composer-open");
    root.style.removeProperty("--composer-height");
    syncPreviewClass();
    state = null;
    renderActions();
    announce();
  }

  // The composer's model as discourse-presence's ComposerPresenceDisplay
  // reads it: the action, its topic or post, and replyDirty (the text
  // changed from what it opened with). Null while closed.
  function announce() {
    var detail = null;
    if (state) {
      detail = {
        mode: state.mode,
        topicId: state.topicId || null,
        postId: state.postId || null,
        // replyDirty: the trimmed text against the trimmed original.
        dirty: !state.loading && textarea.value.trim() !== (state.originalText || "").trim(),
      };
    }
    control.dispatchEvent(new CustomEvent("composer:presence", { detail: detail, bubbles: true }));
  }

  function dirty() {
    return textarea.value.trim() !== "" || title.value.trim() !== "";
  }

  // Minimized, the composer is a bar that opens again when clicked; its
  // draft is saved (collapse).
  function minimize() {
    saveDraft();
    control.classList.remove("open", "fullscreen");
    control.classList.add("draft");
    root.style.setProperty("--composer-height", "40px");
    composeStateChanged();
  }

  // unshrink and openIfDraft: open again at the height it had.
  function expand() {
    control.classList.remove("draft", "fullscreen");
    control.classList.add("open");
    root.style.setProperty("--composer-height", composerHeight);
    composeStateChanged();
    composerMessages.opened();
  }

  // Fullscreen is a compose state of its own: #reply-control.fullscreen
  // without .open, so html.composer-open goes while it lasts.
  function toggleFullscreen() {
    saveDraft();
    var on = !control.classList.contains("fullscreen");
    control.classList.remove("open", "draft", "fullscreen");
    control.classList.add(on ? "fullscreen" : "open");
    composeStateChanged();
  }

  // Composer#composeStateChanged's timing: composerOpened while the
  // compose state is open (not minimized, not fullscreen), the time spent
  // so added to composerTotalOpened when it leaves it.
  function composerTiming() {
    if (!state) {
      return;
    }
    var oldOpen = state.composerOpened;
    if (control.classList.contains("open")) {
      state.composerOpened = oldOpen || Date.now();
    } else {
      if (oldOpen) {
        state.composerTotalOpened = (state.composerTotalOpened || 0) + (Date.now() - oldOpen);
      }
      state.composerOpened = null;
    }
  }

  // Composer#composerTime
  function composerTime(s) {
    var total = s.composerTotalOpened || 0;
    if (s.composerOpened) {
      total += Date.now() - s.composerOpened;
    }
    return total;
  }

  // Composer#typing, on composer-body's keyUp: a throttle that adds 100ms
  // at the end of each 100ms a key was released in.
  var typingThrottle = null;
  control.addEventListener("keyup", function () {
    var s = state;
    if (!s || typingThrottle) {
      return;
    }
    typingThrottle = setTimeout(function () {
      typingThrottle = null;
      s.typingTime = (s.typingTime || 0) + 100;
    }, 100);
  });

  // composer-body's keyUp: the composer messages waiting for typing, and
  // one second after the last key's release (the member paused typing),
  // composer:find-similar.
  var lastKeyTimeout = null;
  control.addEventListener("keyup", function () {
    if (!state) {
      return;
    }
    composerMessages.typedReply();
    clearTimeout(lastKeyTimeout);
    lastKeyTimeout = setTimeout(function () {
      lastKeyTimeout = null;
      composerMessages.findSimilar();
    }, 1000);
  });

  // The composer's height (model.composerHeight): the default it opened
  // with, then what the grippie set; a minimized composer opens again at it.
  var composerHeight = null;

  // Composer#composeStateChanged: html.composer-open while open (the page
  // is padded by the composer's height so its end scrolls clear of it),
  // html.fullscreen-composer while fullscreen.
  function composeStateChanged() {
    composerTiming();
    root.classList.toggle("composer-open", control.classList.contains("open"));
    root.classList.toggle("fullscreen-composer", control.classList.contains("fullscreen"));
    syncPreviewClass();
    applyFullscreen();
    // ComposerMessages hidden while the composer is neither open nor
    // fullscreen (viewOpenOrFullscreen).
    composerMessages.setHidden(!control.classList.contains("open") && !control.classList.contains("fullscreen"));
  }

  // composer-container's htmlClass: html.composer-has-preview while the
  // composer shows (open, minimized or fullscreen) with its preview.
  function syncPreviewClass() {
    root.classList.toggle(
      "composer-has-preview",
      !control.classList.contains("closed") && control.classList.contains("show-preview")
    );
  }

  function applyFullscreen() {
    var on = control.classList.contains("fullscreen");
    var button = control.querySelector(".toggle-fullscreen");
    if (!button) {
      return;
    }
    button.title = on ? data.labelExitFullscreen : data.labelFullscreen;
    button.querySelector("use").setAttribute(
      "href",
      on ? "#discourse-compress" : "#discourse-expand"
    );
  }

  // The grippie, a DResizeSeparator on #reply-control's top edge: a drag
  // or the arrow keys resize the composer between its min-height and what
  // the viewport leaves below the header, --composer-height following
  // (onResizeDrag) and the size kept for next time once it ends
  // (onResizeEnd).
  var grippie = control.querySelector(".grippie");

  // lib/resize-measurements: a computed length in pixels, else null.
  function pixels(declared) {
    if (!declared || !/px$/.test(declared)) {
      return null;
    }
    var value = parseFloat(declared);
    return isFinite(value) ? value : null;
  }

  function measuredSize() {
    return control.isConnected ? pixels(getComputedStyle(control).height) : null;
  }

  function measuredMin() {
    if (!control.isConnected) {
      return 0;
    }
    var min = pixels(getComputedStyle(control).minHeight);
    return min === null ? 250 : min;
  }

  // The viewport less the header (viewportCap), or a smaller max-height;
  // never under the minimum.
  function measuredMax() {
    var cap = window.innerHeight - (parseInt(root.style.getPropertyValue("--header-offset"), 10) || 0);
    if (!control.isConnected) {
      return cap;
    }
    var declared = pixels(getComputedStyle(control).maxHeight);
    return Math.max(declared === null ? cap : Math.min(declared, cap), measuredMin());
  }

  // The minimum wins when the two conflict, as the stylesheet's does.
  function clampSize(size) {
    return Math.max(Math.min(size, measuredMax()), measuredMin());
  }

  // aria-valuenow, -min and -max as last measured (refresh).
  function refreshSeparator(size) {
    var now = size === undefined ? measuredSize() : size;
    var min = measuredMin();
    var max = measuredMax();
    grippie.setAttribute("aria-valuemin", String(min));
    grippie.setAttribute("aria-valuemax", String(max));
    if (now === null) {
      grippie.removeAttribute("aria-valuenow");
    } else {
      grippie.setAttribute("aria-valuenow", String(Math.max(Math.min(now, max), min)));
    }
  }

  // The appEvents a resize fires (composer:resize-started, div-resizing
  // and resize-ended), for what sits beside the composer (the chat
  // drawer), as events bubbling from #reply-control.
  var resizeStarted = false;
  function resizeEvent(name) {
    control.dispatchEvent(new CustomEvent("composer:" + name, { bubbles: true }));
  }

  function onResizeDrag(size) {
    control.classList.add("clear-transitions");
    composerHeight = size + "px";
    root.style.setProperty("--composer-height", size ? composerHeight : "");
    if (!resizeStarted) {
      resizeStarted = true;
      resizeEvent("resize-started");
    }
    resizeEvent("div-resizing");
  }

  function onResizeEnd(size) {
    control.classList.remove("clear-transitions");
    setItem("composerHeight", size + "px");
    refreshSeparator(size);
    if (resizeStarted) {
      resizeStarted = false;
      resizeEvent("resize-ended");
    }
  }

  // d-pointer-drag and d-resize-edge: the primary button's press, a size
  // reported once a frame from the latest position, and a press released
  // where it landed is a click, not a resize. body.d-resizing-ns holds the
  // cursor for the length of the gesture.
  var pointerDrag = null;
  grippie.addEventListener("pointerdown", function (event) {
    if (event.button !== 0 || pointerDrag) {
      return;
    }
    endKeyboardResize();
    var startValue = measuredSize();
    if (startValue === null) {
      return;
    }
    event.preventDefault();
    pointerDrag = {
      id: event.pointerId,
      startY: event.clientY,
      pendingY: event.clientY,
      startValue: startValue,
      started: false,
      frame: null,
    };
    try {
      grippie.setPointerCapture(event.pointerId);
    } catch (_) {
      // A pointer the browser no longer tracks: the document listeners
      // below still see it.
    }
    document.body.classList.add("d-resizing-ns");
  });

  // The handle sits on the top edge, so moving up grows the composer.
  function reportMove(drag, y, final) {
    var size = clampSize(drag.startValue - (y - drag.startY));
    drag.started = true;
    onResizeDrag(size);
    if (final) {
      onResizeEnd(size);
    }
  }

  grippie.addEventListener("pointermove", function (event) {
    var drag = pointerDrag;
    if (!drag || event.pointerId !== drag.id) {
      return;
    }
    drag.pendingY = event.clientY;
    if (drag.frame === null) {
      drag.frame = requestAnimationFrame(function () {
        drag.frame = null;
        if (pointerDrag === drag) {
          reportMove(drag, drag.pendingY, false);
        }
      });
    }
  });

  function endPointerDrag(event) {
    var drag = pointerDrag;
    if (!drag || event.pointerId !== drag.id) {
      return;
    }
    pointerDrag = null;
    if (drag.frame !== null) {
      cancelAnimationFrame(drag.frame);
    }
    document.body.classList.remove("d-resizing-ns");
    // A cancelled pointer's coordinates are not where it last was.
    var cancelled = event.type === "pointercancel" || event.type === "lostpointercapture";
    var y = cancelled ? drag.pendingY : event.clientY;
    if (!drag.started && y === drag.startY) {
      return;
    }
    reportMove(drag, y, true);
  }
  grippie.addEventListener("pointerup", endPointerDrag);
  grippie.addEventListener("pointercancel", endPointerDrag);
  grippie.addEventListener("lostpointercapture", endPointerDrag);

  // The keyboard: ArrowUp grows and ArrowDown shrinks by 16px, Home and End
  // go to the bounds; the size is kept when the key comes up or focus goes.
  var keyboardResize = null;
  function endKeyboardResize() {
    if (!keyboardResize) {
      return;
    }
    var size = keyboardResize.value;
    keyboardResize = null;
    onResizeEnd(size);
  }

  grippie.addEventListener("keydown", function (event) {
    if (pointerDrag || event.ctrlKey || event.metaKey || event.altKey || event.shiftKey) {
      return;
    }
    if (["ArrowUp", "ArrowDown", "Home", "End"].indexOf(event.key) === -1) {
      return;
    }
    if (!keyboardResize) {
      var startValue = measuredSize();
      if (startValue === null) {
        return;
      }
      keyboardResize = { key: event.key, value: startValue };
    }
    event.preventDefault();
    var next;
    if (event.key === "ArrowUp") {
      next = keyboardResize.value + 16;
    } else if (event.key === "ArrowDown") {
      next = keyboardResize.value - 16;
    } else {
      next = event.key === "Home" ? measuredMin() : measuredMax();
    }
    keyboardResize.value = clampSize(next);
    onResizeDrag(keyboardResize.value);
    refreshSeparator(keyboardResize.value);
  });
  grippie.addEventListener("keyup", function (event) {
    if (keyboardResize && event.key === keyboardResize.key) {
      endKeyboardResize();
    }
  });
  grippie.addEventListener("blur", endKeyboardResize);
  window.addEventListener("blur", endKeyboardResize);
  window.addEventListener("pagehide", endKeyboardResize);

  // refresh(): on the viewport changing, and as the composer settles into
  // a state.
  window.addEventListener("resize", function () {
    refreshSeparator();
  });
  if (window.ResizeObserver) {
    new ResizeObserver(function () {
      refreshSeparator();
    }).observe(control);
  }
  refreshSeparator();

  // The toolbar (editor-toolbar.js) over the open editor's
  // TextManipulation: the rich editor's, else the textarea's.
  var textareaManipulation = null;
  function textManipulation() {
    if (richEditor) {
      return richEditor.textManipulation;
    }
    return textareaTextManipulation();
  }

  // The textarea's, which cooks with the preview's engine to tell its
  // autocompletes the caret is in code.
  function textareaTextManipulation() {
    if (!textareaManipulation) {
      textareaManipulation = Discourse.editorToolbar.textarea(textarea, {
        i18n: richI18n,
        siteSettings: richContextSettings(),
        cook: function (raw) {
          return loadRenderer().then(function (render) {
            return render(raw);
          });
        },
      });
    }
    return textareaManipulation;
  }
  // d-editor's showLinkModal: UpsertHyperlink (upsert-hyperlink.js) over
  // the toolbar event, the focus back in `host`'s editor once it closes.
  function showLinkModal(toolbarEvent, host) {
    var sel = toolbarEvent.selected;
    showUpsertHyperlink({ hasSelection: !!sel && sel.start !== sel.end, toolbarEvent: toolbarEvent }, host);
  }

  function showUpsertHyperlink(model, host) {
    Discourse.upsertHyperlink.show(model, {
      i18n: richI18n,
      basePath: base,
      onClose: function () {
        var input = host.querySelector(".d-editor-input:not([hidden])");
        if (input) {
          input.focus();
        }
      },
    });
  }

  var composerBar = Discourse.editorToolbar.bind(control, {
    textManipulation: textManipulation,
    i18n: richI18n,
    listOptions: Discourse.editorToolbar.pluginListOptions(richContext().plugins || []),
    actions: {
      link: function (e) {
        showLinkModal(e, control);
      },
      upload: function () {
        fileInput.click();
      },
      emoji: function () {
        showEmojiPicker(control, textManipulation());
      },
    },
  });

  // enable-emoji.js' button: the emoji picker (emoji-picker.js, loaded the
  // first time) by the button, the emoji picked going in at the selection
  // (emojiSelected).
  function showEmojiPicker(host, tm) {
    var trigger = host.querySelector(".toolbar__button.emoji");
    if (!trigger) {
      return;
    }
    loadEmojiPicker().then(
      function () {
        Discourse.emojiPicker.open(trigger, {
          context: "topic",
          onSelect: function (code) {
            tm.emojiSelected(code);
          },
        });
      },
      function () {
        showError("Could not load the emoji picker.");
      }
    );
  }

  // emoji-picker.js, from the emoji button's script, once.
  var emojiPickerLoading = null;
  function loadEmojiPicker() {
    if (Discourse.emojiPicker) {
      return Promise.resolve();
    }
    var button = control.querySelector(".toolbar__button.emoji[data-src]");
    if (!button) {
      return Promise.reject(new Error("no emoji picker"));
    }
    if (!emojiPickerLoading) {
      emojiPickerLoading = new Promise(function (resolve, reject) {
        var script = document.createElement("script");
        script.src = button.dataset.src;
        script.onload = resolve;
        script.onerror = function () {
          emojiPickerLoading = null;
          reject(new Error("Could not load the emoji picker."));
        };
        document.head.appendChild(script);
      });
    }
    return emojiPickerLoading;
  }

  // d-editor's setupEditor: the emoji, hashtag and mention autocompletes
  // (_applyEmojiAutocomplete, _applyHashtagAutocomplete,
  // _applyMentionAutocomplete) on an editor's TextManipulation, the
  // textarea's once and the rich editor's each time it opens.
  var DEFAULT_EMOJIS = ["slight_smile", "smile", "wink", "sunny", "blush"];

  function applyAutocompletes(tm) {
    if (!Discourse.autocomplete) {
      return;
    }
    applyEmojiAutocomplete(tm);
    applyHashtagAutocomplete(tm);
    applyMentionAutocomplete(tm);
  }

  // afterComplete: schedule("afterRender", tm, tm.blurAndFocus)
  function refocusAfterComplete(tm) {
    return function () {
      setTimeout(function () {
        tm.blurAndFocus();
      }, 0);
    };
  }

  function notInCode(_, context) {
    return Promise.resolve(context.inCodeBlock()).then(function (inCode) {
      return !inCode;
    });
  }

  function applyHashtagAutocomplete(tm) {
    var site = richContext().site || {};
    var order = (site.hashtag_configurations || {})["topic-composer"] || [];
    var options = Discourse.autocomplete.hashtagOptions(order, { afterComplete: refocusAfterComplete(tm) });
    var search = options.dataSource;
    // The results' :emoji: are drawn with the picker's data.
    options.dataSource = function (term) {
      var always = function () {
        return search(term);
      };
      return loadEmojiPicker().then(always, always);
    };
    tm.autocomplete(options);
  }

  function applyEmojiAutocomplete(tm) {
    var settings = richContextSettings();
    if (!settings.enable_emoji) {
      return;
    }
    var SKIP = Discourse.autocomplete.SKIP;
    var trigger = new RegExp(
      "(?:^|" + Discourse.autocomplete.EMOJI_ALLOWED_PRECEDING.source + ")(:(?!:).?[\\w-]*:?(?!:)(?:t\\d?)?:?)$",
      "gi"
    );
    tm.autocomplete({
      key: ":",
      render: Discourse.autocomplete.renderEmojis,
      afterComplete: refocusAfterComplete(tm),
      onKeyUp: function (text, cp) {
        trigger.lastIndex = 0;
        var matches = trigger.exec(text.substring(0, cp));
        if (matches && matches[1]) {
          return [matches[1]];
        }
      },
      transformComplete: function (v) {
        if (v.code) {
          Discourse.emojiStore.track(v.code, "topic");
          return v.code + ":";
        }
        showEmojiPickerAtCaret(tm, v.term);
        return "";
      },
      dataSource: function (term) {
        var full = ":" + term;
        term = term.toLowerCase();
        var denied = (richContext().site || {}).denied_emojis || [];
        return loadEmojiPicker()
          .then(function () {
            if (term.length < Number(settings.emoji_autocomplete_min_chars || 0)) {
              return SKIP;
            }
            if (term === "") {
              var favorites = Discourse.emojiStore.favorites("topic");
              if (favorites.length) {
                return favorites
                  .filter(function (f) {
                    return denied.indexOf(f) === -1;
                  })
                  .slice(0, 5);
              }
              return DEFAULT_EMOJIS;
            }
            // note this will only work for emojis starting with :
            // eg: :-)
            return Discourse.emojiPicker.translations().then(function (translations) {
              if (translations[full]) {
                return [translations[full]];
              }
              var match = term.match(/^:?(.*?):t([2-6])?$/);
              var tonable = match ? Discourse.emojiPicker.isSkinTonable(match[1]) : Promise.resolve(false);
              return tonable.then(function (yes) {
                if (yes && denied.indexOf(match[1]) === -1) {
                  return match[2]
                    ? [match[1] + ":t" + match[2]]
                    : [2, 3, 4, 5, 6].map(function (x) {
                        return match[1] + ":t" + x;
                      });
                }
                return Discourse.emojiPicker.search(term, {
                  maxResults: 5,
                  diversity: Discourse.emojiStore.diversity(),
                  exclude: denied,
                });
              });
            });
          })
          .then(function (list) {
            if (list === SKIP) {
              return [];
            }
            return list.map(function (code) {
              return { code: code, src: Discourse.emojiPicker.url(code) };
            });
          })
          .then(function (list) {
            if (list.length) {
              list.push({ label: richI18n("composer.more_emoji"), term: term });
            }
            return list;
          });
      },
      triggerRule: notInCode,
    });
  }

  // The "more" item's picker (EmojiPickerDetached), by the caret: 18px
  // right of and 10px below where the term is, its pick put in the term's
  // place (emojiSelected).
  var caretAnchor = null;
  function showEmojiPickerAtCaret(tm, term) {
    var handler = tm.autocompleteHandler;
    var coords = handler.getCaretCoords(handler.getCaretPosition());
    var rect = (tm.textarea || tm.view.dom).getBoundingClientRect();
    if (caretAnchor) {
      caretAnchor.remove();
    }
    caretAnchor = document.createElement("span");
    caretAnchor.className = "d-editor-caret-anchor";
    caretAnchor.style.cssText =
      "position: fixed; width: 0; height: 0; left: " +
      (rect.left + coords.left + 18) +
      "px; top: " +
      (rect.top + coords.top + 10) +
      "px;";
    control.appendChild(caretAnchor);
    Discourse.emojiPicker.open(caretAnchor, {
      context: "topic",
      term: term,
      onSelect: function (code) {
        tm.emojiSelected(code);
      },
    });
  }

  // The composer's topic, category and the user replied to (d-editor's
  // topicId, categoryId and replyingToUserId), once its topic and post
  // have loaded.
  function mentionContext() {
    var s = state;
    if (!s) {
      return Promise.resolve({});
    }
    return Promise.all([s.actionContext, s.editPostLoad])
      .catch(function () {})
      .then(function () {
        var topic = s.actionTopic || s.topic || null;
        var categoryId = null;
        if (s.mode === "create-topic") {
          var value = chooser.querySelector("summary").dataset.value;
          categoryId = value ? parseInt(value, 10) : null;
        } else if (topic) {
          categoryId = topic.categoryId || null;
        }
        // composer.replyingToUserId: an edit's post's reply_to_user, the
        // post replied to's author, else the topic's.
        var prioritized = null;
        if (s.mode === "edit" && s.editPost && s.editPost.reply_to_user) {
          prioritized = s.editPost.reply_to_user.id;
        } else if (s.mode === "reply" && s.actionPost && s.actionPost.userId) {
          prioritized = s.actionPost.userId;
        } else if (topic && topic.userId) {
          prioritized = topic.userId;
        }
        return {
          topicId: s.mode === "create-topic" || s.mode === "private-message" ? null : s.topicId || null,
          categoryId: categoryId,
          prioritizedUserId: prioritized,
        };
      });
  }

  function applyMentionAutocomplete(tm) {
    if (!richContextSettings().enable_mentions) {
      return;
    }
    var ac = Discourse.autocomplete;
    tm.autocomplete({
      key: "@",
      render: ac.renderUsers,
      dataSource: function (term) {
        return mentionContext()
          .then(function (context) {
            return ac.userSearch({
              term: term,
              searchContext: "mention",
              topicId: context.topicId,
              categoryId: context.categoryId,
              includeGroups: true,
              prioritizedUserId: context.prioritizedUserId,
            });
          })
          .then(ac.indexUserStatuses);
      },
      transformComplete: function (v) {
        ac.validateSearchResult(v, {
          usernameMissing: richI18n("composer.autocomplete.username_missing"),
          nameMissing: richI18n("composer.autocomplete.name_missing"),
        });
        return v.username || v.name;
      },
      afterComplete: refocusAfterComplete(tm),
      triggerRule: notInCode,
    });
  }

  // The preview: the markdown crate as WebAssembly, loaded the first time
  // the composer opens with the preview shown. Its exports are in
  // crates/markdown/src/wasm.rs.
  var preview = control.querySelector(".d-editor-preview");
  var previewToggle = control.querySelector(".toggle-preview");
  var renderer = null;
  var previewTimer = null;

  function loadRenderer() {
    if (!renderer) {
      renderer = Promise.all([
        WebAssembly.instantiateStreaming(fetch(data.previewWasm), {}),
        fetch(data.previewSettings, { credentials: "same-origin" }).then(function (r) {
          if (!r.ok) {
            throw new Error("settings " + r.status);
          }
          return r.text();
        }),
      ]).then(function (loaded) {
        var wasm = loaded[0].instance.exports;
        var call = function (fn, text) {
          var bytes = new TextEncoder().encode(text);
          var ptr = wasm.alloc(bytes.length);
          new Uint8Array(wasm.memory.buffer, ptr, bytes.length).set(bytes);
          var status = fn(ptr, bytes.length);
          wasm.dealloc(ptr, bytes.length);
          var out = new TextDecoder().decode(
            new Uint8Array(wasm.memory.buffer, wasm.output_ptr(), wasm.output_len())
          );
          if (status !== 0) {
            throw new Error(out);
          }
          return out;
        };
        call(wasm.configure, loaded[1]);
        var render = function (raw) {
          return call(wasm.preview, raw);
        };
        // The rich editor's parser: the same parse as a node tree
        // (docs/rich-editor.md).
        render.tree = function (raw) {
          return JSON.parse(call(wasm.rich_editor_tree, raw));
        };
        // The editor's engine (rich-editor/engine.js wasmEngine).
        render.wasm = wasm;
        render.call = call;
        return render;
      });
    }
    return renderer;
  }

  // isPreviewVisible: the stored choice, for those whose editor has a
  // preview (allowPreview: markdown mode, not the rich editor's).
  function previewShown() {
    return (
      data.allowPreview !== "false" &&
      (getItem("composer.showPreview") || "true") === "true"
    );
  }

  function updatePreview() {
    if (!previewToggle || !previewShown()) {
      return;
    }
    var raw = textarea.value;
    loadRenderer()
      .then(function (render) {
        // What the server cooks the renderer refuses; the preview says so
        // rather than showing something the post will not be.
        try {
          preview.innerHTML = render(raw);
          resolveShortUrls();
          // The topic composer's hashtags (hashtag_configurations'
          // topic-composer order), linked once looked up, then decorated.
          if (window.Discourse && Discourse.hashtags) {
            Discourse.hashtags.resolve(preview, ["category", "tag", "channel"], updatePreview);
            Discourse.hashtags.decorate(preview);
          }
          // Oneboxes: the cached ones now, the rest loaded after 450ms of
          // quiet (composer-editor's _loadOneboxes).
          if (window.Discourse && Discourse.oneboxes) {
            Discourse.oneboxes.applyCached(preview);
            clearTimeout(oneboxTimer);
            oneboxTimer = setTimeout(loadPreviewOneboxes, 450);
          }
          // The post decorators Ember runs on the preview too.
          if (window.Discourse && Discourse.highlightSyntax) {
            Discourse.highlightSyntax(preview);
          }
          if (window.Discourse && Discourse.applyLocalDates) {
            Discourse.applyLocalDates(preview, { title: true });
          }
          if (window.Discourse && Discourse.policyPreview) {
            Discourse.policyPreview(preview);
          }
          if (window.Discourse && Discourse.renderGraphviz) {
            Discourse.renderGraphviz(preview);
          }
          if (window.Discourse && Discourse.renderMath) {
            Discourse.renderMath(preview);
          }
          // previewUpdated: the mentions linked and warned of, and the
          // composer service's afterRefresh (composer-messages.js).
          composerMessages.previewUpdated(preview);
        } catch (e) {
          preview.textContent = e.message;
        }
      })
      .catch(function (e) {
        preview.textContent = "The preview could not load: " + e.message;
      });
  }

  // _loadOneboxes: an edited post's oneboxes are refreshed once.
  var oneboxTimer = null;
  function loadPreviewOneboxes() {
    if (!state) {
      return;
    }
    var category = state.mode === "create-topic" ? chooser.querySelector("summary").dataset.value : null;
    var refresh = state.mode === "edit" && !state.refreshedPost;
    var painted = Discourse.oneboxes.load(preview, {
      topicId: state.topicId || null,
      categoryId: category ? parseInt(category, 10) : null,
      maxOneboxes: parseInt(data.maxOneboxes, 10) || 50,
      refresh: refresh,
    });
    if (refresh && painted > 0) {
      state.refreshedPost = true;
    }
  }

  function schedulePreview() {
    clearTimeout(previewTimer);
    previewTimer = setTimeout(updatePreview, 30);
  }

  // The mobile view has no preview toggle, and shows no preview.
  function applyPreviewShown() {
    var shown = Boolean(previewToggle) && previewShown();
    control.classList.toggle("show-preview", shown);
    control.classList.toggle("hide-preview", !shown);
    syncPreviewClass();
    if (!previewToggle) {
      return;
    }
    previewToggle.hidden = data.allowPreview === "false";
    previewToggle.classList.toggle("active", !shown);
    previewToggle.title = shown ? data.labelHidePreview : data.labelShowPreview;
  }

  function togglePreview() {
    setItem("composer.showPreview", String(!previewShown()));
    applyPreviewShown();
    updatePreview();
  }

  applyPreviewShown();
  textarea.addEventListener("input", schedulePreview);
  textarea.addEventListener("input", announce);

  // The rich editor (d-editor's toggleRichEditor, docs/rich-editor.md):
  // ProseMirror in place of the textarea, opened by the toggle, Ctrl+M, or
  // the composer opening in composition mode 1 (currentUser.useRichEditor).
  // The textarea stays, hidden, and holds the markdown each edit
  // serializes, so drafts, presence and the save read it as before; where
  // this file writes the textarea itself, syncRichEditor follows. Each
  // switch saves the mode as the member's preference after a second of
  // quiet (#saveRichEditorPreference).
  var modeToggle = control.querySelector(".composer-toggle-switch");
  var editorContainer = control.querySelector(".d-editor-container");
  var richMode = data.compositionMode === "1";
  var richEditor = null;
  var richOpening = null;
  var preferenceTimer = null;
  var isApple = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent || "");
  var richEngine = null;

  // The appEvents the editor's extensions and this file exchange
  // (composer:upload-progress:<id>, composer:cancel-upload, ...).
  var richEvents = (function () {
    var handlers = {};
    return {
      on: function (name, fn) {
        (handlers[name] = handlers[name] || []).push(fn);
      },
      off: function (name, fn) {
        handlers[name] = (handlers[name] || []).filter(function (h) {
          return h !== fn;
        });
      },
      trigger: function (name) {
        var args = Array.prototype.slice.call(arguments, 1);
        (handlers[name] || []).slice().forEach(function (fn) {
          fn.apply(null, args);
        });
      },
    };
  })();

  // The rich editor's mention warnings (extensions/mention.js).
  richEvents.on("composer-messages:create", composerMessages.create);
  richEvents.on("composer-messages:close", composerMessages.closeTop);

  // composer-body's keyDown and the composer service's hitEsc: Esc closes
  // the top composer message first, the emoji picker's own aside; with
  // none, an open composer is minimized (shrink) whatever is written in
  // it, a fullscreen one goes back to open with the focus in its editor,
  // and a minimized one stays. The key is theirs who stop it in the
  // reference: a select-kit, and here whoever prevented its default, the
  // rich editor's keymap apart (its Escape, selectParentNode, lets the
  // key through to composer-body).
  control.addEventListener("keydown", function (event) {
    if (event.key !== "Escape" || !state || event.target.closest(".select-kit")) {
      return;
    }
    if (event.defaultPrevented && !event.target.classList.contains("ProseMirror")) {
      return;
    }
    if (document.querySelectorAll(".emoji-picker-modal.fadeIn").length === 1) {
      return;
    }
    event.preventDefault();
    if (composerMessages.count() > 0) {
      composerMessages.closeTop();
    } else if (control.classList.contains("open")) {
      minimize();
    } else if (control.classList.contains("fullscreen")) {
      toggleFullscreen();
      focusEditor();
    }
  });

  // d-editor's replaceToolbar: an extension's toolbar (the link toolbar
  // without room to float) in the button bar's place, with a back button,
  // until it gives the bar back (null, from the toolbar it put there).
  // A slot is a d-editor's: the composer's, or the profile's About me.
  function toolbarSlot(host) {
    return { host: host, replaced: null };
  }
  var composerToolbar = toolbarSlot(control);
  function replaceToolbar(slot, editorModule, toolbar, owner) {
    if (toolbar === null && owner && slot.replaced !== owner) {
      return;
    }
    slot.replaced = toolbar;
    var bar = slot.host.querySelector(".d-editor-button-bar");
    if (!bar) {
      return;
    }
    var holder = bar.querySelector(".d-editor-button-bar__replaced");
    if (holder) {
      holder.remove();
    }
    bar.classList.toggle("--replaced-toolbar", !!toolbar);
    Array.prototype.forEach.call(bar.children, function (child) {
      child.hidden = !!toolbar;
    });
    if (!toolbar) {
      return;
    }
    holder = document.createElement("div");
    holder.className = "d-editor-button-bar__replaced";
    holder.style.display = "contents";
    var back = document.createElement("button");
    back.type = "button";
    back.className = "btn no-text btn-icon d-editor-button-bar__back";
    back.innerHTML = svg("angle-left") + '<span aria-hidden="true">&#8203;</span>';
    back.addEventListener("mousedown", function (event) {
      event.preventDefault();
    });
    back.addEventListener("click", function () {
      replaceToolbar(slot, editorModule, null);
    });
    var buttons = document.createElement("div");
    buttons.style.display = "contents";
    editorModule.renderToolbarButtons(buttons, toolbar, { isFirst: false });
    holder.appendChild(back);
    holder.appendChild(buttons);
    bar.appendChild(holder);
  }

  // The editor's getContext (prosemirror-editor.gts): the site settings,
  // strings and site data its extensions read, from the server
  // (src/composer/rich_editor.rs), and the composer's own state, or for
  // another d-editor (`slot`, not the composer's) no topic.
  var richContextData = null;
  function richContext() {
    if (!richContextData) {
      richContextData = JSON.parse(data.richEditorContext || "{}");
    }
    return richContextData;
  }

  function richContextSettings() {
    return richContext().siteSettings || {};
  }

  // The context's strings, a plural key's form picked by count.
  function richI18n(key, params) {
    var labels = richContext().labels || {};
    if (params && params.count !== undefined) {
      var count = Number(params.count);
      var form = count === 0 && key + ".zero" in labels ? "zero" : count === 1 ? "one" : "other";
      if (key + "." + form in labels) {
        key = key + "." + form;
      }
    }
    var text = key in labels ? labels[key] : key;
    Object.keys(params || {}).forEach(function (name) {
      text = text.split("%{" + name + "}").join(String(params[name]));
    });
    return text;
  }

  function richEditorContext(editorModule, slot) {
    slot = slot || composerToolbar;
    var context = {
      siteSettings: richContextSettings(),
      site: richContext().site || {},
      currentUser: richContext().currentUser || null,
      i18n: richI18n,
      topicId: state && slot === composerToolbar ? state.topicId || null : null,
      // The category a new topic is in, for the oneboxes the editor loads.
      get categoryId() {
        if (!state || slot !== composerToolbar || state.mode !== "create-topic") {
          return null;
        }
        var value = chooser.querySelector("summary").dataset.value;
        return value ? parseInt(value, 10) : null;
      },
      appEvents: richEvents,
      capabilities: {
        viewport: { sm: window.matchMedia("(min-width: 40rem)").matches },
      },
      markdownOptions: {},
      replaceToolbar: function (toolbar, owner) {
        replaceToolbar(slot, editorModule, toolbar, owner);
      },
      // The modal service, for the node views' modals: core's wrap
      // attributes, and the plugins' builders where their scripts are in.
      modal: {
        show: function (name, opts) {
          var model = (opts && opts.model) || {};
          if (name === "upsert-hyperlink") {
            showUpsertHyperlink(model, slot.host);
          } else if (name === "wrap-attributes") {
            editorModule.showWrapAttributesModal(model, context.i18n);
          } else if (name === "poll-ui-builder") {
            showPollBuilder(model);
          } else if (name === "math-edit" && Discourse.mathEdit) {
            Discourse.mathEdit.show(model);
          } else if (name === "policy-builder" && Discourse.policyBuilder) {
            Discourse.policyBuilder.edit(model);
          } else if (name === "local-dates-create" && Discourse.localDatesCreate) {
            Discourse.localDatesCreate.show(model.insertDate, model.initialValues);
          }
        },
      },
    };
    return context;
  }

  // The plugins' extensions (rich-editor/plugins.js), and what their nodes
  // draw with before the editor opens: local dates' moment.
  function richEditorPlugins(editorModule) {
    var plugins = richContext().plugins || [];
    editorModule.registerPluginExtensions(plugins);
    if (plugins.indexOf("local-dates") !== -1 && Discourse.localDates) {
      // Without it the dates show as written.
      return Discourse.localDates.loadMoment().catch(function (e) {
        console.error(e);
      });
    }
    return Promise.resolve();
  }

  // capabilities.hasKeyboard
  function hasKeyboard() {
    return Boolean(window.matchMedia && window.matchMedia("(pointer: fine)").matches);
  }

  // ComposerToggleSwitch's label: formatShortcut("ctrl+m"), a literal
  // Control key everywhere (⌃ on Apple's platforms).
  function drawModeToggle() {
    if (modeToggle) {
      drawToggle(modeToggle, richMode);
    }
  }

  function drawToggle(modeToggle, richMode) {
    var key = richMode ? "labelSwitchToMarkdown" : "labelSwitchToRichText";
    var keyboard = hasKeyboard();
    var label = keyboard
      ? data[key].replace("%{keyboardShortcut}", (isApple ? "⌃" : data.labelKeyCtrl) + " M")
      : data[key + "NoShortcut"];
    modeToggle.classList.toggle("--rte", richMode);
    modeToggle.classList.toggle("--markdown", !richMode);
    modeToggle.setAttribute("aria-checked", String(richMode));
    modeToggle.dataset.richEditor = String(richMode);
    modeToggle.title = label;
    modeToggle.setAttribute("aria-label", label);
    if (keyboard) {
      modeToggle.setAttribute("aria-keyshortcuts", (isApple ? data.labelKeyControl : data.labelKeyCtrl) + "+M");
    } else {
      modeToggle.removeAttribute("aria-keyshortcuts");
    }
    modeToggle.querySelector(".composer-toggle-switch__left-icon").classList.toggle("--active", !richMode);
    modeToggle.querySelector(".composer-toggle-switch__right-icon").classList.toggle("--active", richMode);
  }

  // editorContainerModeClass, and the composer's allowPreview: the rich
  // editor has no preview.
  function drawEditorMode() {
    editorContainer.classList.toggle("--rich-editor-enabled", richMode);
    editorContainer.classList.toggle("--markdown-editor-enabled", !richMode);
    textarea.hidden = Boolean(richEditor);
    data.allowPreview = String(!richMode);
    drawModeToggle();
    applyPreviewShown();
  }

  function saveCompositionMode() {
    saveMode(richMode);
  }

  // #saveRichEditorPreference, from whichever d-editor switched.
  function saveMode(rich) {
    var mode = rich ? "1" : "0";
    data.compositionMode = mode;
    clearTimeout(preferenceTimer);
    preferenceTimer = setTimeout(function () {
      var headers = csrfHeaders();
      headers.Accept = "application/json";
      headers["X-Requested-With"] = "XMLHttpRequest";
      fetch(base + "/u/" + encodeURIComponent(data.username) + ".json", {
        method: "PUT",
        credentials: "same-origin",
        headers: headers,
        body: new URLSearchParams({ composition_mode: mode }).toString(),
      })
        .then(function (r) {
          if (!r.ok) {
            throw new Error("composition_mode " + r.status);
          }
        })
        .catch(function (e) {
          console.error(e);
        });
    }, 1000);
  }

  function focusEditor(options) {
    if (richEditor) {
      richEditor.focus();
    } else {
      textarea.focus(options);
    }
  }

  // Back to the textarea, holding the markdown the editor last wrote.
  function closeRichEditor() {
    if (richEditor) {
      richEditor.textManipulation.autocomplete("destroy");
      richEditor.destroy();
      richEditor = null;
    }
    composerBar.refresh({});
    if (composerToolbar.replaced) {
      replaceToolbar(composerToolbar, null, null);
    }
    richOpening = null;
  }

  // A post the editor cannot hold (UnsupportedTokenError, or a construct
  // the markdown crate itself refuses): the reference's alert, and the
  // markdown editor again. Anything else is a bug, shown as it is.
  function richEditorFailed(e) {
    closeRichEditor();
    if (richMode) {
      richMode = false;
      saveCompositionMode();
    }
    drawEditorMode();
    updatePreview();
    alertRichFailure(e);
  }

  function alertRichFailure(e) {
    var unsupported =
      e && (e.name === "UnsupportedTokenError" || /^not ported yet/.test(e.message || ""));
    if (!unsupported) {
      console.error(e);
    }
    alertText(unsupported ? data.labelUnsupportedToken : (e && e.message) || String(e));
  }

  function openRichEditor() {
    if (richEditor || richOpening) {
      return;
    }
    var opening = Promise.resolve()
      .then(function () {
        return Promise.all([Discourse.richEditor.load(), loadRenderer()]);
      })
      .then(function (loaded) {
        return richEditorPlugins(loaded[0]).then(function () {
          return loaded;
        });
      })
      .then(function (loaded) {
        // Superseded: switched back to markdown meanwhile.
        if (richOpening !== opening) {
          return;
        }
        var refused = null;
        var editor = loaded[0].createEditor({
          container: control.querySelector(".d-editor-textarea-wrapper"),
          value: textarea.value,
          engine: (richEngine = richEngine || loaded[0].wasmEngine(loaded[1].wasm, loaded[1].call)),
          context: richEditorContext(loaded[0]),
          placeholder: textarea.placeholder,
          className: "d-editor-input",
          keymap: Object.assign({}, composerBar.keymap, popupKeymap(), {
            "ctrl+m": function () {
              toggleRichEditor();
            },
          }),
          onChange: function (markdown) {
            textarea.value = markdown;
            textarea.dispatchEvent(new Event("input", { bubbles: true }));
          },
          onStateChange: function (toolbarState) {
            if (richEditor === editor) {
              composerBar.refresh(toolbarState);
            }
          },
          // On opening, the refusal below; later, what a toolbar button
          // inserted.
          onUnsupported: function (e) {
            if (richEditor === editor) {
              setTimeout(function () {
                if (richEditor === editor) {
                  richEditorFailed(e);
                }
              }, 0);
            } else {
              refused = e;
            }
          },
          focusIn: function () {
            wrapper.classList.add("in-focus");
          },
          focusOut: function () {
            wrapper.classList.remove("in-focus");
          },
        });
        richEditor = editor;
        richOpening = null;
        if (refused) {
          richEditorFailed(refused);
          return;
        }
        applyAutocompletes(editor.textManipulation);
        composerBar.refresh(editor.textManipulation.state);
        // Ctrl or Cmd + Enter submits here too.
        editor.view.dom.addEventListener("keydown", function (event) {
          if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            save();
          }
        });
        drawEditorMode();
        if (state) {
          editor.focus();
        }
      });
    richOpening = opening;
    opening.catch(function (e) {
      if (richOpening === opening) {
        richOpening = null;
        richEditorFailed(e);
      }
    });
  }

  // The markdown this file wrote into the textarea, into the editor
  // (convertFromValue), refused as on opening.
  function syncRichEditor() {
    if (!richEditor) {
      return;
    }
    var refused = null;
    try {
      if (!richEditor.setValue(textarea.value)) {
        refused = { name: "UnsupportedTokenError" };
      }
    } catch (e) {
      refused = e;
    }
    if (refused) {
      richEditorFailed(refused);
    }
  }

  function toggleRichEditor() {
    richMode = !richMode;
    if (richMode) {
      openRichEditor();
      drawEditorMode();
    } else {
      closeRichEditor();
      drawEditorMode();
      updatePreview();
      if (state) {
        textarea.focus();
      }
    }
    saveCompositionMode();
  }

  if (modeToggle) {
    // preventFocus: the toggle leaves the focus in the editor.
    modeToggle.addEventListener("mousedown", function (event) {
      event.preventDefault();
    });
  }
  textarea.addEventListener("keydown", function (event) {
    if (
      event.ctrlKey &&
      !event.metaKey &&
      !event.altKey &&
      !event.shiftKey &&
      (event.key === "m" || event.key === "M") &&
      modeToggle
    ) {
      event.preventDefault();
      toggleRichEditor();
    }
  });
  drawModeToggle();
  applyAutocompletes(textareaTextManipulation());

  // Another d-editor with no forced mode (the profile's About me,
  // preferences.js) opens in and saves the same composition mode
  // (currentUser.user_option.composition_mode), with the composer's engine,
  // extensions and context. A closed composer follows a switch made there,
  // as Ember's d-editor opens anew in the saved mode.
  Discourse.composerRichEditor = {
    rich: function () {
      return data.compositionMode === "1";
    },
    drawToggle: drawToggle,
    saveMode: function (rich) {
      saveMode(rich);
      if (control.classList.contains("closed") && richMode !== rich) {
        richMode = rich;
        if (!rich) {
          closeRichEditor();
        }
        drawEditorMode();
      }
    },
    // createEditor over `textarea` in `host`'s textarea wrapper: resolves
    // to { editor, destroy }, or rejects with what refused the markdown.
    // `bar` is the host's toolbar (toolbar(), below): its keys and state.
    create: function (host, textarea, toggle, bar) {
      var slot = toolbarSlot(host);
      var hostWrapper = host.querySelector(".d-editor-textarea-wrapper");
      return Promise.all([Discourse.richEditor.load(), loadRenderer()]).then(function (loaded) {
        return richEditorPlugins(loaded[0]).then(function () {
          var refused = null;
          var editor = loaded[0].createEditor({
            container: hostWrapper,
            value: textarea.value,
            engine: (richEngine = richEngine || loaded[0].wasmEngine(loaded[1].wasm, loaded[1].call)),
            context: richEditorContext(loaded[0], slot),
            placeholder: textarea.placeholder,
            className: "d-editor-input",
            keymap: Object.assign({}, bar ? bar.keymap : {}, { "ctrl+m": toggle }),
            onChange: function (markdown) {
              textarea.value = markdown;
              textarea.dispatchEvent(new Event("input", { bubbles: true }));
            },
            onStateChange: function (toolbarState) {
              if (bar) {
                bar.refresh(toolbarState);
              }
            },
            onUnsupported: function (e) {
              refused = e;
            },
            focusIn: function () {
              hostWrapper.classList.add("in-focus");
            },
            focusOut: function () {
              hostWrapper.classList.remove("in-focus");
            },
          });
          var destroy = function () {
            editor.destroy();
            if (slot.replaced) {
              replaceToolbar(slot, null, null);
            }
            if (bar) {
              bar.refresh({});
            }
          };
          if (refused) {
            destroy();
            throw refused;
          }
          return { editor: editor, destroy: destroy };
        });
      });
    },
    failed: alertRichFailure,
    // The host's toolbar (editor-toolbar.js) over `textarea`, or the editor
    // `rich()` gives while it is open, with the composer's strings.
    toolbar: function (host, textarea, rich) {
      var plain = Discourse.editorToolbar.textarea(textarea, { i18n: richI18n, siteSettings: richContextSettings() });
      return Discourse.editorToolbar.bind(host, {
        textManipulation: function () {
          var editor = rich();
          return editor ? editor.textManipulation : plain;
        },
        i18n: richI18n,
        listOptions: Discourse.editorToolbar.pluginListOptions(richContext().plugins || []),
        actions: {
          link: function (e) {
            showLinkModal(e, host);
          },
        },
      });
    },
  };

  // The new-topic and new-message routes (routes/new-topic.js,
  // new-message.js) as composer.openNewTopic and openNewMessage take
  // their query params.
  // The dialog service: a page that has no dialog loads it (its labels
  // with it) the first time one is needed.
  var dialogLoading = null;
  function withDialog(fn, failed) {
    if (window.Discourse && Discourse.dialog) {
      fn();
      return;
    }
    if (!dialogLoading) {
      dialogLoading = new Promise(function (resolve, reject) {
        if (!document.getElementById("dialog-strings")) {
          var strings = document.createElement("script");
          strings.type = "application/json";
          strings.id = "dialog-strings";
          strings.textContent = data.dialogStrings || "{}";
          document.head.appendChild(strings);
        }
        var script = document.createElement("script");
        script.src = data.dialogSrc;
        script.onload = resolve;
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
    dialogLoading.then(fn, function () {
      dialogLoading = null;
      failed();
    });
  }

  function alertText(message) {
    withDialog(
      function () {
        Discourse.dialog.alert(message);
      },
      function () {
        window.alert(message);
      }
    );
  }

  function unported(what) {
    alertText(data.labelUnported + " " + what);
  }

  // composer.filterTags: staff keep the tags as given; anyone else gets
  // the ones /tags/list.json finds of them (visible, not staff-only).
  function filterTags(tags) {
    if (!tags || tagOpts.staff) {
      return Promise.resolve(tags);
    }
    var query = new URLSearchParams([["only_tags", tags]]);
    return fetch(base + "/tags/list.json?" + query, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (json) {
        return (json.list_tags || [])
          .filter(function (t) {
            return !t.staff;
          })
          .map(function (t) {
            return t.name;
          })
          .join(",");
      });
  }

  // lib/utilities escapeExpression
  function escapeExpression(s) {
    var map = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;", "`": "&#x60;", "=": "&#x3D;" };
    return String(s).replace(/[&<>"'`=]/g, function (c) {
      return map[c];
    });
  }

  function openNewTopic(params, category, sharedDraft) {
    // CREATE_SHARED_DRAFT, and a category the member can't create topics
    // in, which Ember's chooser shows read-only (readOnlyCategoryId).
    if (sharedDraft) {
      unported("creating a shared draft");
      return;
    }
    if (category && !chooser.querySelector('.category-row[data-value="' + category + '"]')) {
      unported("a category the composer shows read-only");
      return;
    }
    filterTags(params.tags)
      .catch(function () {
        return null;
      })
      .then(function (tags) {
        var opts = { mode: "create-topic", category: category || null, body: params.body };
        // The composer service: a title within max_topic_title_length, and
        // tags (escaped, the first max_tags_per_topic, each cut to
        // max_tag_length) when the member may tag topics.
        var max = parseInt(data.maxTitleLength, 10);
        if (params.title && params.title.length <= max) {
          opts.title = params.title;
        }
        if (tags && tagChooser) {
          opts.tags = escapeExpression(tags)
            .split(",")
            .slice(0, tagOpts.maximum)
            .map(function (tag) {
              return tag.substring(0, tagOpts.maxLength);
            });
        }
        open(opts);
      });
  }

  // The message composer's recipients: EmailGroupUserChooser
  // (email-group-user-chooser.js) over the composer's targetRecipients.
  if (userSelector && window.Discourse && Discourse.emailGroupUserChooser) {
    var recipientOpts = {};
    try {
      recipientOpts = JSON.parse(userSelector.querySelector("details").dataset.options || "{}");
    } catch (_) {}
    recipientOpts.onChange = function () {
      syncWarning();
      scheduleDraft();
    };
    recipients = Discourse.emailGroupUserChooser.attach(userSelector.querySelector("details"), recipientOpts);
  }

  function recipientsValue() {
    return recipients ? recipients.values() : [];
  }

  // targetRecipients as a string ("a,b") or a list.
  function setRecipients(list) {
    if (!recipients) {
      return;
    }
    if (typeof list === "string") {
      list = list.split(",");
    }
    recipients.set(list || []);
    syncWarning();
  }

  // showWarning: staff sending to exactly one user (no group).
  function syncWarning() {
    if (!warning) {
      return;
    }
    var show = recipientsValue().length === 1 && !recipients.hasGroups();
    warning.hidden = !show;
    userSelector.querySelector(".users-input").classList.toggle("can-warn", show);
  }

  // Composer#createPost for a message: its title, archetype and
  // target_recipients, the warning when staff check it, and its tags
  // where messages take them.
  function messagePairs(pairs) {
    pairs.push(["title", title.value]);
    if (warning && !warning.hidden && warning.querySelector("input").checked) {
      pairs.push(["is_warning", "true"]);
    }
    pairs.push(["archetype", "private_message"]);
    pairs.push(["target_recipients", recipientsValue().join(",")]);
    if (data.canTagPms) {
      serializedTags().forEach(function (t) {
        if (t.id) {
          pairs.push(["tags[][id]", t.id]);
        }
        pairs.push(["tags[][name]", t.name]);
      });
    }
  }

  // The application route's composePrivateMessage: to a user (or no one),
  // and from a user card opened on a post, that post's link with a
  // reference title.
  function composePrivateMessage(username, post) {
    open({
      mode: "private-message",
      recipients: username ? [username] : [],
      body: post ? location.protocol + "//" + location.host + post.url : undefined,
      title: post ? data.labelReferenceTitle.split("%{title}").join(post.topicTitle) : undefined,
    });
  }

  // composer.openNewMessage: the route's title within
  // max_topic_title_length, its body, and its tags (as a new topic takes
  // them) where messages take tags.
  function openMessageComposer(params, to) {
    filterTags(data.canTagPms && tagChooser ? params.tags : null)
      .catch(function () {
        return null;
      })
      .then(function (tags) {
        var opts = { mode: "private-message", recipients: to || "", body: params.body };
        var max = parseInt(data.maxTitleLength, 10);
        if (params.title && params.title.length <= max) {
          opts.title = params.title;
        }
        if (tags && data.canTagPms && tagChooser) {
          opts.tags = escapeExpression(tags)
            .split(",")
            .slice(0, tagOpts.maximum)
            .map(function (tag) {
              return tag.substring(0, tagOpts.maxLength);
            });
        }
        open(opts);
      });
  }

  // The new-message route: to a username, or to a group once
  // Group.messageable says it may be messaged, or to no one.
  function openNewMessage(params) {
    var groupname = params.groupname || params.group_name;
    if (params.username) {
      openMessageComposer(params, params.username);
      return;
    }
    if (!groupname) {
      openMessageComposer(params, "");
      return;
    }
    fetch(base + "/groups/" + encodeURIComponent(groupname) + "/messageable.json", {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(
        function (json) {
          if (json.messageable) {
            openMessageComposer(params, groupname);
          } else {
            alertText(data.labelCantSendPm.split("%{username}").join(groupname));
          }
        },
        function () {
          alertText(data.labelCreateMessageError);
        }
      );
  }

  // A page load of either route: the server drew the page the route goes
  // to and says where it went and what to open (src/routes/new_topic.rs).
  function routeArrived() {
    var el = document.getElementById("composer-open");
    if (!el || !control.isConnected) {
      return;
    }
    el.remove();
    var go;
    try {
      go = JSON.parse(el.textContent);
    } catch (_) {
      return;
    }
    history.replaceState(history.state, "", go.url);
    if (!go.open) {
      return;
    }
    if (go.route === "new-topic") {
      openNewTopic(go.params, go.category, go.sharedDraft);
    } else {
      openNewMessage(go.params);
    }
  }

  // route-recognizer's parseQueryString, as the router reads a link's
  // query: only the first two parts of a pair, "true" without `=`, the
  // last of a name. Null for an array param (`tags[]=`), which the
  // composer trips over: the server refuses those.
  function parseQueryString(query) {
    var params = {};
    if (!query) {
      return params;
    }
    var decode = function (part) {
      try {
        return decodeURIComponent(part.replace(/\+/g, "%20"));
      } catch (_) {
        return "";
      }
    };
    var pairs = query.split("&");
    for (var i = 0; i < pairs.length; i++) {
      var pair = pairs[i].split("=");
      var key = decode(pair[0]);
      if (pair.length === 1) {
        params[key] = "true";
      } else if (key.length > 2 && key.slice(-2) === "[]") {
        return null;
      } else {
        params[key] = pair[1] ? decode(pair[1]) : "";
      }
    }
    return params;
  }

  // A link to either route from another page: the route aborts that
  // transition and opens the composer where the member is, unless a
  // category is asked for, whose page the route goes to (the server's).
  document.addEventListener(
    "click",
    function (event) {
      if (!control.isConnected || event.defaultPrevented || event.button !== 0) {
        return;
      }
      if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) {
        return;
      }
      var link = event.target.closest && event.target.closest("a[href]");
      if (!link || (link.target && link.target !== "_self") || link.hasAttribute("download")) {
        return;
      }
      var url = new URL(link.href, location.href);
      if (url.origin !== location.origin) {
        return;
      }
      var params = parseQueryString(url.search.slice(1));
      if (!params) {
        return;
      }
      if (url.pathname === base + "/new-topic") {
        if (params.category || params.category_id) {
          return;
        }
        event.preventDefault();
        event.stopPropagation();
        openNewTopic(params, null, false);
      } else if (url.pathname === base + "/new-message") {
        event.preventDefault();
        event.stopPropagation();
        openNewMessage(params);
      }
    },
    true
  );

  // The category chooser.
  function selectCategory(id) {
    var row = chooser.querySelector('.category-row[data-value="' + id + '"]');
    var header = chooser.querySelector(".select-kit-selected-name");
    var summary = chooser.querySelector("summary");
    if (!row) {
      return;
    }
    // A subcategory's header shows its parent's badge before its own, as
    // the chooser's row does.
    var badges = row.querySelectorAll(".category-status > .badge-category__wrapper");
    header.innerHTML = "";
    var name = document.createElement("span");
    name.className = "name";
    badges.forEach(function (b) {
      var badge = b.cloneNode(true);
      var count = badge.querySelector(".topic-count");
      if (count) {
        count.remove();
      }
      name.appendChild(badge);
    });
    header.appendChild(name);
    header.dataset.value = id;
    header.dataset.name = row.dataset.name;
    summary.dataset.value = id;
    chooser.classList.add("has-selection");
    if (tagChooser) {
      tagChooser.setAttribute("category-id", id);
      tagHeader();
      validateTags(false);
    }
    chooser.querySelectorAll(".category-row").forEach(function (r) {
      r.classList.toggle("is-selected", r === row);
    });
    // discourse-post-voting's createAsPostVoting resets to the category's
    // default.
    if (state && state.mode === "create-topic") {
      state.createAsPostVoting = postVotingDefault(state);
      renderActions();
    }
  }

  // A topic's category that isn't among those the chooser lists (one
  // the editor can't post in) stays selected, shown with the topic
  // title's badge, as Ember's chooser shows its value.
  function keepCategory(id) {
    var summary = chooser.querySelector("summary");
    if (summary.dataset.value === String(id)) {
      return;
    }
    var header = chooser.querySelector(".select-kit-selected-name");
    var badge = document.querySelector("#topic-title .badge-category__wrapper");
    header.innerHTML = "";
    var name = document.createElement("span");
    name.className = "name";
    if (badge) {
      name.appendChild(badge.cloneNode(true));
    }
    header.appendChild(name);
    header.dataset.value = id;
    summary.dataset.value = id;
    chooser.classList.add("has-selection");
    chooser.querySelectorAll(".category-row").forEach(function (r) {
      r.classList.remove("is-selected");
    });
  }

  chooser.addEventListener("toggle", function () {
    chooser.classList.toggle("is-expanded", chooser.open);
    if (chooser.open) {
      chooser.querySelector(".filter-input").focus();
    }
  });
  chooser.querySelector(".filter-input").addEventListener("input", function (e) {
    var term = e.target.value.toLowerCase();
    chooser.querySelectorAll(".category-row").forEach(function (row) {
      row.hidden = term !== "" && row.dataset.name.toLowerCase().indexOf(term) < 0;
    });
  });
  chooser.addEventListener("click", function (e) {
    var row = e.target.closest(".category-row");
    if (row) {
      selectCategory(row.dataset.value);
      chooser.open = false;
    }
  });

  // MiniTagChooser (a multi select over /tags/filter/search) on a
  // `.mini-tag-chooser` drawn closed by composer_view (its options in
  // data-options); its body is drawn when it opens, as select-kit draws
  // it. The composer's tags and the review queue's ReviewableFieldTags
  // (static/js/review.js, Discourse.miniTagChooser) both use it. `hooks`:
  //   categoryId()   the categoryId option ("" or null for none)
  //   minimum()      the minimum option (modifyNoSelection's label)
  //   prioritizeRecent()  the prioritizeRecentTags option and the site's
  //                  prioritize_recently_used_tags
  //   changed(added) after the selection changes (`added`: the item just
  //                  selected, the search result as returned)
  //   opened()       after it opens and draws its body
  //   failed(error)  when the search fails
  // The selection is a list of {id, name, isNew}: a name alone is a tag
  // without an id.
  function miniTagChooser(el, hooks) {
    var selectedTags = [];
    var tagSearchTimer = null;
    var tagSearchSeq = 0;
    var tagRequired = null;
    var tagForbidden = false;
    var tagHighlighted = -1;
    var tagRows = [];
    var tagOpts = {};
    try {
      tagOpts = JSON.parse(el.dataset.options || "{}");
    } catch (_) {}

    function call(name) {
      var args = Array.prototype.slice.call(arguments, 1);
      return typeof hooks[name] === "function" ? hooks[name].apply(null, args) : undefined;
    }

    function tagLabel(key, vars) {
      var text = (tagOpts.labels || {})[key] || "";
      Object.keys(vars || {}).forEach(function (k) {
        text = text.split("%{" + k + "}").join(vars[k]);
      });
      return text;
    }

    function renderTag(name, count) {
      var html =
        '<a data-tag-name="' + escapeHtml(name.toLowerCase()) + '" class="discourse-tag ' +
        escapeHtml(tagOpts.style || "simple") + '">' + escapeHtml(name) + "</a>";
      if (count) {
        html += ' <span class="discourse-tag-count">x' + count + "</span>";
      }
      return html;
    }

    function tagHeader() {
      var summary = el.querySelector("summary");
      // modifyNoSelection: the minimum, when there is one.
      var minimum = call("minimum") || 0;
      var none =
        minimum > 0
          ? tagLabel(minimum === 1 ? "required_one" : "required_other", { count: minimum })
          : tagLabel("optional");
      var names = selectedTags.map(function (t) {
        return t.name;
      });
      var max = tagOpts.maximum;
      var caret = max && selectedTags.length >= max ? "" : svg("plus", "angle-icon");
      summary.querySelector(".select-kit-header-wrapper").innerHTML =
        '<span class="formatted-selection">' + escapeHtml(names.length ? names.join(", ") : none) + "</span>" + caret;
      summary.dataset.name = names.join(",");
      summary.dataset.value = selectedTags
        .map(function (t) {
          return t.id || t.name;
        })
        .join(",");
      el.classList.toggle("no-tags", selectedTags.length === 0);
      el.classList.toggle("has-selection", selectedTags.length > 0);
    }

    function tagFilter() {
      var input = el.querySelector(".filter-input");
      return input ? input.value : "";
    }

    // tagUtils.createContentFromInput
    function tagFromInput(input) {
      input = input
        .trim()
        .replace(/\s+/g, "-")
        .replace(/[\/\?#\[\]@!\$&'\(\)\*\+,;=%\\`^\s|\{\}"<>]+/g, "")
        .replace(/-{2,}/g, "-")
        .substring(0, tagOpts.maxLength || 20)
        .replace(/^\.+|\.+$/g, "");
      if (tagOpts.forceLowercase) {
        input = input.toLowerCase();
      }
      return input;
    }

    // tagUtils.validateCreate
    function canCreateTag(filter, results) {
      if (!tagOpts.allowAny || !filter.length) {
        return false;
      }
      if (tagOpts.maximum && selectedTags.length >= tagOpts.maximum) {
        return false;
      }
      var term = filter.replace(new RegExp(tagOpts.filterRegexp || "^$", "g"), "").trim().toLowerCase();
      if (tagForbidden) {
        return false;
      }
      var inCollection = results.some(function (r) {
        return (r.name || "").toLowerCase() === term;
      });
      var inSelection = selectedTags.some(function (t) {
        return t.name.toLowerCase() === term;
      });
      return !(inCollection || inSelection);
    }

    function drawTagBody(results, filter) {
      var body = el.querySelector(".select-kit-body");
      var input = body.querySelector(".filter-input");
      var focused = document.activeElement === input;
      var value = input ? input.value : "";
      var placeholder = tagRequired
        ? tagLabel(tagRequired.min_count === 1 ? "required_group_one" : "required_group_other", {
            count: tagRequired.min_count,
            name: tagRequired.name,
          })
        : tagLabel(tagOpts.allowAny ? "filter_placeholder_with_any" : "filter_placeholder");
      var html =
        '<div class="select-kit-filter multi-select-filter is-expanded"><input class="filter-input" autocapitalize="off" autocomplete="off" autocorrect="off" name="filter-input-search" placeholder="' +
        escapeHtml(placeholder) + '" spellcheck="false" tabindex="0" type="search"></div>';
      if (selectedTags.length) {
        html += '<div class="selected-content">';
        selectedTags.forEach(function (t, i) {
          html +=
            '<button aria-label="' + escapeHtml(tagLabel("delete_item", { name: t.name })) +
            '" class="btn btn-default selected-choice tag-choice " data-name="' + escapeHtml(t.name) +
            '" data-value="' + escapeHtml(String(t.id || t.name)) + '" data-index="' + i + '" type="button">' +
            svg("xmark") + '<span class="d-button-label">' + escapeHtml(t.name) + "</span></button>";
        });
        html += "</div>";
      }
      tagRows = [];
      var maxReached = tagOpts.maximum && selectedTags.length >= tagOpts.maximum;
      if (maxReached && filter) {
        html +=
          '<ul class="select-kit-errors-collection"><li class="select-kit-error">' +
          escapeHtml(tagLabel(tagOpts.maximum === 1 ? "max_one" : "max_other", { count: tagOpts.maximum })) +
          "</li></ul>";
      }
      if (canCreateTag(filter, results)) {
        var created = tagFromInput(filter);
        if (created) {
          tagRows.push({ create: true, name: created });
        }
      }
      results.forEach(function (r) {
        tagRows.push(r);
      });
      if (tagRows.length) {
        html += '<ul aria-live="polite" class="select-kit-collection" role="menu">';
        tagRows.forEach(function (r, i) {
          var highlighted = i === tagHighlighted ? " is-highlighted" : "";
          if (r.create) {
            html +=
              '<li aria-checked="false" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(r.name) +
              '" data-value="' + escapeHtml(r.name) + '" title="' + escapeHtml(r.name) + '" tabindex="0" class="select-kit-row' +
              highlighted + '"><span class="name">' + escapeHtml(tagLabel("create", { content: r.name })) + "</span></li>";
            return;
          }
          var disabled = r.disabled ? " disabled" : "";
          html +=
            '<li aria-checked="false" role="menuitemradio" data-index="' + i + '" data-name="' + escapeHtml(r.name) +
            '" data-value="' + escapeHtml(String(r.id)) + '" title="' + escapeHtml(r.title || r.name) + '" tabindex="0"' +
            (r.disabled ? ' aria-disabled="true"' : "") + ' class="select-kit-row tag-row' + highlighted + disabled + '">  ' +
            renderTag(r.name, r.count);
          if (r.disabled) {
            html += '<span class="disabled-reason">' + escapeHtml(r.title || "") + "</span>";
          } else if (r.target_tag) {
            html += '<span class="synonym-hint">' + escapeHtml(tagLabel("synonym_hint", { tag_name: r.target_tag.name })) + "</span>";
          }
          html += "</li>";
        });
        html += "</ul>";
      }
      if (filter) {
        if (!tagRows.length) {
          html += '<span class="no-content" role="alert">' + escapeHtml(tagLabel("no_content")) + "</span>";
        } else {
          html +=
            '<span class="results-count" role="alert">' +
            escapeHtml(tagLabel(tagRows.length === 1 ? "results_one" : "results_other", { count: tagRows.length })) +
            "</span>";
        }
      }
      body.innerHTML = html;
      var next = body.querySelector(".filter-input");
      next.value = value;
      if (focused || el.open) {
        next.focus();
      }
    }

    // MiniTagChooser#search and _transformJson: the results less the
    // selected tags, sorted by name when the site sorts tags so.
    function searchTags() {
      var filter = tagFilter();
      var seq = ++tagSearchSeq;
      if (tagOpts.maximum === 0) {
        drawTagBody([], filter);
        return;
      }
      var params = new URLSearchParams();
      params.append("q", filter || "");
      params.append("limit", tagOpts.limit);
      var category = call("categoryId");
      if (category) {
        params.append("categoryId", category);
      }
      selectedTags
        .filter(function (t) {
          return t.id;
        })
        .slice(0, 100)
        .forEach(function (t) {
          params.append("selected_tag_ids[]", t.id);
        });
      params.append("filterForInput", "true");
      var recent = call("prioritizeRecent") && !filter;
      if (recent) {
        params.append("prioritizeRecentTags", "true");
      }
      fetch(base + "/tags/filter/search?" + params.toString(), {
        credentials: "same-origin",
        headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
      })
        .then(function (r) {
          return r.json();
        })
        .then(function (json) {
          if (seq !== tagSearchSeq || !el.open) {
            return;
          }
          tagForbidden = Boolean(json.forbidden);
          tagRequired = json.required_tag_group || null;
          var results = json.results || [];
          if (!recent && tagOpts.sortAlphabetically) {
            var byName = function (a, b) {
              return a.name.localeCompare(b.name);
            };
            results = results
              .filter(function (r) {
                return !r.disabled;
              })
              .sort(byName)
              .concat(
                results
                  .filter(function (r) {
                    return r.disabled;
                  })
                  .sort(byName)
              );
          }
          var names = selectedTags.map(function (t) {
            return t.name;
          });
          results = results.filter(function (r) {
            return names.indexOf(r.name) < 0;
          });
          // select-kit highlights the first row once there is a filter.
          tagHighlighted = filter ? 0 : -1;
          drawTagBody(results, filter);
        })
        .catch(function (error) {
          call("failed", error);
        });
    }

    function changed(added) {
      tagHeader();
      call("changed", added);
      searchTags();
    }

    function chooseTagRow(index) {
      var row = tagRows[index];
      if (!row || row.disabled) {
        return;
      }
      if (tagOpts.maximum && selectedTags.length >= tagOpts.maximum) {
        return;
      }
      var added = row.create ? { name: row.name, isNew: true } : row;
      selectedTags.push(added);
      var input = el.querySelector(".filter-input");
      if (input) {
        input.value = "";
      }
      changed(added);
    }

    el.addEventListener("toggle", function () {
      el.classList.toggle("is-expanded", el.open);
      if (el.open) {
        tagHighlighted = -1;
        drawTagBody([], "");
        call("opened");
        searchTags();
      } else {
        el.querySelector(".select-kit-body").innerHTML = "";
      }
    });
    el.addEventListener("input", function (e) {
      if (!e.target.classList.contains("filter-input")) {
        return;
      }
      clearTimeout(tagSearchTimer);
      tagSearchTimer = setTimeout(searchTags, 200);
    });
    el.addEventListener("keydown", function (e) {
      if (!el.open || !e.target.classList.contains("filter-input")) {
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        el.open = false;
      } else if (e.key === "Enter") {
        e.preventDefault();
        chooseTagRow(tagHighlighted);
      } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (!tagRows.length) {
          return;
        }
        var from = tagHighlighted < 0 ? (e.key === "ArrowDown" ? -1 : 0) : tagHighlighted;
        tagHighlighted = (from + (e.key === "ArrowDown" ? 1 : tagRows.length - 1)) % tagRows.length;
        el.querySelectorAll(".select-kit-row").forEach(function (row, i) {
          row.classList.toggle("is-highlighted", i === tagHighlighted);
        });
      } else if (e.key === "Backspace" && tagFilter() === "" && selectedTags.length) {
        selectedTags.pop();
        changed(null);
      }
    });
    el.addEventListener("click", function (e) {
      var choice = e.target.closest(".selected-choice");
      if (choice) {
        e.preventDefault();
        selectedTags.splice(parseInt(choice.dataset.index, 10), 1);
        changed(null);
        return;
      }
      var row = e.target.closest(".select-kit-row");
      if (row) {
        e.preventDefault();
        chooseTagRow(parseInt(row.dataset.index, 10));
      }
    });
    el.addEventListener("mouseover", function (e) {
      var row = e.target.closest(".select-kit-row");
      if (row) {
        tagHighlighted = parseInt(row.dataset.index, 10);
        el.querySelectorAll(".select-kit-row").forEach(function (r) {
          r.classList.toggle("is-highlighted", r === row);
        });
      }
    });
    // A click elsewhere closes it, as select-kit's click outside does.
    document.addEventListener("click", function (e) {
      if (el.open && !el.contains(e.target)) {
        el.open = false;
      }
    });

    return {
      options: tagOpts,
      selected: function () {
        return selectedTags;
      },
      set: function (tags) {
        selectedTags = (tags || []).slice(0);
        tagHeader();
      },
      header: tagHeader,
    };
  }

  // The composer's tag chooser, as composer-container renders it for a
  // member who may tag topics: the category's minimum, recent tags first.
  var tagChooser = control.querySelector(".mini-tag-chooser");
  var tagsTip = control.querySelector(".tags-input .popup-tip");
  // The tags kept without a chooser (a message's, site.can_tag_pms).
  var unchosenTags = [];
  var tags = tagChooser
    ? miniTagChooser(tagChooser, {
        categoryId: function () {
          return chooser ? chooser.querySelector("summary").dataset.value : "";
        },
        minimum: minimumRequiredTags,
        prioritizeRecent: function () {
          return tagOpts.prioritizeRecent;
        },
        changed: function () {
          validateTags(false);
          scheduleDraft();
        },
        // select-kit's floating body: on screens up to 450px wide it
        // spans the viewport less 10px each side (applySmallScreenOffset
        // and applySmallScreenMaxWidth).
        opened: function () {
          var body = tagChooser.querySelector(".select-kit-body");
          if (window.innerWidth <= 450) {
            var x = tagChooser.getBoundingClientRect().x;
            body.style.left = 10 - x + "px";
            body.style.width = window.innerWidth - 20 + "px";
            body.style.minWidth = "0";
          } else {
            body.style.left = "";
            body.style.width = "";
            body.style.minWidth = "";
          }
        },
        failed: function () {
          showError("Could not search tags.");
        },
      })
    : null;
  var tagOpts = tags ? tags.options : {};

  function selectedTags() {
    return tags ? tags.selected() : unchosenTags;
  }

  function tagHeader() {
    tags.header();
  }

  // The category's minimumRequiredTags (its minimum, or its required
  // groups' counts when more).
  function minimumRequiredTags() {
    if (!chooser) {
      return 0;
    }
    var id = chooser.querySelector("summary").dataset.value;
    var row = id ? chooser.querySelector('.category-row[data-value="' + id + '"]') : null;
    return row ? parseInt(row.dataset.minimumRequiredTags || "0", 10) : 0;
  }

  // serializeTags
  function serializedTags() {
    return selectedTags().map(function (t) {
      return t.isNew || !t.id ? { name: t.name } : { id: t.id, name: t.name };
    });
  }

  function setTags(list) {
    var normalized = (list || []).map(function (t) {
      return typeof t === "string" ? { name: t, isNew: true } : { id: t.id, name: t.name, isNew: !t.id };
    });
    if (tags) {
      tags.set(normalized);
      validateTags(false);
    } else {
      unchosenTags = normalized;
    }
  }

  // composer.tagValidation: staff and members of sites that can't tag
  // aside, the category's minimumRequiredTags.
  function validateTags(show) {
    if (!tagsTip) {
      return true;
    }
    var min = minimumRequiredTags();
    var failed = !tagOpts.staff && state && state.mode === "create-topic" && min > selectedTags().length;
    if (failed && show) {
      var text = tagLabel(min === 1 ? "tags_missing_one" : "tags_missing_other", { count: min });
      tagsTip.className = "popup-tip bad";
      tagsTip.setAttribute("role", "alert");
      tagsTip.setAttribute("aria-label", text);
      tagsTip.innerHTML = escapeHtml(text) + " " + svg("circle-xmark");
    } else if (!failed) {
      tagsTip.className = "popup-tip good hide";
      tagsTip.removeAttribute("role");
      tagsTip.removeAttribute("aria-label");
      tagsTip.innerHTML = " " + svg("circle-xmark");
    }
    return !failed;
  }

  function tagLabel(key, vars) {
    var text = (tagOpts.labels || {})[key] || "";
    Object.keys(vars || {}).forEach(function (k) {
      text = text.split("%{" + k + "}").join(vars[k]);
    });
    return text;
  }

  // For the review queue's ReviewableFieldTags.
  Discourse.miniTagChooser = miniTagChooser;

  function form(pairs) {
    return pairs
      .map(function (p) {
        return encodeURIComponent(p[0]) + "=" + encodeURIComponent(p[1]);
      })
      .join("&");
  }

  // Composer#editPost: a first post's topic fields through Topic.update
  // (PUT /t, or only its tags through PUT /t/:id/tags without the right
  // to edit the topic), then the post through PUT /posts. A 409 marks an
  // edit conflict: saving again overwrites.
  function editPost(s) {
    var promise = Promise.resolve(null);
    if (s.topic && title.value) {
      var topic = s.topic;
      if (topic.canEdit) {
        // The tags only where tagging is on (the topic has none otherwise).
        var props = { title: title.value };
        if (tagChooser) {
          props.tags = serializedTags();
        }
        var category = chooser.querySelector("summary").dataset.value;
        // Topic.update: never a category for a message.
        if (!topic.privateMessage) {
          props.category_id = category ? parseInt(category, 10) : null;
        }
        if (!s.editConflict) {
          props.original_title = topic.title;
          if (tagChooser) {
            props.original_tags = topic.tags;
          }
        }
        var headers = csrfHeaders();
        headers["Content-Type"] = "application/json";
        promise = fetch(base + "/t/" + (topic.slug || "-") + "/" + topic.id, {
          method: "PUT",
          headers: headers,
          credentials: "same-origin",
          body: JSON.stringify(props),
        });
      } else if (topic.canEditTags) {
        // Topic#updateTags: the tags as objects ({id, name}), as Rack
        // reads `tags[][...]`; none sends no tags.
        var tagPairs = [];
        serializedTags().forEach(function (t) {
          if (t.id) {
            tagPairs.push(["tags[][id]", t.id]);
          }
          tagPairs.push(["tags[][name]", t.name]);
        });
        promise = fetch(base + "/t/" + topic.id + "/tags", {
          method: "PUT",
          headers: csrfHeaders(),
          credentials: "same-origin",
          body: form(tagPairs),
        });
      }
    }
    return promise.then(function (topicResponse) {
      if (topicResponse && !topicResponse.ok) {
        if (topicResponse.status === 409) {
          s.editConflict = true;
        }
        return topicResponse;
      }
      var pairs = [
        ["post[raw]", textarea.value],
        ["post[topic_id]", s.topicId],
      ];
      if (!s.editConflict && s.originalText !== null && s.originalText !== undefined) {
        pairs.push(["post[original_text]", s.originalText]);
      }
      // The reply target only when changed: a stale one would clobber
      // another editor's change.
      var stored = s.editPost && (s.editPost.reply_to_post_number == null ? null : s.editPost.reply_to_post_number);
      if (s.editPost && s.editReplyTo !== undefined && s.editReplyTo !== stored) {
        pairs.push(["post[reply_to_post_number]", s.editReplyTo === null ? "" : String(s.editReplyTo)]);
      }
      return fetch(base + "/posts/" + s.postId, {
        method: "PUT",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form(pairs),
      }).then(function (r) {
        if (r.status === 409) {
          s.editConflict = true;
        }
        return r;
      });
    });
  }

  // The composer service's save: a reply saved from another topic's page
  // asks which topic it goes to first (the reply-where dialog), unless
  // that topic is closed for a member. Shift with the button saves a
  // reply without going to it (saveAction's jump).
  function save(event) {
    if (!state) {
      return;
    }
    submitPost(!(event && event.shiftKey && state.mode === "reply") && !state.skipJumpOnSave, false);
  }

  function submitPost(jump, force) {
    if (!state) {
      return;
    }
    // cantSubmitPost on the tags: the validation's tip shows instead.
    if (state.mode === "create-topic" && !validateTags(true)) {
      return;
    }
    if (!force && state.mode === "reply" && askWhere(jump)) {
      return;
    }
    var request;
    var s = state;
    // Composer#createPost stages the reply in the stream it is read in,
    // with the preview's cooked: on this topic's page, when it is loaded
    // to the end. The mobile view draws no preview, so stages nothing. A
    // staged post is drawn before the save returns, so post:highlight
    // focuses it; here the live stream brings it.
    var topicRoute = window.Discourse && Discourse.topicRoute;
    var staged =
      state.mode === "reply" &&
      !!topicRoute &&
      String(topicRoute.topicId()) === String(state.topicId) &&
      topicRoute.atTopicEnd() &&
      !!previewToggle &&
      previewShown() &&
      preview.innerHTML.replace(/<span class="marker"><\/span>/g, "") !== "";
    if (state.mode === "edit") {
      request = editPost(state);
    } else {
      // The draft goes with the post (PostCreator's draft_key).
      var pairs = [
        ["raw", textarea.value],
        ["draft_key", state.draftKey],
      ];
      if (state.mode === "create-topic") {
        pairs.push(["title", title.value]);
        var category = chooser.querySelector("summary").dataset.value;
        if (category) {
          pairs.push(["category", category]);
        }
        // serializeTags: an existing tag by id and name, a new one by
        // name, as Rack reads `tags[][...]`.
        serializedTags().forEach(function (t) {
          if (t.id) {
            pairs.push(["tags[][id]", t.id]);
          }
          pairs.push(["tags[][name]", t.name]);
        });
      } else if (state.mode === "private-message") {
        messagePairs(pairs);
      } else {
        pairs.push(["topic_id", state.topicId]);
        if (state.replyTo) {
          pairs.push(["reply_to_post_number", state.replyTo]);
        }
        // _create_serializer's whisper and no_bump.
        if (state.whisper) {
          pairs.push(["whisper", "true"]);
        }
        if (state.noBump) {
          pairs.push(["no_bump", "true"]);
        }
      }
      if (state.mode === "create-topic") {
        // unlist_topic, and discourse-post-voting's serializeOnCreate
        // fields.
        if (state.unlistTopic) {
          pairs.push(["unlist_topic", "true"]);
        }
        if (actionsOpts.postVoting) {
          var only = postVotingOnly(state);
          pairs.push(["create_as_post_voting", String(!!state.createAsPostVoting || only)]);
          pairs.push(["only_post_voting_in_this_category", String(only)]);
        }
      }
      // _create_serializer's typingTime and composerTime.
      pairs.push(["typing_duration_msecs", state.typingTime || 0]);
      pairs.push(["composer_open_duration_msecs", composerTime(state)]);
      request = fetch(base + "/posts", {
        method: "POST",
        headers: csrfHeaders(),
        credentials: "same-origin",
        body: form(pairs),
      });
    }
    // A pending draft save would outlive the post.
    clearTimeout(draftTimer);
    draftTimer = null;
    submit.disabled = true;
    request
      .then(function (r) {
        return r.text().then(function (body) {
          var json = null;
          try {
            json = JSON.parse(body);
          } catch (e) {
            // Not JSON: an error page.
          }
          return { ok: r.ok, status: r.status, json: json };
        });
      })
      .then(function (res) {
        submit.disabled = false;
        if (!res.ok || (res.json && res.json.errors)) {
          showError(
            res.json && res.json.errors
              ? res.json.errors.join(" ")
              : "Could not save (" + res.status + ")."
          );
          return;
        }
        // composer:saved
        composerMessages.saved();
        var json = res.json || {};
        // Sent for review: PostEnqueued's modal, the draft gone, and no
        // post to go to.
        if (json.action === "enqueued") {
          destroyDraft(s);
          close();
          postEnqueued(json);
          return;
        }
        var mode = s.mode;
        close();
        if (s.onSaved) {
          s.onSaved();
        }
        var post = json.post || json;
        if (!post.topic_id) {
          return;
        }
        if (!jump) {
          return;
        }
        // DiscourseURL.routeTo(post.url, { keepFilter, skipIfOnScreen }),
        // whether staged or not: the service reads stagedPost before
        // Composer#save (after the async beforeSave) has staged anything.
        var url = base + "/t/" + (post.topic_slug || "topic") + "/" + post.topic_id + (post.post_number > 1 ? "/" + post.post_number : "");
        if (topicRoute) {
          topicRoute.routeToPost(url, {
            highlight: mode === "edit" || staged,
            arriving: mode === "reply" && String(topicRoute.topicId()) === String(post.topic_id) && topicRoute.atTopicEnd(),
          });
        } else {
          go(url);
        }
      })
      .catch(function () {
        submit.disabled = false;
        showError("Something went wrong.");
      });
  }

  // The page for a URL, swapped in as a link's (hx-boost).
  function go(url) {
    if (!window.htmx) {
      window.location.assign(url);
      return;
    }
    var link = document.createElement("a");
    link.href = url;
    link.hidden = true;
    document.getElementById("main-outlet").appendChild(link);
    window.htmx.process(link);
    link.click();
  }

  // The reply-where dialog (TopicReplyChoiceDialog) when the reply's topic
  // isn't the one on the page: reply there, or here, to the page's topic
  // and no post; cancelling saves the draft. False when there is nothing
  // to ask: off a topic page, on the reply's own topic, or on a topic
  // closed to a member.
  function askWhere(jump) {
    var here = document.querySelector("#topic template.reply-where-topic");
    if (!here || String(here.dataset.topicId) === String(state.topicId) || here.dataset.offer !== "true") {
      return false;
    }
    var s = state;
    if (!s.whereHtml) {
      showError("Not supported yet: replying from another topic's page to a topic not opened on its own.");
      return true;
    }
    withDialog(
      function () {
        var holder = Discourse.dialog.dialog({
          title: data.labelPostingNotOnTopic,
          class: "reply-where-modal",
          bodyHtml:
            '<button class="btn no-text btn-primary btn-reply-where btn-reply-on-original" type="button">' +
            s.whereHtml +
            '</button><button class="btn no-text btn-reply-where btn-reply-here btn-default" type="button">' +
            here.innerHTML +
            "</button>",
          buttons: [{ label: data.labelCancel, class: "btn-default btn-text btn-reply-where__cancel", action: "cancel" }],
          didCancel: function () {
            if (state === s) {
              saveDraft();
            }
          },
        });
        holder.querySelector(".btn-reply-on-original").addEventListener("click", function () {
          Discourse.dialog.close();
          submitPost(jump, true);
        });
        holder.querySelector(".btn-reply-here").addEventListener("click", function () {
          Discourse.dialog.close();
          s.topicId = here.dataset.topicId;
          s.replyTo = null;
          s.whereHtml = here.innerHTML;
          submitPost(jump, true);
        });
      },
      function () {
        showError("Something went wrong.");
      }
    );
    return true;
  }

  // PostEnqueuedModal: the review notice, with the member's pending count.
  function postEnqueued(json) {
    var approval = {};
    try {
      approval = JSON.parse(data.approval);
    } catch (_) {}
    if (!window.Discourse || !Discourse.modal) {
      return;
    }
    var count = json.pending_count || 0;
    var pending = (count === 1 ? approval.pending_one : approval.pending_other) || "";
    Discourse.modal.show({
      className: "post-enqueued-modal",
      title: approval.title,
      body: function (body) {
        var p = document.createElement("p");
        p.textContent = approval.description || "";
        body.appendChild(p);
        var counted = document.createElement("p");
        counted.innerHTML = pending.replace("%{count}", String(count));
        body.appendChild(counted);
      },
      footer: function (footer, close) {
        var ok = document.createElement("button");
        ok.className = "btn btn-primary";
        ok.type = "button";
        ok.innerHTML = '<span class="d-button-label"></span>';
        ok.querySelector(".d-button-label").textContent = approval.ok || "OK";
        ok.addEventListener("click", close);
        footer.appendChild(ok);
      },
    });
  }

  // The composer actions menu: ComposerActions, a DMenu on the composer's
  // title whose items are ComposerActionItemBuilder's for the action
  // (reply, create topic, message), its toggles (whisper, do not bump,
  // unlisted) and discourse-post-voting's toggle; the reply target link
  // and the whisper indicator beside it. What the items do is
  // ComposerActionStateService's: the topic and post the composer was
  // opened on are kept (its snapshot) across the switches.
  var actionsOpts = {};
  try {
    actionsOpts = JSON.parse(actionButton.dataset.composerActions || "{}");
  } catch (_) {}
  var actionStrings = actionsOpts.strings || {};
  var actionsMenu = null;
  var WHISPER = 4;

  function at(key, vars) {
    var s = actionStrings[key] || key;
    Object.keys(vars || {}).forEach(function (k) {
      s = s.split("%{" + k + "}").join(vars[k]);
    });
    return s;
  }

  function mobileView() {
    return root.classList.contains("mobile-view");
  }

  // The topic the composer replies in, as the menu reads it (the topic
  // model in Ember).
  function loadActionTopic(topicId) {
    return fetchJson("/t/" + topicId + ".json")
      .then(function (t) {
        var details = t.details || {};
        return {
          id: t.id,
          title: t.title,
          url: base + "/t/" + (t.slug || "topic") + "/" + t.id,
          userId: t.user_id,
          categoryId: t.category_id,
          privateMessage: t.archetype === "private_message",
          postVoting: !!t.is_post_voting,
          pmWithNonHumanUser: !!t.pm_with_non_human_user,
          slowMode: (t.slow_mode_seconds || 0) > 0,
          postsCount: t.posts_count,
          allowedUsers: (details.allowed_users || []).map(function (u) {
            return u.username;
          }),
          allowedGroups: (details.allowed_groups || []).map(function (g) {
            return g.name;
          }),
        };
      })
      .catch(function () {
        return null;
      });
  }

  // The post replied to.
  function loadActionPost(topicId, postNumber) {
    return fetchJson("/posts/by_number/" + topicId + "/" + postNumber)
      .then(function (p) {
        return {
          id: p.id,
          postNumber: p.post_number,
          userId: p.user_id,
          username: p.username,
          name: p.name,
          postType: p.post_type,
          createdAt: p.created_at,
        };
      })
      .catch(function () {
        return null;
      });
  }

  // Post#url (postUrl): the topic's, with the number past the first post.
  function actionPostUrl(topic, post) {
    return topic.url + (post.postNumber > 1 ? "/" + post.postNumber : "");
  }

  // prioritizeNameFallback
  function postDisplayName(post) {
    if (!post) {
      return at("composer.composer_actions.unknown_user");
    }
    if (!actionsOpts.prioritizeUsername && post.name) {
      return post.name;
    }
    return post.username || post.name || at("composer.composer_actions.unknown_user");
  }

  // ComposerActionStateService#remember
  function rememberActions(s) {
    var snap = s.snapshot || (s.snapshot = { topic: null, post: null });
    var topic = s.actionTopic;
    var post = s.actionPost;
    if (topic && (!snap.topic || topic.id !== snap.topic.id)) {
      snap.topic = topic;
      snap.post = post;
    }
    if (post && (!snap.post || post.id !== snap.post.id)) {
      snap.post = post;
    }
  }

  // The topic and post the menu needs, loaded when a reply opens.
  function loadActionContext(s) {
    if (s.mode !== "reply" || !s.topicId) {
      s.actionContext = Promise.resolve();
      return;
    }
    var postNumber = parseInt(s.replyTo, 10);
    var topic = s.actionTopic ? Promise.resolve(s.actionTopic) : loadActionTopic(s.topicId);
    var post =
      s.actionPost || !(postNumber > 1) ? Promise.resolve(s.actionPost || null) : loadActionPost(s.topicId, postNumber);
    s.actionContext = Promise.all([topic, post]).then(function (found) {
      if (state !== s) {
        return;
      }
      s.actionTopic = found[0];
      s.actionPost = found[1];
      // Composer#open: replying to a whisper whispers.
      if (s.whisper === undefined && s.actionPost && s.actionPost.postType === WHISPER) {
        s.whisper = true;
      }
      rememberActions(s);
      renderActions();
    });
  }

  // The topic of a reply being edited (composer.model.topic), for the
  // options menu item shown only in a PM with a non-human user
  // (add-poll-ui-builder) and the duplicate link warning
  // (composer-messages.js); a first post's comes with loadTopic.
  function loadEditTopic(s) {
    if (s.mode !== "edit" || s.firstPost || !s.topicId) {
      return;
    }
    s.editTopicLoad = loadActionTopic(s.topicId).then(function (topic) {
      if (state === s) {
        s.editTopic = topic;
      }
    });
  }

  // The post edited, as the composer's model holds it (store.find("post")
  // in Composer#open): whether it may be edited, and the post it replies
  // to with that post's author, which ChangeReplyTo changes
  // (Composer#setReplyTo). A draft's reply target stays over the post's.
  function loadEditPost(s) {
    if (s.mode !== "edit" || !s.postId) {
      return;
    }
    s.editPostLoad = fetchJson("/posts/" + s.postId + ".json")
      .then(function (post) {
        if (state !== s) {
          return;
        }
        s.editPost = post;
        if (s.editReplyTo === undefined) {
          var u = post.reply_to_user;
          s.editReplyTo = post.reply_to_post_number == null ? null : post.reply_to_post_number;
          s.editReplyToUser = u
            ? { id: u.id, username: u.username, name: u.name, avatar_template: u.avatar_template }
            : null;
        }
        renderEditReplyTo(s);
      })
      .catch(function () {});
  }

  // canOpenReplyToModal: an edit of a post past the first, by one who may
  // edit it (ComposerActionTitle's canEditReplyTo without the first's).
  function canEditReplyTo(s) {
    return !!s && s.mode === "edit" && !!s.editPost && !!s.editPost.can_edit;
  }

  // ComposerActionTitle's replyTargetSegment in an edit on the desktop:
  // the author of the post replied to, a button opening ChangeReplyTo for
  // one who may edit the post.
  function renderEditReplyTo(s) {
    var title = control.querySelector(".composer-action-title .action-title");
    title.querySelectorAll("[data-reply-target-segment]").forEach(function (el) {
      el.remove();
    });
    if (!s || state !== s || s.mode !== "edit" || mobileView()) {
      return;
    }
    var user = s.editReplyToUser;
    if (!user || !user.username || !user.avatar_template) {
      return;
    }
    var segment =
      svg("share", "reply-to-glyph") +
      '<img loading="lazy" alt="" width="24" height="24" src="' +
      escapeHtml(String(user.avatar_template).replace("{size}", "24")) +
      '" class="avatar"><span class="original-username">' + escapeHtml(user.username) + "</span>";
    var holder = document.createElement("template");
    if (canEditReplyTo(s)) {
      holder.innerHTML =
        '<button class="btn btn-text composer-edit-reply-to btn-default" type="button"><span class="d-button-label">' +
        segment + "</span></button>";
      var button = holder.content.firstChild;
      button.title = at("composer.change_reply_to.open");
      button.addEventListener("click", openChangeReplyTo);
    } else {
      holder.innerHTML = segment;
    }
    Array.prototype.slice.call(holder.content.childNodes).forEach(function (node) {
      if (node.nodeType === 1) {
        node.setAttribute("data-reply-target-segment", "");
      }
    });
    title.appendChild(holder.content);
  }

  // ComposerService#openChangeReplyToModal
  function openChangeReplyTo() {
    var s = state;
    if (!s || !Discourse.changeReplyTo) {
      return;
    }
    Discourse.changeReplyTo.show({
      topicId: s.topicId,
      editingPostNumber: (s.editPost && s.editPost.post_number) || parseInt(s.postNumber, 10),
      currentPostNumber: s.editReplyTo,
      timelineScript: data.timelineScript,
      t: at,
      onSelect: function (post) {
        if (state !== s) {
          return;
        }
        if (!post) {
          s.editReplyTo = null;
          s.editReplyToUser = null;
        } else {
          s.editReplyTo = post.post_number;
          s.editReplyToUser = {
            id: post.user_id,
            username: post.username,
            name: post.name,
            avatar_template: post.avatar_template,
          };
        }
        renderEditReplyTo(s);
      },
    });
  }

  // What the options menu's conditions read, loaded as the composer opens:
  // the topic replied in or edited (in Ember the composer's topic model is
  // there before the toolbar draws), and the post edited.
  function menuContext(s) {
    return Promise.all([s.actionContext, s.topicLoad, s.editTopicLoad, s.editPostLoad]).catch(function () {});
  }

  function postVotingOnly(s) {
    var pv = actionsOpts.postVoting;
    var category = chooser.querySelector("summary").dataset.value;
    return (
      !!pv &&
      s.mode === "create-topic" &&
      !!category &&
      pv.onlyCategories.indexOf(parseInt(category, 10)) !== -1
    );
  }

  // createAsPostVoting's defaultValue, reset when the category changes.
  function postVotingDefault(s) {
    var pv = actionsOpts.postVoting;
    var category = parseInt(chooser.querySelector("summary").dataset.value, 10);
    return (
      !!pv && s.mode === "create-topic" && (postVotingOnly(s) || pv.defaultCategories.indexOf(category) !== -1)
    );
  }

  function inPrivateMessage(s) {
    if (s.mode === "private-message") {
      return true;
    }
    if (s.actionTopic) {
      return s.actionTopic.privateMessage;
    }
    return s.mode === "reply" && document.body.classList.contains("archetype-private_message");
  }

  // composer.canToggleWhisper, canToggleNoBump and canUnlistTopic
  function canToggleWhisper(s) {
    return (
      !!actionsOpts.whisperer && s.mode === "reply" && !(s.actionPost && s.actionPost.postType === WHISPER)
    );
  }

  function canToggleNoBump(s) {
    return s.mode === "reply" && !!actionsOpts.canSkipBump;
  }

  function canUnlistTopic(s) {
    return s.mode === "create-topic" && !!actionsOpts.staff;
  }

  // composer.isWhispering
  function isWhispering(s) {
    return (s.mode === "reply" && !!s.actionPost && s.actionPost.postType === WHISPER) || !!s.whisper;
  }

  function menuItem(id, icon, key) {
    return {
      id: id,
      icon: icon,
      name: at("composer.composer_actions." + key + ".label"),
      description: at("composer.composer_actions." + key + ".desc"),
    };
  }

  function toggleItem(id, cls, key, on) {
    return {
      id: id,
      isToggle: true,
      class: cls,
      label: at("composer.composer_actions." + key + ".label"),
      description: at("composer.composer_actions." + key + ".desc"),
      state: !!on,
    };
  }

  // ComposerActionItemBuilder#build, then discourse-post-voting's
  // composer-actions-content transformer.
  function actionItems(s) {
    var items = [];
    if (s.mode === "edit") {
      return items;
    }
    var topic = s.actionTopic;
    var post = s.actionPost;
    var snap = s.snapshot || {};
    var reply = s.mode === "reply";
    if (s.mode === "create-topic" && actionsOpts.sharedDrafts) {
      items.push(menuItem("shared_draft", "far-clipboard", "shared_draft"));
    }
    if (s.mode === "private-message" && actionsOpts.canCreateTopic) {
      items.push(menuItem("create_topic", "far-pen-to-square", "create_topic"));
    }
    // replyToTopic: from a reply to a post, or a message started from a
    // topic.
    var replyTopic = (reply && post && topic) || (s.mode === "private-message" && snap.topic) ? topic || snap.topic : null;
    if (replyTopic) {
      items.push(menuItem("reply_to_topic", "share", replyTopic.privateMessage ? "reply_to_message" : "reply_to_topic"));
    }
    if (reply && topic) {
      // replyToPost: back to the post after replying to the topic.
      if (!post && snap.post && snap.topic && snap.topic.id === topic.id) {
        items.push(
          Object.assign(menuItem("reply_to_post", "share", "reply_to_post"), {
            name: at("composer.composer_actions.reply_to_post.label", { postUsername: postDisplayName(snap.post) }),
          })
        );
      }
      if (!topic.privateMessage && actionsOpts.canCreateTopic) {
        items.push(menuItem("reply_as_new_topic", "far-pen-to-square", "reply_as_new_topic"));
      }
      if (topic.privateMessage && (topic.allowedUsers.length > 1 || topic.allowedGroups.length > 0)) {
        items.push(menuItem("reply_as_new_group_message", "plus", "reply_as_new_group_message"));
      }
    }
    if (s.mode === "create-topic") {
      if (snap.post && snap.topic) {
        items.push(
          Object.assign(menuItem("reply_to_post", "share", "reply_to_post"), {
            name: at("composer.composer_actions.reply_to_post.label", { postUsername: postDisplayName(snap.post) }),
          })
        );
      }
      if (snap.topic) {
        items.push(
          menuItem("reply_to_topic", "share", snap.topic.privateMessage ? "reply_to_message" : "reply_to_topic")
        );
      }
      if (actionsOpts.canSendPrivateMessages) {
        items.push(menuItem("create_private_message", "envelope", "create_personal_message"));
      }
    }
    if (canToggleWhisper(s)) {
      items.push(toggleItem("toggle_whisper", "composer-toggle-whisper", "toggle_whisper", s.whisper));
    }
    if (canToggleNoBump(s)) {
      items.push(toggleItem("toggle_topic_bump", "composer-toggle-no-bump", "toggle_topic_bump", s.noBump));
    }
    if (canUnlistTopic(s)) {
      items.push(toggleItem("toggle_unlisted", "composer-toggle-unlisted", "toggle_unlisted", s.unlistTopic));
    }
    if (actionsOpts.postVoting && s.mode === "create-topic" && !postVotingOnly(s)) {
      items.push(
        menuItem("togglePostVoting", "plus", s.createAsPostVoting ? "remove_as_post_voting" : "create_as_post_voting")
      );
    }
    return items;
  }

  function hasToggles(s) {
    return canToggleWhisper(s) || canToggleNoBump(s) || canUnlistTopic(s);
  }

  // The trigger's label: discourse-post-voting's actionTitle, else the
  // action's.
  function actionLabel(s) {
    if (actionsOpts.postVoting) {
      if (s.mode === "create-topic" && (s.createAsPostVoting || postVotingOnly(s))) {
        return at("composer.create_post_voting.label");
      }
      if (s.mode === "reply" && s.actionTopic && s.actionTopic.postVoting) {
        return at("post_voting.topic.answer.label");
      }
    }
    if (s.mode === "create-topic") {
      return at("composer.composer_actions.create_topic.label");
    }
    if (s.mode === "private-message") {
      return at("composer.composer_actions.create_personal_message.label");
    }
    if (s.actionPost || (s.replyTo && parseInt(s.replyTo, 10) > 1 && s.replyToUsername)) {
      return s.actionPost ? postDisplayName(s.actionPost) : s.replyToUsername;
    }
    return inPrivateMessage(s)
      ? at("composer.composer_actions.reply_to_message.trigger")
      : at("composer.composer_actions.reply_to_topic.trigger");
  }

  function actionIcon(s) {
    if (s.mode === "create-topic") {
      return "far-pen-to-square";
    }
    if (s.mode === "private-message") {
      return "envelope";
    }
    if (s.actionTopic && s.actionTopic.slowMode) {
      return "hourglass-start";
    }
    if (s.mode === "reply" && s.whisper) {
      return "far-eye-slash";
    }
    return "share";
  }

  // The save button: discourse-post-voting's saveLabel, a whisper's, a
  // reply in a message's, else the action's (saveLabel and saveIcon).
  function syncSubmit() {
    var s = state;
    if (!s) {
      return;
    }
    var inMessage = s.mode === "reply" && inPrivateMessage(s);
    var holder = document.createElement("span");
    holder.innerHTML = inMessage ? data.submitPrivateMessage : data["submit" + modeName(s.mode)];
    var iconEl = holder.querySelector("svg");
    var labelEl = holder.querySelector(".d-button-label");
    var label = null;
    if (actionsOpts.postVoting) {
      if (s.mode === "create-topic" && (s.createAsPostVoting || postVotingOnly(s))) {
        label = at("composer.create_post_voting.label");
      } else if (s.mode === "reply" && s.actionTopic && s.actionTopic.postVoting) {
        label = at("post_voting.topic.answer.label");
      }
    }
    if (isWhispering(s) && s.mode === "reply") {
      label = label || at("composer.create_whisper");
      if (iconEl) {
        iconEl.outerHTML = svg("far-eye-slash");
      }
    }
    if (label && labelEl) {
      labelEl.textContent = label;
    }
    submit.innerHTML = holder.innerHTML;
  }

  // composer-container's indicators beside the title: unlisted, and do
  // not bump while whispering (on a phone, by the save button).
  function syncIndicators(s) {
    var replyTo = control.querySelector(".reply-to");
    replyTo.querySelectorAll(":scope > .unlist, :scope > .no-bump").forEach(function (el) {
      el.remove();
    });
    var panel = control.querySelector(".submit-panel");
    panel.querySelectorAll(":scope > .no-bump").forEach(function (el) {
      el.remove();
    });
    if (!s) {
      return;
    }
    var after = replyTo.querySelector(".composer-action-title");
    if (mobileView()) {
      if (s.noBump && s.mode === "reply") {
        var bump = document.createElement("span");
        bump.className = "no-bump";
        bump.innerHTML = svg("anchor");
        var draft = panel.querySelector("#draft-status, #file-uploading, .composer-error");
        panel.insertBefore(bump, draft);
      }
      return;
    }
    if (s.unlistTopic && s.mode === "create-topic") {
      var unlist = document.createElement("span");
      unlist.className = "unlist";
      unlist.textContent = "(" + at("composer.unlist") + ")";
      after.after(unlist);
      after = unlist;
    }
    if (isWhispering(s) && s.noBump && s.mode === "reply") {
      var noBump = document.createElement("span");
      noBump.className = "no-bump";
      noBump.innerHTML = svg("anchor");
      after.after(noBump);
    }
  }

  // Whether the post replied to is on the screen (an IntersectionObserver,
  // as ComposerActions watches its article).
  var replyTargetObserver = null;
  var replyTargetVisible = false;
  function watchReplyTarget(s) {
    if (replyTargetObserver) {
      replyTargetObserver.disconnect();
      replyTargetObserver = null;
    }
    replyTargetVisible = false;
    var page = document.querySelector("#topic[data-topic-id]");
    if (!s || !s.actionPost || !page || String(page.dataset.topicId) !== String(s.topicId)) {
      return;
    }
    var article = document.querySelector('article[data-post-id="' + s.actionPost.id + '"]');
    if (!article || !window.IntersectionObserver) {
      return;
    }
    replyTargetObserver = new IntersectionObserver(function (entries) {
      entries.forEach(function (entry) {
        replyTargetVisible = entry.isIntersecting;
      });
      if (state === s) {
        renderReplyTarget(s);
      }
    });
    replyTargetObserver.observe(article);
  }

  // The reply target link: to the post (when it is not on the screen) or
  // the topic (from another page) replied to.
  function renderReplyTarget(s) {
    var holder = actionButton.parentNode;
    var old = holder.querySelector(".composer-actions-reply-target-link");
    if (old) {
      old.remove();
    }
    if (!s || s.mode !== "reply" || !s.actionTopic) {
      return;
    }
    var topic = s.actionTopic;
    var post = s.actionPost;
    var page = document.querySelector("#topic[data-topic-id]");
    var onTopic = !!page && String(page.dataset.topicId) === String(topic.id);
    var show = mobileView() || (post ? !replyTargetVisible : !onTopic);
    if (!show) {
      return;
    }
    var label = post
      ? at("post.post_number", { number: post.postNumber })
      : onTopic && !mobileView()
        ? null
        : topic.title;
    var titleText = post
      ? at("composer.composer_actions.reply_to_post.label", { postUsername: postDisplayName(post) })
      : at(
          topic.privateMessage
            ? "composer.composer_actions.reply_to_message.label"
            : "composer.composer_actions.reply_to_topic.label"
        );
    var link = document.createElement("a");
    link.className =
      "composer-actions-reply-target-link btn btn-transparent " +
      (label ? "btn-icon-text" : "btn-icon no-text") +
      (mobileView() ? " btn-small" : "");
    link.href = post ? topic.url + "/" + post.postNumber : topic.url;
    link.title = titleText;
    link.setAttribute("aria-label", titleText);
    link.innerHTML = svg("link");
    if (label) {
      var span = document.createElement("span");
      span.className = "d-button-label composer-actions-reply-target-link__label";
      span.textContent = label;
      link.appendChild(span);
    }
    actionButton.after(link);
  }

  // The whisper indicator: visible or whispering, a toggle of its own.
  function renderWhisperIndicator(s) {
    var holder = actionButton.parentNode;
    var old = holder.querySelector(".composer-whisper-indicator");
    if (old) {
      old.remove();
    }
    if (!s || !canToggleWhisper(s)) {
      return;
    }
    var button = document.createElement("button");
    button.type = "button";
    button.className =
      "btn btn-icon-text composer-whisper-indicator btn-transparent " +
      (s.whisper ? "--whispering" : "--public") +
      (mobileView() ? " btn-small" : "");
    button.innerHTML =
      svg(s.whisper ? "far-eye-slash" : "far-eye") +
      '<span class="d-button-label"></span>';
    button.querySelector(".d-button-label").textContent = at(
      s.whisper ? "composer.whisper_indicator.whispering" : "composer.whisper_indicator.public"
    );
    button.addEventListener("click", function () {
      if (state === s) {
        s.whisper = !s.whisper;
        renderActions();
        scheduleDraft();
      }
    });
    holder.appendChild(button);
  }

  // Everything the action and its toggles draw: the trigger, the reply
  // target link, the whisper indicator, the indicators and the save
  // button.
  function renderActions() {
    var s = state;
    if (actionsMenu && actionsMenu.expanded && (!s || actionsMenu.state !== s)) {
      actionsMenu.close({ focusTrigger: false });
      actionsMenu = null;
    }
    if (!s) {
      renderReplyTarget(null);
      renderEditReplyTo(null);
      renderWhisperIndicator(null);
      syncIndicators(null);
      watchReplyTarget(null);
      return;
    }
    if (s.mode === "edit") {
      actionButton.innerHTML = data.actionEdit;
    } else {
      var content = actionItems(s).length > 0 || hasToggles(s);
      actionButton.innerHTML =
        svg(actionIcon(s)) +
        '<span class="d-button-label"></span>' +
        (content ? svg("angle-down", "composer-actions-caret") : "");
      actionButton.querySelector(".d-button-label").textContent = actionLabel(s);
    }
    if (actionsMenu && actionsMenu.expanded && actionsMenu.content) {
      var shown = actionsMenu.content.querySelector("ul.dropdown-menu");
      if (shown) {
        shown.outerHTML = actionsMenuHtml(s);
      }
    }
    watchReplyTarget(s);
    renderReplyTarget(s);
    renderEditReplyTo(s);
    renderWhisperIndicator(s);
    syncIndicators(s);
    syncSubmit();
  }

  function actionsMenuHtml(s) {
    var items = actionItems(s);
    var html = '<ul class="dropdown-menu">';
    items
      .filter(function (i) {
        return !i.isToggle;
      })
      .forEach(function (i) {
        html +=
          '<li class="dropdown-menu__item"><button class="btn no-text composer-actions-btn' +
          (i.description ? " --with-description" : "") +
          '" data-action-id="' + escapeHtml(i.id) + '" type="button"><div class="composer-actions-btn__icons">' +
          svg(i.icon) +
          '</div><div class="composer-actions-btn__texts"><span class="composer-actions-btn__label">' +
          escapeHtml(i.name) +
          '</span><span class="composer-actions-btn__description">' +
          escapeHtml(i.description || "") +
          "</span></div></button></li>";
      });
    var toggles = items.filter(function (i) {
      return i.isToggle;
    });
    if (!items.length) {
      html +=
        '<div class="composer-actions-btn">' + escapeHtml(at("composer.composer_actions.no_actions_available")) + "</div>";
    }
    if (toggles.length) {
      html += '<li><hr class="dropdown-menu__divider"></li><div class="composer-actions-toggles">';
      toggles.forEach(function (i) {
        html +=
          '<li class="dropdown-menu__item"><div class="' + i.class +
          ' composer-toggle-item --with-description btn" data-toggle-id="' + escapeHtml(i.id) +
          '"><div class="composer-toggle-item__icons"><div class="d-toggle-switch"><label class="d-toggle-switch__label"><button aria-checked="' +
          i.state + '" aria-label="' + escapeHtml(i.label) +
          '" class="d-toggle-switch__checkbox" role="switch" type="button"></button><span class="d-toggle-switch__checkbox-slider">' +
          (i.state ? svg("check") : "") +
          '</span></label></div></div><div class="composer-toggle-item__texts"><span class="composer-toggle-item__label">' +
          escapeHtml(i.label) + '</span><span class="composer-toggle-item__description">' +
          escapeHtml(i.description) + "</span></div></div></li>";
      });
      html += "</div>";
    }
    return html + "</ul>";
  }

  // ComposerActionStateService's clearUnsupportedToggles and
  // reapplyToggles: the toggles go with the composer the new action
  // supports.
  function carriedToggles(opts, from) {
    opts.whisper = opts.mode === "reply" && from.whisper ? true : undefined;
    opts.noBump = opts.mode === "reply" && !!from.noBump;
    opts.unlistTopic = opts.mode === "create-topic" && !!from.unlistTopic;
    return opts;
  }

  // _continuedFromText
  function continuedFromText(post, topic) {
    var url = post && topic ? actionPostUrl(topic, post) : topic && topic.url;
    if (!url || !topic || !topic.title) {
      return null;
    }
    var link = "[" + escapeExpression(topic.title) + "](" + location.protocol + "//" + location.host + url + ")";
    return at("post.continue_discussion", { postLink: link });
  }

  // Composer#prependText with new_line, unless the text has it already.
  function prepended(text, reply) {
    if (!text || reply.indexOf(text) !== -1) {
      return reply;
    }
    return (reply.length > 0 ? text.trim() + "\n\n" : text) + reply;
  }

  // #openComposer: the composer closed (its draft kept) and opened again
  // on the new action with what was written.
  function switchComposer(opts, s) {
    var snapshot = s.snapshot;
    carriedToggles(opts, s);
    close();
    opts.snapshot = snapshot;
    open(opts);
  }

  // #replyFromExisting: the draft destroyed, then a new composer with the
  // text, continuing from the topic or post.
  function replyFromExisting(opts, s) {
    var snapshot = s.snapshot || {};
    var reply = textarea.value;
    carriedToggles(opts, s);
    destroyDraft(s)
      .catch(function () {})
      .then(function () {
        if (state !== s) {
          return;
        }
        close();
        opts.snapshot = snapshot;
        opts.carry = true;
        opts.reply = prepended(continuedFromText(snapshot.post, snapshot.topic), reply);
        open(opts);
      });
  }

  // #switchCreate: the category, title and tags go along.
  function switchCreate(mode, s) {
    var category = chooser.querySelector("summary").dataset.value;
    var max = parseInt(data.maxTitleLength, 10);
    switchComposer(
      {
        mode: mode,
        carry: true,
        draftKey: s.draftKey,
        draftSequence: s.draftSequence,
        reply: textarea.value,
        category: mode === "create-topic" ? category || null : null,
        title: title.value.length <= max ? title.value : "",
        tags: serializedTags().map(function (t) {
          return t.name;
        }),
      },
      s
    );
  }

  function selectAction(id) {
    var s = state;
    if (!s) {
      return;
    }
    rememberActions(s);
    var snap = s.snapshot || {};
    switch (id) {
      case "reply_as_new_group_message":
        replyFromExisting(
          {
            mode: "private-message",
            draftKey: s.draftKey,
            draftSequence: s.draftSequence,
            recipients: snap.topic.allowedUsers.concat(snap.topic.allowedGroups).join(","),
          },
          s
        );
        return;
      case "reply_as_new_topic":
        // Draft.get("new_topic"): a saved new topic draft is overwritten.
        fetchDraft("new_topic")
          .catch(function () {
            return null;
          })
          .then(function (found) {
            var go = function () {
              replyFromExisting(
                {
                  mode: "create-topic",
                  draftKey: "new_topic_" + Date.now(),
                  category: s.actionTopic && s.actionTopic.categoryId,
                },
                s
              );
            };
            if (found && found.draft) {
              withDialog(
                function () {
                  Discourse.dialog.confirm({
                    message: at("composer.composer_actions.reply_as_new_topic.confirm"),
                    confirmLabel: at("composer.ok_proceed"),
                    didConfirm: go,
                  });
                },
                function () {
                  if (window.confirm(at("composer.composer_actions.reply_as_new_topic.confirm"))) {
                    go();
                  }
                }
              );
            } else {
              go();
            }
          });
        return;
      case "reply_to_post":
        switchComposer(
          {
            mode: "reply",
            carry: true,
            draftKey: s.draftKey,
            draftSequence: s.draftSequence,
            reply: textarea.value,
            topicId: snap.topic.id,
            replyTo: snap.post.postNumber,
            actionTopic: snap.topic,
            actionPost: snap.post,
          },
          s
        );
        return;
      case "reply_to_topic":
        switchComposer(
          {
            mode: "reply",
            carry: true,
            draftKey: s.draftKey,
            draftSequence: s.draftSequence,
            reply: textarea.value,
            topicId: snap.topic.id,
            actionTopic: snap.topic,
            actionPost: null,
          },
          s
        );
        return;
      case "shared_draft":
        unported("creating a shared draft");
        return;
      case "create_topic":
        switchCreate("create-topic", s);
        return;
      case "create_private_message":
        switchCreate("private-message", s);
        return;
      case "togglePostVoting":
        // discourse-post-voting's composer-actions-on-select.
        s.createAsPostVoting = !s.createAsPostVoting;
        renderActions();
        return;
    }
  }

  function toggleAction(id) {
    var s = state;
    if (!s) {
      return;
    }
    if (id === "toggle_whisper") {
      s.whisper = !s.whisper;
    } else if (id === "toggle_topic_bump") {
      s.noBump = !s.noBump;
    } else if (id === "toggle_unlisted") {
      s.unlistTopic = !s.unlistTopic;
    }
    renderActions();
    scheduleDraft();
  }

  // A press on the trigger of the open menu closes it (the menu's click
  // outside), and the click that follows does not open it again.
  var closingByTrigger = false;
  actionButton.addEventListener("pointerdown", function () {
    closingByTrigger = !!(actionsMenu && actionsMenu.expanded);
  });

  actionButton.addEventListener("click", function (event) {
    var s = state;
    if (!s || s.mode === "edit" || control.classList.contains("draft") || !window.Discourse || !Discourse.float) {
      return;
    }
    event.stopPropagation();
    if (closingByTrigger) {
      closingByTrigger = false;
      actionsMenu = null;
      return;
    }
    if (actionsMenu && actionsMenu.expanded) {
      actionsMenu.close({ focusTrigger: false });
      actionsMenu = null;
      return;
    }
    (s.actionContext || Promise.resolve()).then(function () {
      if (state !== s || !(actionItems(s).length > 0 || hasToggles(s))) {
        return;
      }
      var menu = Discourse.float.menu(actionButton, {
        mainClass: "composer-actions composer-actions-dropdown",
        placement: "bottom-start",
        modalForMobile: true,
        content: actionsMenuHtml(s),
      });
      menu.state = s;
      actionsMenu = menu;
      var content = menu.content;
      if (!content) {
        return;
      }
      content.addEventListener("click", function (e) {
        var toggle = e.target.closest("[data-toggle-id]");
        if (toggle) {
          e.preventDefault();
          toggleAction(toggle.dataset.toggleId);
          return;
        }
        var item = e.target.closest("[data-action-id]");
        if (item) {
          menu.close({ focusTrigger: true });
          actionsMenu = null;
          selectAction(item.dataset.actionId);
        }
      });
    });
  });

  // The toolbar's options menu (ToolbarPopupMenuOptions over
  // Composer#popupMenuOptions): the items the server listed for the
  // site's settings, as a DMenu. The ones whose actions aren't ported
  // say so instead of acting.
  var optionsTrigger = control.querySelector(".toolbar-menu__options-trigger");
  var popup = { items: [], unported: [] };
  if (optionsTrigger) {
    try {
      popup = JSON.parse(optionsTrigger.dataset.popupOptions);
    } catch (_) {}
  }

  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function svg(name, extra) {
    return (
      '<svg class="fa d-icon d-icon-' + name + " svg-icon fa-width-auto" + (extra ? " " + extra : "") +
      ' svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  var apple = /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent);

  // DShortcut's keycaps for `mod+<shortcut>` (formatShortcut): Ctrl (⌘ on
  // Apple platforms) first, then Shift, then the key.
  function shortcutKbd(shortcut, always) {
    if (!window.matchMedia("(pointer: fine)").matches) {
      return "";
    }
    var keys = shortcut.split("+");
    var key = keys.pop();
    var caps = [apple ? ["⌘", true] : ["Ctrl", false]];
    if (keys.indexOf("Shift") !== -1) {
      caps.push(apple ? ["⇧", true] : ["Shift", false]);
    }
    caps.push([key.length === 1 ? key.toUpperCase() : key, false]);
    return (
      '<kbd aria-hidden="true" class="d-shortcut shortcut' + (always ? " --always-visible" : "") + '" dir="ltr">' +
      caps
        .map(function (cap) {
          return (
            '<kbd class="d-shortcut__key' + (cap[1] ? " d-shortcut__key--glyph" : "") + '">' + escapeHtml(cap[0]) + "</kbd>"
          );
        })
        .join("") +
      "</kbd>"
    );
  }

  function popupItemShown(item) {
    if (item.when === "touch") {
      return document.documentElement.classList.contains("discourse-touch");
    }
    if (item.when === "edit-reply") {
      return canEditReplyTo(state) && (state.editPost.post_number || parseInt(state.postNumber, 10)) > 1;
    }
    // composer.model.topic?.pm_with_non_human_user: the topic replied in
    // or edited.
    if (item.when === "pm-with-non-human-user") {
      var topic = state && (state.mode === "edit" ? state.topic || state.editTopic : state.mode === "reply" ? state.actionTopic : null);
      return !!topic && topic.pmWithNonHumanUser;
    }
    return true;
  }

  // The options whose active() reads the editor's state: spoiler-alert's
  // and toggle-wrap's.
  var POPUP_ACTIVE = {
    spoiler: function (s) {
      return s.inSpoiler;
    },
    "toggle-wrap": function (s) {
      return s.inWrap;
    },
  };

  function popupMenuContent(items) {
    var toolbarState = textManipulation().state || {};
    var html = '<ul class="dropdown-menu">';
    items.forEach(function (item) {
      var active = POPUP_ACTIVE[item.name] && POPUP_ACTIVE[item.name](toolbarState);
      html +=
        '<li class="dropdown-menu__item"><button class="btn no-text btn-icon' + (active ? " --active" : "") + '" title="' + escapeHtml(item.label) + '"' +
        (item.named ? ' data-name="' + escapeHtml(item.name) + '"' : "") + ' type="button">' + svg(item.icon) +
        '<span class="d-button-label"><span class="d-button-label__text">' + escapeHtml(item.label) + "</span>" +
        (item.shortcut ? shortcutKbd(item.shortcut, item.alwaysShowShortcut) : "") +
        (item.showActiveIcon ? svg("check", "d-button-label__active-icon") : "") +
        "</span></button></li>";
    });
    return html + "</ul>";
  }


  // The post a JSON request answers, or why not.
  function fetchJson(path) {
    return fetch(base + path, {
      credentials: "same-origin",
      headers: { Accept: "application/json", "X-Requested-With": "XMLHttpRequest" },
    }).then(function (r) {
      if (!r.ok) {
        throw new Error(r.status + " " + r.statusText);
      }
      return r.json();
    });
  }

  // Composer#importQuote: the whole post replied to (the topic's first
  // post when replying to the topic; when editing, the post the edited
  // one replies to, else that post) quoted at the cursor.
  function importQuote(_, e) {
    if (!state || !state.topicId || !Discourse.buildQuote) {
      return;
    }
    var s = state;
    var post;
    if (s.mode === "edit") {
      post = fetchJson("/posts/by_number/" + s.topicId + "/" + s.postNumber).then(function (edited) {
        return edited.reply_to_post_number
          ? fetchJson("/posts/by_number/" + s.topicId + "/" + edited.reply_to_post_number)
          : edited;
      });
    } else if (s.mode === "reply") {
      post = fetchJson("/posts/by_number/" + s.topicId + "/" + (s.replyTo || 1));
    } else {
      return;
    }
    control.classList.add("loading");
    post
      .then(function (p) {
        if (state !== s) {
          return;
        }
        e.addText(Discourse.buildQuote(p, p.raw, { full: true }));
      })
      .catch(function (e) {
        showError(String(e.message || e));
      })
      .finally(function () {
        control.classList.remove("loading");
      });
  }

  // Each option's action over the toolbar event (d-editor's
  // newToolbarEvent, editor-toolbar.js) of the open editor, as the
  // composer's and the plugins' popup menu options take it.
  var popupActions = {
    quote: importQuote,
    "format-code": function (_, e) {
      e.formatCode();
    },
    details: function (_, e) {
      e.applySurround(
        '\n[details="' + richI18n("composer.details_title") + '"]\n',
        "\n[/details]\n",
        "details_text",
        { multiline: false }
      );
    },
    // ComposerService#insertTable: a four-column table with its default
    // headings and an empty row, as a block (composer:insert-block).
    "toggle-spreadsheet": function () {
      var headings = [1, 2, 3, 4].map(function (number) {
        return richI18n("composer.table.default_heading", { number: number });
      });
      insertBlock(
        [
          "| " + headings.join(" | ") + " |",
          "|" + " --- |".repeat(headings.length),
          "|" + "  |".repeat(headings.length),
        ].join("\n")
      );
    },
    footnote: function (_, e) {
      e.addText("^[" + richI18n("footnote.title") + "]");
    },
    spoiler: function (_, e) {
      if (e.commands && e.commands.toggleSpoiler) {
        e.commands.toggleSpoiler();
      } else {
        e.applySurround("[spoiler]", "[/spoiler]", "spoiler_text", { multiline: false, useBlockMode: true });
      }
    },
    "local-dates": function (_, e) {
      if (Discourse.localDatesCreate) {
        Discourse.localDatesCreate.show(e.addText);
      }
    },
    // discourse-graphviz's sample graph (static/js/graphviz.js).
    graphviz: function (_, e) {
      if (Discourse.graphvizComposer) {
        Discourse.graphvizComposer(function (head, tail) {
          e.applySurround(head, tail, "graphviz_sample", { multiline: false });
        });
      }
    },
    // discourse-math's Insert Math (static/js/discourse-math.js), in
    // block mode when the cursor is at a line's start.
    "insert-math": function (_, e) {
      if (Discourse.mathInsert) {
        Discourse.mathInsert.show(e.selected.pre, e.addText);
      }
    },
    // discourse-poll's builder (static/js/poll-builder.js), loaded the
    // first time, once the menu has closed (and given the focus back to
    // its trigger).
    poll: function (closing, e) {
      (closing || Promise.resolve()).then(function () {
        showPollBuilder(undefined, e);
      });
    },
    // discourse-policy's builder (static/js/policy.js).
    policy: function (_, e) {
      if (Discourse.policyBuilder) {
        Discourse.policyBuilder.show(e.addText);
      }
    },
    // ComposerService#toggleWrap: WrapAttributesModal over the wrap the
    // selection is in, else a new one.
    "toggle-wrap": function (_, e) {
      var state = e.state || {};
      Discourse.richEditor.load().then(function (editorModule) {
        editorModule.showWrapAttributesModal(
          {
            initialAttributes: state.inWrap ? state.wrapAttributes || "" : "",
            onApply: function (attributesString) {
              if (state.inWrap) {
                e.commands.updateWrap(attributesString);
              } else if (e.commands && e.commands.insertWrap) {
                e.commands.insertWrap(editorModule.parseAttributesString(attributesString));
              } else {
                var wrapTag = attributesString.trim() ? "[wrap" + attributesString + "]" : "[wrap]";
                e.applySurround(wrapTag + "\n", "\n[/wrap]", "wrap_text");
              }
            },
            onRemove: state.inWrap
              ? function () {
                  e.commands.removeWrap();
                }
              : undefined,
          },
          richI18n
        );
      });
    },
    // ComposerService#openChangeReplyToModal, once the menu has closed.
    "change-reply-to": function (closing) {
      (closing || Promise.resolve()).then(openChangeReplyTo);
    },
    // discourse-templates' insert template (static/js/templates.js).
    "discourse-templates": function () {
      if (Discourse.templates) {
        Discourse.templates.showComposerUI();
      }
    },
  };

  // edit: the rich editor's Edit poll ({ poll, onSave }); e: the toolbar
  // event the options menu gave.
  function showPollBuilder(edit, e) {
    var config = {};
    try {
      config = JSON.parse(data.pollBuilder || "{}");
    } catch (_) {}
    var showBuilder = function () {
      Discourse.pollBuilder.show(
        config,
        {
          getText: function () {
            return textarea.value;
          },
          addText: function (text) {
            (e || composerBar.event()).addText(text);
          },
        },
        edit
      );
    };
    if (Discourse.pollBuilder) {
      showBuilder();
      return;
    }
    var script = document.createElement("script");
    script.src = config.src;
    script.onload = showBuilder;
    script.onerror = function () {
      showError("Could not load the poll builder.");
    };
    document.head.appendChild(script);
  }

  // The options' shortcuts in the editor (mod+Shift+D, mod+Shift+.):
  // the option's shortcutAction, else its action.
  var popupShortcuts = {
    "local-dates": function (_, e) {
      if (Discourse.localDatesCreate) {
        Discourse.localDatesCreate.currentDateMarkup().then(function (markup) {
          if (markup) {
            e.addText(markup);
          }
        });
      }
    },
  };

  function runPopupAction(item, viaShortcut, closing) {
    var action = (viaShortcut && popupShortcuts[item.name]) || popupActions[item.name];
    if (item.ported && action) {
      action(closing, composerBar.event());
    } else {
      showError(data.labelUnported + " " + item.label);
    }
  }

  if (optionsTrigger) {
    optionsTrigger.addEventListener("click", function (event) {
      event.stopPropagation();
      if (popup.unported.length) {
        showError(data.labelUnported + " " + popup.unported.join(", "));
        return;
      }
      var s = state;
      (s ? menuContext(s) : Promise.resolve()).then(function () {
        if (state !== s) {
          return;
        }
        var shown = popup.items.filter(popupItemShown);
        var menu = Discourse.float.menu(optionsTrigger, {
          identifier: "toolbar-menu__options",
          mainClass: "toolbar-menu__options-content toolbar-popup-menu-options",
          placement: "bottom",
          fallbackPlacements: ["top"],
          offset: 5,
          modalForMobile: true,
          content: popupMenuContent(shown),
        });
        var content = menu && menu.content;
        if (content) {
          content.addEventListener("click", function (e) {
            var button = e.target.closest(".dropdown-menu__item button");
            if (!button) {
              return;
            }
            var index = Array.prototype.indexOf.call(
              content.querySelectorAll(".dropdown-menu__item button"),
              button
            );
            var item = shown[index];
            // ToolbarPopupMenuOptions' onSelect: the menu closed (and the
            // focus back on its trigger) before the option acts, so the
            // editor it focuses keeps the focus.
            var closing = Promise.resolve(menu.close());
            if (item) {
              closing.then(function () {
                runPopupAction(item, false, closing);
              });
            }
          });
        }
      });
    });
  }

  // The same shortcuts in the rich editor's keymap (d-editor's).
  function popupKeymap() {
    var keys = {};
    popup.items.forEach(function (item) {
      if (!item.shortcut || popup.unported.length) {
        return;
      }
      keys[((apple ? "meta+" : "ctrl+") + item.shortcut).toLowerCase()] = function () {
        if (popupItemShown(item)) {
          runPopupAction(item, true);
        }
        return false;
      };
    });
    return keys;
  }

  textarea.addEventListener("keydown", function (event) {
    if (!(event.ctrlKey || event.metaKey) || !event.shiftKey || popup.unported.length) {
      return;
    }
    popup.items.forEach(function (item) {
      if (!item.shortcut || !popupItemShown(item)) {
        return;
      }
      var key = item.shortcut.split("+").pop();
      // Shift+. arrives as ">" on most layouts; the code is the key.
      var code = key === "." ? "Period" : "Key" + key.toUpperCase();
      if (event.code === code) {
        event.preventDefault();
        runPopupAction(item, true);
      }
    });
  });

  // The preview's renderer for others: the insert date modal's preview.
  // The composer service's open, for openers outside this file (the
  // full-page search's "start a new topic").
  Discourse.composerOpen = function (opts) {
    open(opts);
  };

  // What discourse-templates (static/js/templates.js) reads and writes of
  // the composer: whether it has the focus, its topic and the post it
  // replies to or edits, the preview, and the insert (the title when
  // blank, the content as a block).
  Discourse.composerTemplates = {
    focused: function () {
      var editor = control.querySelector(".d-editor");
      return Boolean(state) && Boolean(editor) && editor.contains(document.activeElement);
    },
    context: function () {
      if (!state) {
        return null;
      }
      return {
        topicId: state.topicId || null,
        postNumber: (state.mode === "edit" ? state.postNumber : state.replyTo) || null,
      };
    },
    previewVisible: function () {
      return Boolean(previewToggle) && previewShown();
    },
    previewWrapper: function () {
      return control.querySelector(".d-editor-preview-wrapper");
    },
    textarea: textarea,
    insert: function (templateTitle, content) {
      if (!state) {
        return;
      }
      if (!title.value && !title.disabled) {
        title.value = templateTitle;
      }
      insertBlock(content);
    },
  };

  Discourse.composerCook = function (raw) {
    return loadRenderer().then(function (render) {
      return render(raw);
    });
  };

  control.addEventListener("click", function (event) {
    if (event.target.closest(".composer-toggle-switch")) {
      toggleRichEditor();
      return;
    }
    if (event.target.closest(".toggle-preview")) {
      togglePreview();
    } else if (event.target.closest(".toggle-fullscreen")) {
      toggleFullscreen();
    } else if (event.target.closest(".toggle-toolbar")) {
      // ComposerEditor#toggleToolbar: the mobile view's toolbar.
      var wmd = control.querySelector(".wmd-controls");
      var visible = !wmd.classList.contains("toolbar-visible");
      wmd.classList.toggle("toolbar-visible", visible);
      event.target.closest(".toggle-toolbar").title = visible
        ? data.labelHideToolbar
        : data.labelShowToolbar;
    } else if (event.target.closest("#mobile-file-upload")) {
      var picker = control.querySelector("#file-uploader");
      if (picker) {
        picker.click();
      }
    } else if (event.target.closest(".toggle-minimize")) {
      minimize();
    } else if (event.target.closest(".toggle-save-and-close")) {
      // saveAndCloseComposer: the draft is kept for the next time the
      // composer opens on its key; with nothing written it goes.
      if (state && dirty()) {
        saveDraft();
        close();
      } else if (state) {
        destroyDraft(state);
        close();
      }
    } else if (event.target.closest(".discard-button")) {
      // cancelComposer: DiscardDraftModal's question, then the draft goes.
      var confirmText =
        state && state.mode === "edit" ? data.labelDiscardConfirmEdit : data.labelDiscardConfirm;
      if (state && (!dirty() || window.confirm(confirmText))) {
        destroyDraft(state);
        close();
      }
    } else if (event.target.closest(".save-or-cancel .create")) {
      save(event);
    } else if (control.classList.contains("draft")) {
      expand();
    }
  });

  // A redraw in the other layout (page.js) keeps the composer, its text
  // and its state, and takes the layout's controls from the new page:
  // ComposerToggles, and the save and discard buttons with what goes
  // beside them (the panel's nodes before the upload line).
  function lead(panel) {
    var nodes = [];
    for (var n = panel.firstChild; n && !(n.matches && n.matches("#file-uploading, #draft-status, .composer-error")); n = n.nextSibling) {
      nodes.push(n);
    }
    return nodes;
  }

  control.addEventListener("discourse:redraw", function (event) {
    var incoming = event.detail;
    control.querySelector(".composer-controls").replaceWith(incoming.querySelector(".composer-controls"));
    var panel = control.querySelector(".submit-panel");
    var before = lead(panel);
    var rest = before.length ? before[before.length - 1].nextSibling : panel.firstChild;
    before.forEach(function (n) {
      n.remove();
    });
    lead(incoming.querySelector(".submit-panel")).forEach(function (n) {
      panel.insertBefore(n, rest);
    });
    var saving = submit.disabled;
    submit = control.querySelector(".save-or-cancel .create");
    submit.disabled = saving;
    previewToggle = control.querySelector(".toggle-preview");
    if (state) {
      renderActions();
      applyDiscardIcon(state.mode);
    }
    var toolbar = control.querySelector(".toggle-toolbar");
    if (toolbar) {
      toolbar.title = control.querySelector(".wmd-controls").classList.contains("toolbar-visible")
        ? data.labelHideToolbar
        : data.labelShowToolbar;
    }
    applyFullscreen();
    applyPreviewShown();
    updatePreview();
  });

  // d-editor's focus ring is on the wrapper.
  var wrapper = control.querySelector(".d-editor-textarea-wrapper");
  textarea.addEventListener("focus", function () {
    wrapper.classList.add("in-focus");
  });
  textarea.addEventListener("blur", function () {
    wrapper.classList.remove("in-focus");
  });

  textarea.addEventListener("input", scheduleDraft);
  title.addEventListener("input", scheduleDraft);

  // The toolbar's shortcuts, Tab and Shift+Tab.
  textarea.addEventListener("keydown", function (event) {
    composerBar.keydown(event);
  });

  textarea.addEventListener("keydown", function (event) {
    // Ctrl or Cmd + Enter submits, as in Ember.
    if (event.key === "Enter" && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      save();
    }
  });

  // insertBlock (TextareaTextManipulation#_addBlock): the text on lines
  // of its own in place of the selection, the cursor after it; in the
  // rich editor its blocks in the selection's place.
  function insertBlock(text) {
    text = (text || "").trim();
    if (!text) {
      return;
    }
    if (richEditor) {
      richEditor.textManipulation.insertBlock(text);
      return;
    }
    var value = textarea.value;
    var start = textarea.selectionStart;
    var end = textarea.selectionEnd;
    while (end > start && /\s/.test(value.charAt(end - 1))) {
      end--;
    }
    var pre = value.slice(0, start);
    var post = value.slice(end);
    start -= pre.match(/\n*$/)[0].length;
    if (pre.length > 0) {
      text = "\n\n" + text;
    }
    end += post.match(/^\n*/)[0].length;
    text = post.length > 0 ? text + "\n\n" : text + "\n";
    textarea.setRangeText(text, start, end, "end");
    textarea.setSelectionRange(start + text.length, start + text.length);
    textarea.blur();
    syncRichEditor();
    focusEditor({ preventScroll: true });
    updatePreview();
    scheduleDraft();
  }

  // TopicController#selectText from quote-button.js: into the open
  // composer at its cursor, after a minimized one's text, else a reply
  // (to the post unless it is the first) opened with the quote. Without
  // the right to reply Ember starts a new topic with it instead.
  document.addEventListener("composer:quote", function (event) {
    var d = event.detail || {};
    if (!control.isConnected || !d.quote) {
      return;
    }
    if (state && control.classList.contains("open")) {
      insertBlock(d.quote);
      return;
    }
    if (state && control.classList.contains("draft")) {
      textarea.value = textarea.value + "\n" + d.quote;
      syncRichEditor();
      expand();
      updatePreview();
      scheduleDraft();
      return;
    }
    if (!d.canCreatePost) {
      unported("replying as a new topic");
      return;
    }
    var first = parseInt(d.postNumber, 10) === 1;
    open({
      mode: "reply",
      topicId: d.topicId,
      replyTo: first ? undefined : d.postNumber,
      replyToUsername: first ? undefined : d.username,
      quote: d.quote,
    });
  });

  // The post's edit from quote-button.js (toggleFastEdit's editPost).
  document.addEventListener("composer:edit", function (event) {
    var d = event.detail || {};
    if (!control.isConnected || !d.postId) {
      return;
    }
    open({ mode: "edit", topicId: d.topicId, postId: d.postId, postNumber: d.postNumber });
  });

  // Openers anywhere on the page.
  document.addEventListener("click", function (event) {
    if (!control.isConnected) {
      return;
    }
    var create = event.target.closest("#create-topic");
    if (create) {
      open({ mode: "create-topic", category: create.dataset.category });
      return;
    }
    // composePrivateMessage: the profile's Message button, to the user the
    // profile is for, and the messages page's New Message, to no one. The
    // user card's are user-card.js's.
    if (event.target.closest("#new-private-message-btn")) {
      composePrivateMessage(null, null);
      return;
    }
    var profilePm = event.target.closest("button.compose-pm");
    if (profilePm && !profilePm.closest("#user-card")) {
      var match = location.pathname.slice(base.length).match(/^\/u\/([^/?#]+)/);
      composePrivateMessage(match ? decodeURIComponent(match[1]) : null, null);
      return;
    }
    var topic = document.querySelector("#topic[data-topic-id]");
    var topicId = topic && topic.dataset.topicId;
    var edit = event.target.closest(".post-action-menu__edit");
    if (edit && topicId) {
      open({
        mode: "edit",
        topicId: topicId,
        postId: edit.dataset.postId,
        postNumber: edit.dataset.postNumber,
      });
      return;
    }
    // A small action's edit: its custom text, by the article's post id.
    var smallEdit = event.target.closest(".small-action-edit");
    var smallAction = smallEdit && smallEdit.closest("article.small-action");
    if (smallAction && topicId) {
      open({
        mode: "edit",
        topicId: topicId,
        postId: smallAction.dataset.postId,
        postNumber: smallAction.parentElement.dataset.postNumber,
      });
      return;
    }
    var postReply = event.target.closest(".post-action-menu__reply");
    if (postReply && topicId) {
      open({
        mode: "reply",
        topicId: topicId,
        replyTo: postReply.dataset.postNumber,
        replyToUsername: postReply.dataset.username,
      });
      return;
    }
    if (
      topicId &&
      event.target.closest(".topic-footer-main-buttons .create, .reply-to-post")
    ) {
      open({ mode: "reply", topicId: topicId });
    }
  });

  // Uploads, as lib/uppy/composer-upload.js sends them (one POST to
  // /uploads.json per file, without Uppy) with TextareaPlaceholderHandler's
  // placeholders: "[Uploading: name…]()" at the cursor while the file goes,
  // replaced by getUploadMarkdown's markdown when it is stored. Validation
  // is the server's (its errors are shown); Ember also checks extensions and
  // sizes before sending.
  var fileInput = control.querySelector("#file-uploader");
  var uploadingLine = control.querySelector("#file-uploading");
  var uploads = [];
  var uploadSequence = 0;
  // upload-short-url.js's cache: short url to {url, short_path}.
  var shortUrls = {};

  function escapeRegExp(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  }

  // insertText: at the cursor, replacing the selection.
  function insertText(text) {
    var start = textarea.selectionStart;
    textarea.setRangeText(text, start, textarea.selectionEnd, "end");
    syncRichEditor();
  }

  // replaceText: the first occurrence, keeping the cursor where it was
  // relative to the text around it.
  function replaceText(from, to) {
    var at = textarea.value.indexOf(from);
    if (at < 0) {
      return;
    }
    var cursor = textarea.selectionStart;
    textarea.setRangeText(to, at, at + from.length, "preserve");
    if (cursor > at) {
      var moved = Math.max(at + to.length, cursor + to.length - from.length);
      textarea.selectionStart = textarea.selectionEnd = moved;
    }
    syncRichEditor();
    updatePreview();
    scheduleDraft();
  }

  // #uploadPlaceholder: the file's name (numbered when a placeholder for
  // the same name is already there), on a line of its own.
  function uploadPlaceholder(file) {
    var name = file.name.replace(/[​-‍﻿]/g, "") || data.labelClipboard;
    var pattern = new RegExp(
      "\\[" +
        escapeRegExp(data.labelUploadingFilename).replace(
          "%\\{filename\\}",
          escapeRegExp(name) + "(?:\\()?([0-9])?(?:\\))?"
        ) +
        "\\]\\(\\)",
      "g"
    );
    var matches = textarea.value.match(pattern);
    if (matches) {
      var last = new RegExp(pattern.source).exec(matches[matches.length - 1]);
      name = name + "(" + (last[1] ? parseInt(last[1], 10) + 1 : 1) + ")";
    }
    var placeholder =
      "[" + data.labelUploadingFilename.replace("%{filename}", name) + "]()\n";
    var start = textarea.selectionStart;
    if (!(start === 0 || textarea.value.charAt(start - 1) === "\n")) {
      placeholder = "\n" + placeholder;
    }
    return placeholder;
  }

  // I18n.toHumanSize
  function humanSize(bytes) {
    var units = JSON.parse(data.sizeUnits);
    var size = bytes;
    var iterations = 0;
    while (size >= 1024 && iterations < 4) {
      size = size / 1024;
      iterations += 1;
    }
    var number;
    var unit;
    if (iterations === 0) {
      number = String(size);
      unit = size === 1 ? units.byte_one : units.byte_other;
    } else {
      number = size - Math.floor(size) === 0 ? size.toFixed(0) : size.toFixed(1);
      unit = units[[null, "kb", "mb", "gb", "tb"][iterations]];
    }
    return units.format.replace("%n", number).replace("%u", unit);
  }

  function markdownName(filename) {
    var name = filename.slice(0, filename.lastIndexOf("."));
    return name.replace(/\[|\]|\|/g, "");
  }

  // getUploadMarkdown
  function uploadMarkdown(upload) {
    var name = upload.original_filename;
    if (/\.(png|webp|jpe?g|gif|svg|ico|heic|heif|avif|jxl)$/i.test(name)) {
      return (
        "![" + markdownName(name) + "|" + upload.thumbnail_width + "x" +
        upload.thumbnail_height + "](" + (upload.short_url || upload.url) + ")"
      );
    }
    if (/\.(mp3|og[ga]|opus|wav|m4[abpr]|aac|flac)$/i.test(name)) {
      return "![" + markdownName(name) + "|audio](" + upload.short_url + ")";
    }
    if (/\.(mov|mp4|webm|m4v|3gp|ogv|avi|mpeg)$/i.test(name)) {
      return "![" + markdownName(name) + "|video](" + upload.short_url + ")";
    }
    return (
      "[" + name.replace(/\[|\]|\|/g, "") + "|attachment](" + upload.short_url +
      ") (" + humanSize(upload.filesize) + ")"
    );
  }

  // The progress line: the uploads' combined progress while they send,
  // "Processing upload" while the server stores them.
  function showProgress() {
    if (uploads.length === 0) {
      uploadingLine.hidden = true;
      return;
    }
    var sent = 0;
    var total = 0;
    uploads.forEach(function (u) {
      sent += u.sent;
      total += u.total;
    });
    var sending = uploads.some(function (u) {
      return u.sent < u.total;
    });
    var label = uploadingLine.querySelector("span");
    label.textContent = sending
      ? data.labelUploading + " " + Math.round((sent / Math.max(total, 1)) * 100) + "%"
      : data.labelProcessing;
    uploadingLine.querySelector("#cancel-file-upload").hidden = !sending;
    uploadingLine.hidden = false;
  }

  function finishUpload(upload) {
    uploads = uploads.filter(function (u) {
      return u !== upload;
    });
    showProgress();
  }

  // The rich editor's side (ProsemirrorPlaceholderHandler,
  // rich-editor/extensions/upload-placeholder.js): the placeholder is a
  // node of its document, and the upload's markdown replaces it there.
  var richUploadsModule = null;
  function richUploads() {
    if (!richUploadsModule) {
      richUploadsModule = import("discourse/rich-editor/extensions/upload-placeholder");
    }
    return richUploadsModule;
  }

  // composer:upload-progress:<id> for the placeholder's node view.
  function richProgress(upload) {
    if (upload.rich) {
      richEvents.trigger(
        "composer:upload-progress:" + upload.fileId,
        Math.round((upload.sent / Math.max(upload.total, 1)) * 100)
      );
    }
  }

  // opts.skipPlaceholder: the rich editor's own upload of a pasted data:
  // image, answered with composer:upload-success / composer:upload-error.
  function uploadFile(file, pasted, opts) {
    var skipPlaceholder = !!(opts && opts.skipPlaceholder);
    var upload = {
      id: ++uploadSequence,
      sent: 0,
      total: file.size,
      placeholder: uploadPlaceholder(file),
      xhr: new XMLHttpRequest(),
    };
    upload.fileId = "composer-upload-" + upload.id;
    upload.file = { id: upload.fileId, name: file.name, data: file };
    uploads.push(upload);
    if (skipPlaceholder) {
      upload.skip = true;
    } else if (richEditor) {
      upload.rich = true;
      upload.editor = richEditor;
      upload.handler = richUploads().then(function (module) {
        var handler = module.placeholderHandler(upload.editor, data.labelUploadingFilename);
        handler.insert(upload.file);
        upload.module = module;
        return handler;
      });
    } else {
      insertText(upload.placeholder);
    }
    updatePreview();
    showProgress();

    var body = new FormData();
    body.append("upload_type", "composer");
    if (pasted) {
      body.append("pasted", "true");
    }
    body.append("file", file, file.name);
    var xhr = upload.xhr;
    xhr.open("POST", base + "/uploads.json?client_id=" + clientId);
    var headers = csrfHeaders();
    delete headers["Content-Type"];
    Object.keys(headers).forEach(function (k) {
      xhr.setRequestHeader(k, headers[k]);
    });
    xhr.upload.addEventListener("progress", function (e) {
      if (e.lengthComputable) {
        upload.sent = e.loaded;
        upload.total = e.total;
        showProgress();
        richProgress(upload);
      }
    });
    xhr.addEventListener("load", function () {
      var json = null;
      try {
        json = JSON.parse(xhr.responseText);
      } catch (e) {
        // An error page.
      }
      finishUpload(upload);
      if (xhr.status >= 200 && xhr.status < 300 && json && json.short_url) {
        // cacheShortUploadUrl: the preview needs no lookup for it.
        shortUrls[json.short_url] = { url: json.url, short_path: json.short_path };
        uploadDone(upload, json);
      } else {
        uploadFailed(upload);
        showError(
          json && json.errors ? json.errors.join(" ") : "Could not upload (" + xhr.status + ")."
        );
      }
    });
    xhr.addEventListener("error", function () {
      finishUpload(upload);
      uploadFailed(upload);
      showError("Could not upload " + file.name + ".");
    });
    xhr.send(body);
  }

  // The upload's markdown where its placeholder is: the textarea's, the
  // rich editor's node (success), or the textarea's cursor when the
  // member has switched editors meanwhile.
  function uploadDone(upload, json) {
    var markdown = uploadMarkdown(json);
    if (upload.skip) {
      richEvents.trigger("composer:upload-success", upload.file.name, json);
      return;
    }
    if (!upload.rich) {
      replaceText(upload.placeholder.trim(), markdown);
      return;
    }
    upload.handler.then(function (handler) {
      upload.module.cacheShortUploadUrl(json.short_url, { url: json.url, short_path: json.short_path });
      if (richEditor === upload.editor) {
        handler.success(upload.file, markdown);
      } else {
        insertText(markdown);
        updatePreview();
        scheduleDraft();
      }
    });
  }

  function uploadFailed(upload) {
    if (upload.skip) {
      richEvents.trigger("composer:upload-error", upload.file.name);
    } else if (upload.rich) {
      upload.handler.then(function (handler) {
        if (richEditor === upload.editor) {
          handler.cancel(upload.file);
        }
      });
    } else {
      replaceText(upload.placeholder, "");
    }
  }

  function addFiles(files, pasted) {
    if (!state || files.length === 0) {
      return;
    }
    showError("");
    Array.prototype.forEach.call(files, function (file) {
      uploadFile(file, pasted);
    });
  }

  // The cancel link: every upload stopped and its placeholder removed.
  function cancelUploads() {
    uploads.slice().forEach(function (upload) {
      upload.xhr.abort();
      finishUpload(upload);
      uploadFailed(upload);
    });
  }

  // The rich editor's placeholder cancelled (its button, or the member
  // deleting it), and its own uploads of pasted data: images.
  richEvents.on("composer:cancel-upload", function (event) {
    uploads.slice().forEach(function (upload) {
      if (upload.fileId === (event && event.fileId)) {
        upload.xhr.abort();
        finishUpload(upload);
        uploadFailed(upload);
      }
    });
  });
  richEvents.on("composer:add-files", function (files, opts) {
    if (!state) {
      return;
    }
    Array.prototype.forEach.call(files, function (file) {
      uploadFile(file, false, opts);
    });
  });

  if (fileInput) {
    fileInput.addEventListener("change", function () {
      addFiles(fileInput.files, false);
      fileInput.value = "";
    });
    uploadingLine.querySelector("#cancel-file-upload").addEventListener("click", function (e) {
      e.preventDefault();
      cancelUploads();
    });
    // _pasteEventListener: files pasted without text are uploaded, in the
    // textarea or the rich editor.
    var pasteFiles = function (event) {
      var clipboard = event.clipboardData;
      if (!clipboard) {
        return;
      }
      var types = Array.prototype.slice.call(clipboard.types);
      if (!types.includes("Files") || types.includes("text/plain")) {
        return;
      }
      if (clipboard.files && clipboard.files.length > 0) {
        event.preventDefault();
        addFiles(clipboard.files, true);
      }
    };
    textarea.addEventListener("paste", pasteFiles);
    var richWrapper = control.querySelector(".d-editor-textarea-wrapper");
    if (richWrapper) {
      richWrapper.addEventListener(
        "paste",
        function (event) {
          if (event.target.closest && event.target.closest(".ProseMirror")) {
            pasteFiles(event);
          }
        },
        true
      );
    }
    // DropTarget: files dropped anywhere on the composer.
    control.addEventListener("dragover", function (event) {
      if (event.dataTransfer && Array.prototype.includes.call(event.dataTransfer.types, "Files")) {
        event.preventDefault();
      }
    });
    control.addEventListener("drop", function (event) {
      if (event.dataTransfer && event.dataTransfer.files.length > 0) {
        event.preventDefault();
        addFiles(event.dataTransfer.files, false);
      }
    });
  }

  // resolveAllShortUrls on the preview: upload:// sources and links
  // pointed at the uploads, from the cache or one lookup for the rest.
  var pendingLookup = null;
  function resolveShortUrls() {
    var elements = preview.querySelectorAll("img[data-orig-src], source[data-orig-src], a[data-orig-href]");
    var missing = [];
    elements.forEach(function (el) {
      var attr = el.hasAttribute("data-orig-href") ? "data-orig-href" : "data-orig-src";
      var short = el.getAttribute(attr);
      var cached = shortUrls[short];
      if (!cached) {
        if (missing.indexOf(short) < 0) {
          missing.push(short);
        }
        return;
      }
      if (cached.url === "missing") {
        return;
      }
      el.removeAttribute(attr);
      if (attr === "data-orig-href") {
        el.href = cached.short_path;
      } else {
        el.src = cached.url;
      }
    });
    if (missing.length === 0 || pendingLookup) {
      return;
    }
    pendingLookup = fetch(base + "/uploads/lookup-urls", {
      method: "POST",
      headers: csrfHeaders(),
      credentials: "same-origin",
      body: form(
        missing.map(function (u) {
          return ["short_urls[]", u];
        })
      ),
    })
      .then(function (r) {
        return r.ok ? r.json() : [];
      })
      .then(function (found) {
        found.forEach(function (u) {
          shortUrls[u.short_url] = { url: u.url, short_path: u.short_path };
        });
        missing.forEach(function (u) {
          shortUrls[u] = shortUrls[u] || { url: "missing", short_path: "missing" };
        });
      })
      .catch(function () {
        // Unresolved: the placeholders stay.
      })
      .finally(function () {
        pendingLookup = null;
        resolveShortUrls();
      });
  }

  // TopicDraftsDropdown: the member's latest drafts in a DMenu under the
  // trigger beside New Topic. A draft on a topic goes to the topic (its
  // postUrl, which takes the draft's post id for a post number, as
  // Ember's does); a new topic's opens the composer on it.
  var DRAFTS_LIMIT = 4;
  var draftsMenu = null;

  function closeDraftsMenu() {
    if (!draftsMenu) {
      return;
    }
    draftsMenu.trigger.setAttribute("aria-expanded", "false");
    draftsMenu.trigger.classList.remove("-expanded");
    draftsMenu.content.remove();
    draftsMenu = null;
  }

  // lib/utilities' postUrl
  function draftPostUrl(draft) {
    var url = base + "/t/" + (draft.slug ? draft.slug + "/" : "topic/") + draft.topic_id;
    var postNumber = draft.data.postId || null;
    if (postNumber > 1) {
      url += "/" + postNumber;
    }
    return url;
  }

  function draftIcon(key) {
    var name = key.indexOf("new_topic") === 0
      ? "layer-group"
      : key.indexOf("new_private_message") === 0
        ? "envelope"
        : "reply";
    return (
      '<svg class="fa d-icon d-icon-' + name +
      ' svg-icon fa-width-auto svg-string" width="1em" height="1em" aria-hidden="true" xmlns="http://www.w3.org/2000/svg"><use href="#' +
      name + '"></use></svg>'
    );
  }

  function escapeHtml(s) {
    var div = document.createElement("div");
    div.textContent = s;
    return div.innerHTML;
  }

  function openDraftsMenu(trigger) {
    var labels = trigger.dataset;
    var portals = document.getElementById("d-menu-portals");
    if (!portals) {
      portals = document.createElement("div");
      portals.id = "d-menu-portals";
      document.body.appendChild(portals);
    }
    var content = document.createElement("div");
    content.className = "fk-d-menu topic-drafts-menu-content -animated -expanded";
    content.setAttribute("role", "dialog");
    content.dataset.content = "";
    content.dataset.identifier = "topic-drafts-menu";
    content.dataset.strategy = "absolute";
    content.dataset.placement = "bottom-end";
    content.style.visibility = "hidden";
    content.innerHTML = '<div class="fk-d-menu__inner-content"><ul class="dropdown-menu"></ul></div>';
    portals.appendChild(content);
    trigger.setAttribute("aria-expanded", "true");
    trigger.classList.add("-expanded");
    draftsMenu = { trigger: trigger, content: content, drafts: [] };
    var menu = draftsMenu;

    // UserDraftsStream's first page.
    fetch(base + "/drafts.json?offset=0&limit=30", { credentials: "same-origin" })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(r.status);
        }
        return r.json();
      })
      .then(function (result) {
        if (draftsMenu !== menu) {
          return;
        }
        var drafts = (result.drafts || []).slice(0, DRAFTS_LIMIT).map(function (d) {
          d.data = JSON.parse(d.data);
          if (d.draft_key.indexOf("new_topic") === 0 || d.draft_key.indexOf("new_private_message") === 0) {
            d.title = d.data.title;
          }
          return d;
        });
        menu.drafts = drafts;
        var html = drafts
          .map(function (d, i) {
            return (
              '<li class="dropdown-menu__item topic-drafts-item"><button class="btn btn-icon-text" type="button" data-draft-index="' +
              i + '">' + draftIcon(d.draft_key) +
              '<span class="d-button-label">' + escapeHtml(d.title || labels.labelUntitled) +
              "</span></button></li>"
            );
          })
          .join("");
        var count = parseInt(labels.draftCount, 10);
        if (count > DRAFTS_LIMIT) {
          var other = count - DRAFTS_LIMIT;
          var otherText = (other === 1 ? labels.labelOtherDraftsOne : labels.labelOtherDraftsOther).replace(
            "%{count}",
            other
          );
          html +=
            '<li><hr class="dropdown-menu__divider"></li><li class="dropdown-menu__item"><a class="btn btn-link view-all-drafts" href="' +
            base + '/my/activity/drafts"><span data-other-drafts="' + other + '">' +
            escapeHtml(otherText) + "</span><span>" + escapeHtml(labels.labelViewAll) + "</span></a></li>";
        }
        content.querySelector(".dropdown-menu").innerHTML = html;
        // bottom-end, ten pixels below the trigger.
        var rect = trigger.getBoundingClientRect();
        content.style.left = rect.right + window.scrollX - content.offsetWidth + "px";
        content.style.top = rect.bottom + window.scrollY + 10 + "px";
        content.style.visibility = "visible";
      })
      .catch(function (e) {
        console.error("Failed to fetch drafts with error:", e);
        closeDraftsMenu();
      });
  }

  document.addEventListener("click", function (event) {
    if (!control.isConnected) {
      return;
    }
    var trigger = event.target.closest(".topic-drafts-menu-trigger");
    if (trigger) {
      if (draftsMenu) {
        closeDraftsMenu();
      } else {
        openDraftsMenu(trigger);
      }
      return;
    }
    if (!draftsMenu) {
      return;
    }
    var item = event.target.closest(".topic-drafts-item [data-draft-index]");
    if (item) {
      var draft = draftsMenu.drafts[parseInt(item.dataset.draftIndex, 10)];
      closeDraftsMenu();
      if (draft.topic_id) {
        window.location.href = draftPostUrl(draft);
      } else {
        open({
          mode: draft.draft_key.indexOf("new_private_message") === 0 ? "private-message" : "create-topic",
          draftKey: draft.draft_key,
          draftSequence: draft.sequence,
        });
      }
    } else if (!event.target.closest(".fk-d-menu")) {
      closeDraftsMenu();
    }
  });
  document.addEventListener("keydown", function (event) {
    if (control.isConnected && event.key === "Escape") {
      closeDraftsMenu();
    }
  });

  // DVirtualHeight, on a phone or an iPad: --composer-vh follows the
  // visual viewport, and html.keyboard-visible says the on-screen keyboard
  // took more than 150px of it.
  if (window.visualViewport && (root.classList.contains("mobile-view") || root.classList.contains("ipados-device"))) {
    var previousHeight = null;
    var viewportTimer = null;
    var onViewportResize = function () {
      // isZoomed
      if (window.visualViewport.scale > 1) {
        return;
      }
      var height = Math.round(window.visualViewport.height);
      if (previousHeight && Math.abs(previousHeight - height) <= 1) {
        return;
      }
      previousHeight = height;
      root.style.setProperty("--composer-vh", height / 100 + "px");
      root.classList.toggle("keyboard-visible", window.innerHeight - window.visualViewport.height > 150);
    };
    onViewportResize();
    window.visualViewport.addEventListener("resize", function () {
      clearTimeout(viewportTimer);
      viewportTimer = setTimeout(onViewportResize, 50);
    });
  }

  // setupComposerPosition on a phone or an iPad: the page scrolled so the
  // composer's top went above the window comes back (correctScrollPosition),
  // and a drag in an editor with nothing to scroll doesn't scroll the page.
  if (root.classList.contains("mobile-device") || root.classList.contains("ipados-device")) {
    window.addEventListener("scroll", function () {
      if (!control.classList.contains("open")) {
        return;
      }
      setTimeout(function () {
        var rect = control.getBoundingClientRect();
        if (rect.top < -1) {
          window.scrollTo({ top: window.scrollY + rect.top, behavior: "instant" });
        }
      }, 150);
    });
    textarea.addEventListener("touchmove", function (event) {
      if (event.touches.length > 1) {
        return;
      }
      if (textarea.scrollHeight <= textarea.clientHeight && window.getSelection().toString() === "") {
        event.preventDefault();
        event.stopPropagation();
      }
    }, { passive: false });
  }

  // Last, once everything it opens with is set up.
  if (window.Discourse && Discourse.onPage) {
    Discourse.onPage(routeArrived);
  } else {
    routeArrived();
  }

  window.composer = { open: open, composePrivateMessage: composePrivateMessage };
})();
