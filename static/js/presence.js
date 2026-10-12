// lib/user-presence and the presence service (services/presence.js).
//
// Discourse.userPresent({ userUnseenTime, browserHiddenTime }): whether the
// member has been active (a click, key, scroll or touch) and the tab
// visible lately; Discourse.onPresenceChange({ ..., callback }) hears it
// change.
//
// Discourse.presence.getChannel(name) gives a channel to enter({
// onlyWhileActive, activeOptions }) and leave, and to subscribe() to:
// its users() then follow the server's state, and on("change", fn) hears
// each change. Entering and leaving are batched into /presence/update
// (debounced, at most once a second, and every 30 seconds while present);
// subscribing reads /presence/get and then follows /presence<name> on
// the bus, one stream for every subscribed channel.
(function () {
  "use strict";

  if (!Discourse.once("presence")) {
    return;
  }

  // lib/user-presence

  var DEFAULT_USER_UNSEEN_MS = 60000;
  var DEFAULT_BROWSER_HIDDEN_MS = 0;
  var presenceCallbacks = [];
  var browserHiddenAt = null;
  var lastUserActivity = Date.now();
  var callbackWaitingForPresence = false;
  var debounceUpdateDate = null;

  function userPresent(options) {
    options = options || {};
    var browserHiddenTime =
      options.browserHiddenTime === undefined ? DEFAULT_BROWSER_HIDDEN_MS : options.browserHiddenTime;
    var userUnseenTime = options.userUnseenTime === undefined ? DEFAULT_USER_UNSEEN_MS : options.userUnseenTime;
    if (browserHiddenAt && Date.now() - browserHiddenAt >= browserHiddenTime) {
      return false;
    }
    return Date.now() - lastUserActivity < userUnseenTime;
  }

  function onPresenceChange(options) {
    var userUnseenTime = options.userUnseenTime === undefined ? DEFAULT_USER_UNSEEN_MS : options.userUnseenTime;
    var browserHiddenTime =
      options.browserHiddenTime === undefined ? DEFAULT_BROWSER_HIDDEN_MS : options.browserHiddenTime;
    presenceCallbacks.push({
      userUnseenTime: userUnseenTime,
      browserHiddenTime: browserHiddenTime,
      lastState: userPresent({ userUnseenTime: userUnseenTime, browserHiddenTime: browserHiddenTime }),
      callback: options.callback,
    });
  }

  function processChanges() {
    var hidden = document.hidden;
    if (!!browserHiddenAt !== hidden) {
      browserHiddenAt = hidden ? Date.now() : null;
    }
    callbackWaitingForPresence = false;
    presenceCallbacks.forEach(function (entry) {
      var state = userPresent(entry);
      if (entry.lastState !== state) {
        try {
          entry.callback(state);
        } finally {
          entry.lastState = state;
        }
      }
      if (!state) {
        callbackWaitingForPresence = true;
      }
    });
  }

  // seenUser: activity counted, at once when someone waits for it.
  function seenUser() {
    if (callbackWaitingForPresence) {
      clearTimeout(debounceUpdateDate);
      debounceUpdateDate = null;
      lastUserActivity = Date.now();
      processChanges();
    } else if (!debounceUpdateDate) {
      debounceUpdateDate = setTimeout(function () {
        debounceUpdateDate = null;
        lastUserActivity = Date.now();
      }, 1000);
    }
  }

  document.addEventListener("touchmove", seenUser, { passive: true });
  document.addEventListener("click", seenUser, { passive: true });
  document.addEventListener("keydown", seenUser, { passive: true });
  window.addEventListener("scroll", seenUser, { passive: true });
  window.addEventListener("focus", seenUser, { passive: true });
  document.addEventListener(
    "visibilitychange",
    function () {
      if (document.hidden) {
        processChanges();
      } else {
        seenUser();
      }
    },
    { passive: true }
  );
  setInterval(processChanges, 10000);

  Discourse.userPresent = userPresent;
  Discourse.onPresenceChange = onPresenceChange;

  // services/presence

  var PRESENCE_INTERVAL_S = 30;
  var DEFAULT_PRESENCE_DEBOUNCE_MS = 500;
  var PRESENCE_THROTTLE_MS = 1000;
  var PRESENCE_GET_RETRY_MS = 5000;
  var DEFAULT_ACTIVE_OPTIONS = { userUnseenTime: 60000, browserHiddenTime: 10000 };

  // messageBus.clientId: one per tab, kept across page swaps.
  var clientId = Array.prototype.map
    .call(crypto.getRandomValues(new Uint8Array(16)), function (b) {
      return ("0" + b.toString(16)).slice(-2);
    })
    .join("");

  function basePath() {
    var meta = document.querySelector("meta[name='discourse-base-uri']");
    return meta ? meta.content : "";
  }

  function csrfToken() {
    var meta = document.querySelector("meta[name='csrf-token']");
    return meta ? meta.content : null;
  }

  function loggedIn() {
    return !!csrfToken();
  }

  var queuedEvents = [];
  var presentProxies = new Map();
  var subscribedProxies = new Map();
  var states = new Map();
  var initialDataRequests = {};
  var initialDataRunning = false;
  var previousPresentButInactive = new Set();
  var debounceMs = DEFAULT_PRESENCE_DEBOUNCE_MS;
  var debounceTimer = null;
  var nextUpdateTimer = null;
  var throttleTimer = null;
  var lastUpdate = null;
  var updateRunning = false;

  function deferred() {
    var d = {};
    d.promise = new Promise(function (resolve, reject) {
      d.resolve = resolve;
      d.reject = reject;
    });
    return d;
  }

  function addTo(map, proxy) {
    var set = map.get(proxy.name);
    if (!set) {
      set = new Set();
      map.set(proxy.name, set);
    }
    set.add(proxy);
    return set.size;
  }

  function removeFrom(map, proxy) {
    var set = map.get(proxy.name);
    if (set) {
      set.delete(proxy);
      if (set.size === 0) {
        map.delete(proxy.name);
      }
    }
    return set ? set.size : 0;
  }

  // _getInitialData: the channels asked for in one tick, fetched together.
  function getInitialData(name) {
    if (!initialDataRequests[name]) {
      initialDataRequests[name] = deferred();
    }
    Promise.resolve().then(makeInitialDataRequest);
    return initialDataRequests[name].promise;
  }

  function makeInitialDataRequest() {
    if (initialDataRunning) {
      setTimeout(makeInitialDataRequest, 0);
      return;
    }
    var names = Object.keys(initialDataRequests).slice(0, 50);
    if (names.length === 0) {
      return;
    }
    initialDataRunning = true;
    var query = new URLSearchParams();
    names.forEach(function (name) {
      query.append("channels[]", name);
    });
    fetch(basePath() + "/presence/get?" + query.toString(), {
      credentials: "same-origin",
      headers: { "X-Requested-With": "XMLHttpRequest", Accept: "application/json" },
    })
      .then(function (r) {
        if (!r.ok) {
          throw new Error(String(r.status));
        }
        return r.json();
      })
      .then(function (result) {
        Object.keys(result).forEach(function (name) {
          var request = initialDataRequests[name];
          if (!request) {
            return;
          }
          if (result[name]) {
            request.resolve(result[name]);
          } else {
            request.reject(new Error("PresenceChannel '" + name + "' not found"));
          }
          delete initialDataRequests[name];
        });
      })
      .catch(function () {
        setTimeout(makeInitialDataRequest, PRESENCE_GET_RETRY_MS);
      })
      .finally(function () {
        initialDataRunning = false;
      });
  }

  // The bus stream behind every subscribed channel: reopened from the
  // earliest position any of them needs when the set changes (a message
  // seen twice changes nothing: entering and leaving are idempotent).
  var stream = null;
  var streamChannels = "";
  var streamPosition = null;

  function parsePosition(p) {
    var parts = String(p || "").split("-");
    return parts.length === 2 ? [BigInt(parts[0]), BigInt(parts[1])] : null;
  }

  function earlier(a, b) {
    var pa = parsePosition(a);
    var pb = parsePosition(b);
    if (!pa) {
      return b;
    }
    if (!pb) {
      return a;
    }
    return pa[0] < pb[0] || (pa[0] === pb[0] && pa[1] <= pb[1]) ? a : b;
  }

  function restartStream(from) {
    var names = Array.from(states.keys()).sort();
    var channels = names
      .map(function (name) {
        return "/presence" + name;
      })
      .join(",");
    if (stream && channels === streamChannels && !from) {
      return;
    }
    var position = from ? earlier(from, streamPosition) : streamPosition;
    if (stream) {
      stream.close();
      stream = null;
    }
    streamChannels = channels;
    if (!channels || !window.EventSource) {
      return;
    }
    streamPosition = position;
    var url =
      basePath() + "/bus/events?channels=" + encodeURIComponent(channels) +
      (position ? "&position=" + encodeURIComponent(position) : "");
    stream = new EventSource(url, { withCredentials: true });
    stream.addEventListener("message", function (event) {
      streamPosition = event.lastEventId || streamPosition;
      var message = JSON.parse(event.data);
      var state = states.get(String(message.channel).replace(/^\/presence/, ""));
      if (state) {
        state.process(message.data);
      }
    });
    // Messages trimmed before they were read: each channel read again.
    stream.addEventListener("gap", function () {
      states.forEach(function (state) {
        state.resubscribe();
      });
    });
  }

  // PresenceChannelState: a channel's users as the server tells them.
  function ChannelState(name) {
    this.name = name;
    this.users = null;
    this.count = null;
    this.countOnly = null;
    this.listeners = [];
    this.subscribed = false;
  }

  ChannelState.prototype.subscribe = function (initialData) {
    var self = this;
    var data = initialData ? Promise.resolve(initialData) : getInitialData(this.name);
    return data.then(function (initial) {
      self.count = initial.count;
      if (initial.users) {
        self.users = initial.users.slice();
        self.countOnly = false;
      } else {
        self.users = null;
        self.countOnly = true;
      }
      self.subscribed = true;
      restartStream(initial.last_message_id);
      self.trigger();
    });
  };

  ChannelState.prototype.unsubscribe = function () {
    this.subscribed = false;
    this.users = null;
    this.count = null;
    this.trigger();
  };

  ChannelState.prototype.resubscribe = function () {
    this.unsubscribe();
    return this.subscribe();
  };

  ChannelState.prototype.trigger = function () {
    this.listeners.slice().forEach(function (fn) {
      fn();
    });
  };

  ChannelState.prototype.process = function (data) {
    if (!this.subscribed) {
      return;
    }
    if (this.countOnly && data.count_delta !== undefined) {
      this.count += data.count_delta;
      this.trigger();
    } else if (!this.countOnly && (data.entering_users || data.leaving_user_ids)) {
      var users = this.users;
      (data.entering_users || []).forEach(function (user) {
        if (
          !users.some(function (u) {
            return u.id === user.id;
          })
        ) {
          users.push(user);
        }
      });
      if (data.leaving_user_ids) {
        var leaving = new Set(data.leaving_user_ids);
        this.users = users.filter(function (u) {
          return !leaving.has(u.id);
        });
      }
      this.count = this.users.length;
      this.trigger();
    } else {
      this.resubscribe();
    }
  };

  // PresenceChannel: one holder's view of a channel.
  function Channel(name) {
    this.name = name;
    this.present = false;
    this.subscribed = false;
    this.activeOptions = null;
    this.state = null;
    this.handlers = [];
    var self = this;
    this.publishChange = function () {
      self.handlers.forEach(function (fn) {
        fn(self);
      });
    };
  }

  Channel.prototype.users = function () {
    return this.subscribed && this.state ? this.state.users : undefined;
  };

  Channel.prototype.count = function () {
    return this.subscribed && this.state ? this.state.count : undefined;
  };

  Channel.prototype.on = function (fn) {
    this.handlers.push(fn);
  };

  Channel.prototype.off = function (fn) {
    this.handlers = this.handlers.filter(function (h) {
      return h !== fn;
    });
  };

  Channel.prototype.enter = function (options) {
    options = options || {};
    var onlyWhileActive = options.onlyWhileActive !== false;
    this.activeOptions = onlyWhileActive ? options.activeOptions || DEFAULT_ACTIVE_OPTIONS : null;
    if (this.present) {
      return Promise.resolve();
    }
    this.present = true;
    return queueChange(this, "enter", addTo(presentProxies, this) > 1);
  };

  Channel.prototype.leave = function () {
    if (!this.present) {
      return Promise.resolve();
    }
    this.present = false;
    return queueChange(this, "leave", removeFrom(presentProxies, this) > 0);
  };

  Channel.prototype.subscribe = function (initialData) {
    if (this.subscribed) {
      return Promise.resolve();
    }
    var self = this;
    addTo(subscribedProxies, this);
    var state = states.get(this.name);
    var ready;
    if (!state) {
      state = new ChannelState(this.name);
      states.set(this.name, state);
      ready = state.subscribe(initialData);
    } else {
      ready = Promise.resolve();
    }
    return ready.then(function () {
      self.state = state;
      self.subscribed = true;
      state.listeners.push(self.publishChange);
      self.publishChange();
    });
  };

  Channel.prototype.unsubscribe = function () {
    if (!this.subscribed) {
      return Promise.resolve();
    }
    var state = this.state;
    state.listeners = state.listeners.filter(function (fn) {
      return fn !== this.publishChange;
    }, this);
    if (removeFrom(subscribedProxies, this) === 0) {
      state.unsubscribe();
      states.delete(this.name);
      restartStream();
    }
    this.subscribed = false;
    this.state = null;
    this.publishChange();
    return Promise.resolve();
  };

  function queueChange(proxy, type, alreadyDone) {
    if (!loggedIn()) {
      return Promise.reject(new Error("Must be logged in to " + type + " a presence channel"));
    }
    if (alreadyDone) {
      return Promise.resolve();
    }
    var d = deferred();
    queuedEvents.push({ channel: proxy.name, type: type, deferred: d });
    scheduleNextUpdate();
    return d.promise;
  }

  // _dedupQueue: the last event for each channel stands for all of it.
  function dedupQueue() {
    var byChannel = {};
    queuedEvents.forEach(function (e) {
      if (byChannel[e.channel]) {
        byChannel[e.channel].deferred.resolve(e.deferred.promise);
      }
      byChannel[e.channel] = e;
    });
    queuedEvents = Object.keys(byChannel).map(function (k) {
      return byChannel[k];
    });
  }

  function updateServer() {
    lastUpdate = Date.now();
    updateRunning = true;
    clearTimeout(nextUpdateTimer);
    nextUpdateTimer = null;
    dedupQueue();
    var queue = queuedEvents;
    queuedEvents = [];
    var present = [];
    var presentButInactive = new Set();
    var leaving = queue
      .filter(function (e) {
        return e.type === "leave";
      })
      .map(function (e) {
        return e.channel;
      });
    presentProxies.forEach(function (proxies, name) {
      var active = Array.from(proxies).some(function (p) {
        return !p.activeOptions || userPresent(p.activeOptions);
      });
      if (active) {
        present.push(name);
      } else {
        presentButInactive.add(name);
        if (!previousPresentButInactive.has(name)) {
          leaving.push(name);
        }
      }
    });
    previousPresentButInactive = presentButInactive;
    if (queue.length === 0 && present.length === 0 && leaving.length === 0) {
      updateRunning = false;
      scheduleNextUpdate();
      return;
    }
    var body = new URLSearchParams();
    body.append("client_id", clientId);
    present.forEach(function (name) {
      body.append("present_channels[]", name);
    });
    leaving.forEach(function (name) {
      body.append("leave_channels[]", name);
    });
    fetch(basePath() + "/presence/update", {
      method: "POST",
      credentials: "same-origin",
      headers: {
        "X-CSRF-Token": csrfToken(),
        "X-Requested-With": "XMLHttpRequest",
        Accept: "application/json",
      },
      body: body,
    })
      .then(function (r) {
        if (r.status === 429) {
          var wait = Number(r.headers.get("Retry-After") || 10);
          var error = new Error("rate limited");
          error.waitSeconds = wait;
          throw error;
        }
        if (!r.ok) {
          throw new Error(String(r.status));
        }
        return r.json();
      })
      .then(function (response) {
        queue.forEach(function (e) {
          if (response[e.channel] === false) {
            e.deferred.reject(new Error("PresenceChannel '" + e.channel + "' not found"));
          } else {
            e.deferred.resolve();
          }
        });
        debounceMs = DEFAULT_PRESENCE_DEBOUNCE_MS;
      })
      .catch(function (e) {
        queuedEvents = queue.concat(queuedEvents);
        debounceMs = e.waitSeconds
          ? e.waitSeconds * 1000
          : Math.min(debounceMs * 2, PRESENCE_INTERVAL_S * 1000);
      })
      .finally(function () {
        updateRunning = false;
        scheduleNextUpdate();
      });
  }

  function throttledUpdateServer() {
    if (!lastUpdate || Date.now() - lastUpdate > PRESENCE_THROTTLE_MS) {
      updateServer();
    } else if (!throttleTimer) {
      throttleTimer = setTimeout(function () {
        throttleTimer = null;
        updateServer();
      }, PRESENCE_THROTTLE_MS - (Date.now() - lastUpdate));
    }
  }

  function scheduleNextUpdate() {
    if (updateRunning) {
      return;
    }
    if (queuedEvents.length > 0) {
      clearTimeout(nextUpdateTimer);
      nextUpdateTimer = null;
      clearTimeout(debounceTimer);
      debounceTimer = setTimeout(throttledUpdateServer, debounceMs);
    } else if (!nextUpdateTimer && presentProxies.size > 0) {
      nextUpdateTimer = setTimeout(throttledUpdateServer, PRESENCE_INTERVAL_S * 1000);
    }
  }

  // _beaconLeaveAll: what the tab is present in, left as it closes.
  window.addEventListener("beforeunload", function () {
    if (!loggedIn()) {
      return;
    }
    dedupQueue();
    var leaving = queuedEvents
      .filter(function (e) {
        return e.type === "leave";
      })
      .map(function (e) {
        return e.channel;
      });
    presentProxies.forEach(function (_, name) {
      leaving.push(name);
    });
    if (leaving.length === 0) {
      return;
    }
    var body = new URLSearchParams();
    body.append("client_id", clientId);
    leaving.forEach(function (name) {
      body.append("leave_channels[]", name);
    });
    body.append("authenticity_token", csrfToken());
    navigator.sendBeacon(basePath() + "/presence/update", body);
  });

  onPresenceChange(
    Object.assign({}, DEFAULT_ACTIVE_OPTIONS, {
      callback: function () {
        if (loggedIn()) {
          throttledUpdateServer();
        }
      },
    })
  );

  Discourse.presence = {
    getChannel: function (name) {
      return new Channel(name);
    },
  };
})();
