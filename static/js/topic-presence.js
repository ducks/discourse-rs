// discourse-presence: who else is replying to the topic (or whispering
// in it) or editing the post, shown in the composer
// (ComposerPresenceDisplay, before its controls) and above the topic's
// footer buttons (TopicPresenceDisplay); and the member's own replying
// or editing reported while their text has changed
// (ComposerPresenceManager). The composer says what it holds with a
// `composer:presence` event (composer.js); the strings and settings come
// with it (#reply-control's data-presence).
(function () {
  "use strict";

  if (!Discourse.once("topic-presence")) {
    return;
  }

  // ComposerPresenceManager's KEEP_ALIVE.
  var KEEP_ALIVE = 10 * 1000;

  function config() {
    var control = document.getElementById("reply-control");
    if (!control || !control.dataset.presence || !window.Discourse.presence) {
      return null;
    }
    try {
      return JSON.parse(control.dataset.presence);
    } catch (e) {
      return null;
    }
  }

  function escapeHtml(s) {
    return String(s === undefined || s === null ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#x27;");
  }

  function basePath() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  // ComposerPresenceManager#notifyState: present in the channel while the
  // text has changed, for KEEP_ALIVE from entering.
  var manager = { name: null, channel: null, timer: null };

  function leave() {
    clearTimeout(manager.timer);
    manager.timer = null;
    if (manager.channel) {
      manager.channel.leave().catch(function () {});
    }
    manager.channel = null;
    manager.name = null;
  }

  function notifyState(cfg, name, replying) {
    if (!replying) {
      leave();
      return;
    }
    if (cfg.hide_presence) {
      return;
    }
    if (manager.name !== name) {
      leave();
      manager.name = name;
      manager.channel = Discourse.presence.getChannel(name);
      manager.channel.enter().catch(function () {});
      manager.timer = setTimeout(leave, KEEP_ALIVE);
    }
  }

  // The users and text both displays draw: DUserLink around a small avatar
  // for each, then the description and its wave.
  function usersHtml(cfg, users, description) {
    var avatars = users
      .map(function (u) {
        var label = cfg.profile_possessive.replace("%{username}", u.username);
        return (
          '<a aria-label="' + escapeHtml(label) + '" data-user-card="' + escapeHtml(u.username) +
          '" href="' + escapeHtml(basePath() + "/u/" + u.username) + '" tabindex="0">' +
          '<img alt="" width="24" height="24" src="' +
          escapeHtml((u.avatar_template || "").replace("{size}", "24")) +
          '" class="avatar" title="' + escapeHtml(u.username) + '"></a>'
        );
      })
      .join("");
    return (
      '<div class="presence-users"><div class="presence-avatars">' + avatars + "</div>" +
      '<span class="presence-text"><span class="description">' + escapeHtml(description) +
      '</span><span class="wave"><span class="dot">.</span><span class="dot">.</span>' +
      '<span class="dot">.</span></span></span></div>'
    );
  }

  function label(cfg, key, count) {
    return cfg[key + (count === 1 ? "_one" : "_other")];
  }

  // A set of channels a display follows, and their users together.
  function Watch(names, onChange) {
    var self = this;
    this.channels = names.map(function (name) {
      var channel = Discourse.presence.getChannel(name);
      channel.on(onChange);
      channel.subscribe().catch(function () {});
      return channel;
    });
    this.stop = function () {
      self.channels.forEach(function (channel) {
        channel.off(onChange);
        channel.unsubscribe();
      });
    };
    this.users = function () {
      var all = [];
      self.channels.forEach(function (channel) {
        (channel.users() || []).forEach(function (u) {
          all.push(u);
        });
      });
      return all;
    };
  }

  // ComposerPresenceDisplay
  var composer = { key: null, watch: null, notifyKey: null, detail: null };

  function composerState(detail) {
    if (!detail) {
      return null;
    }
    if (detail.mode === "edit") {
      return "edit";
    }
    // replyingToTopic; the port's composer has no whisper toggle.
    return detail.mode === "reply" ? "reply" : null;
  }

  function drawComposer(cfg) {
    var outlet = document.querySelector(".before-composer-controls-outlet.presence");
    if (!outlet) {
      return;
    }
    var users = composer.watch ? composer.watch.users() : [];
    users = users
      .filter(function (u) {
        return u.id !== cfg.user_id;
      })
      .slice(0, cfg.max_users_shown);
    var state = composerState(composer.detail);
    outlet.innerHTML =
      users.length > 0
        ? usersHtml(cfg, users, label(cfg, state === "edit" ? "editing" : "replying", users.length))
        : "";
  }

  document.addEventListener("composer:presence", function (event) {
    var cfg = config();
    if (!cfg) {
      return;
    }
    var detail = event.detail;
    composer.detail = detail;
    var state = composerState(detail);
    // The channels it shows: the topic's reply (and whisper) channels while
    // replying, the post's edit channel while editing.
    var names = [];
    if (state === "reply" && detail.topicId) {
      names.push("/discourse-presence/reply/" + detail.topicId);
      if (cfg.whisperer) {
        names.push("/discourse-presence/whisper/" + detail.topicId);
      }
    } else if (state === "edit" && detail.postId) {
      names.push("/discourse-presence/edit/" + detail.postId);
    }
    var key = names.join(",");
    if (key !== composer.key) {
      if (composer.watch) {
        composer.watch.stop();
      }
      composer.key = key;
      composer.watch = names.length
        ? new Watch(names, function () {
            drawComposer(cfg);
          })
        : null;
    }
    drawComposer(cfg);
    // notifyState runs again (leaving first) when what it reads changes.
    var entity = state === "edit" ? detail.postId : detail && detail.topicId;
    var notifyKey = state && entity ? state + "/" + entity + "/" + detail.dirty : null;
    if (notifyKey !== composer.notifyKey) {
      composer.notifyKey = notifyKey;
      leave();
      if (notifyKey) {
        notifyState(cfg, "/discourse-presence/" + state + "/" + entity, detail.dirty);
      }
    }
  });

  // TopicPresenceDisplay, above the topic's footer buttons. This script
  // loads in the head: the page hook waits for the body.
  function onPageReady(fn) {
    if (document.readyState === "loading") {
      document.addEventListener("DOMContentLoaded", function () {
        Discourse.onPage(fn);
      });
    } else {
      Discourse.onPage(fn);
    }
  }

  onPageReady(function () {
    var outlet = document.querySelector(".topic-above-footer-buttons-outlet.presence");
    var topic = document.querySelector("#topic[data-topic-id]");
    var cfg = config();
    if (!outlet || !topic || !cfg) {
      return;
    }
    var id = topic.dataset.topicId;
    var names = ["/discourse-presence/reply/" + id];
    if (cfg.whisperer) {
      names.push("/discourse-presence/whisper/" + id);
    }
    var watch = new Watch(names, function () {
      var users = watch.users().filter(function (u) {
        return u.id !== cfg.user_id;
      });
      outlet.innerHTML =
        users.length > 0 ? usersHtml(cfg, users, label(cfg, "replying_to_topic", users.length)) : "";
    });
    return function () {
      watch.stop();
    };
  });
})();
