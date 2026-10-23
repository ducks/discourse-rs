// discourse-poll's widget in the browser (components/poll.gjs and what it
// renders): what its clicks do. The server draws the widget, on the page
// (crate::plugins::poll::view) and again in the state a click brings it to
// (GET /live/poll/:post_id/:poll_name?state=<UiState as JSON>), and this
// script swaps the drawing in. Each widget's data-poll-state is the state
// it was drawn in.
//
// What Ember keeps on the poll object outlives the component (the vote in
// progress, the results toggle, amending): kept here by post and poll, a
// post drawn anew (an edit, a like over the live stream) gets its polls
// drawn again in it. The component's own (results shown) lasts as long
// as the widget.
//
// Live updates: the page's stream (/live) carries `poll` events when a
// post's polls change on /polls/<topic_id>; each of the post's widgets is
// drawn again for the viewer. Polls in an expanded quote are drawn for the
// quoted post. Anonymous readers are sent to log in (deferAnonymousAction's
// vote_poll is not ported). Not ported yet, and refused when clicked: the
// actions behind PollButtonsDropdown (the tally toggle, the breakdown,
// export, close and open).
(function () {
  "use strict";

  var Discourse = (window.Discourse = window.Discourse || {});
  if (!Discourse.once("poll")) {
    return;
  }

  // poll.inProgressVote, showResultsToggle and amendingVoteToggle, by
  // "<post id>:<poll name>".
  var kept = {};
  // The latest redraw asked for each widget: older answers are dropped.
  var requests = {};
  // Widgets with a redraw on its way, and those of them whose poll changed
  // meanwhile (a /polls message), to draw again once it is in.
  var inflight = {};
  var stale = {};
  var casting = {};

  function basePath() {
    var meta = document.querySelector('meta[name="discourse-base-uri"]');
    return meta ? meta.content : "";
  }

  var strings = null;
  function t(key) {
    if (!strings) {
      var el = document.getElementById("poll-strings");
      try {
        strings = el ? JSON.parse(el.textContent) : {};
      } catch (e) {
        strings = {};
      }
    }
    return strings[key] === undefined ? key : strings[key];
  }

  function csrfHeaders() {
    var headers = {};
    try {
      headers = JSON.parse(document.body.getAttribute("hx-headers") || "{}");
    } catch (e) {
      // No token: the server refuses the request and says so.
    }
    headers["X-Requested-With"] = "XMLHttpRequest";
    headers.Accept = "application/json";
    return headers;
  }

  // jQuery.param, as Ember's ajax sends data: arrays of scalars as a[]=,
  // objects and arrays of objects as a[0][k]=.
  function param(data) {
    var parts = [];
    function add(key, value) {
      if (Array.isArray(value)) {
        value.forEach(function (v, i) {
          add(key + "[" + (v !== null && typeof v === "object" ? i : "") + "]", v);
        });
      } else if (value !== null && typeof value === "object") {
        Object.keys(value).forEach(function (k) {
          add(key + "[" + k + "]", value[k]);
        });
      } else {
        parts.push(encodeURIComponent(key) + "=" + encodeURIComponent(value == null ? "" : value));
      }
    }
    Object.keys(data).forEach(function (k) {
      add(k, data[k]);
    });
    return parts.join("&").replace(/%20/g, "+");
  }

  // ajax(): the JSON answer, or an Error carrying what popupAjaxError
  // shows (extractError: the errors, else the generic error).
  function ajax(method, path, data) {
    var url = basePath() + path;
    var options = { method: method, credentials: "same-origin", headers: csrfHeaders() };
    if (method === "GET") {
      if (data) {
        url += "?" + param(data);
      }
    } else if (data) {
      options.headers["Content-Type"] = "application/x-www-form-urlencoded; charset=UTF-8";
      options.body = param(data);
    }
    return fetch(url, options).then(function (response) {
      return response.text().then(function (text) {
        var json = null;
        try {
          json = text ? JSON.parse(text) : null;
        } catch (e) {
          // Not JSON: the status decides.
        }
        if (!response.ok) {
          var error = new Error(
            (json && json.errors && [].concat(json.errors).join("\n")) ||
              (json && json.error) ||
              t("generic_error")
          );
          error.status = response.status;
          throw error;
        }
        return json;
      });
    });
  }

  function popupAjaxError(error, fallback) {
    if (!Discourse.dialog) {
      return;
    }
    if (error && error.message) {
      Discourse.dialog.popupAjaxError(error.message);
    } else {
      Discourse.dialog.alert(t(fallback));
    }
  }

  function stateOf(outer) {
    try {
      return JSON.parse(outer.dataset.pollState || "{}");
    } catch (e) {
      return {};
    }
  }

  function keyOf(postId, name) {
    return postId + ":" + name;
  }

  function keptFor(key) {
    return (kept[key] = kept[key] || {});
  }

  // The UiState to draw the widget in: its component's, with changes.
  function uiState(state, changes) {
    var k = kept[keyOf(state.post_id, state.name)] || {};
    var ui = {
      results: state.results,
      amending: !!state.amending,
    };
    if (k.toggle !== undefined) {
      ui.toggle = k.toggle;
    }
    if (k.vote !== undefined) {
      ui.vote = k.vote;
    }
    if (state.quoted) {
      ui.quoted = true;
    }
    Object.keys(changes || {}).forEach(function (name) {
      if (changes[name] === undefined) {
        delete ui[name];
      } else {
        ui[name] = changes[name];
      }
    });
    return ui;
  }

  // What a new component starts from: what the poll object kept.
  function initialState(key) {
    var k = kept[key];
    if (!k || (k.toggle === undefined && !k.amending && k.vote === undefined)) {
      return null;
    }
    var ui = { amending: !!k.amending };
    if (k.toggle !== undefined) {
      ui.toggle = k.toggle;
    }
    if (k.vote !== undefined) {
      ui.vote = k.vote;
    }
    return ui;
  }

  function fetchWidget(postId, name, ui) {
    var url =
      basePath() +
      "/live/poll/" +
      encodeURIComponent(postId) +
      "/" +
      encodeURIComponent(name) +
      "?state=" +
      encodeURIComponent(JSON.stringify(ui));
    return fetch(url, { credentials: "same-origin" }).then(function (response) {
      if (!response.ok) {
        throw new Error(response.status + " " + response.statusText);
      }
      return response.text();
    });
  }

  function parseWidget(html) {
    var template = document.createElement("template");
    template.innerHTML = html.trim();
    return template.content.firstElementChild;
  }

  // preserveButtonsPosition: the buttons stay where they were on screen.
  function buttonsTop(outer) {
    var buttons = outer && outer.querySelector(".poll-buttons");
    return buttons ? buttons.getBoundingClientRect().top : null;
  }

  // Draws the widget again in `ui`; resolves with the new widget.
  function redraw(outer, ui, options) {
    options = options || {};
    var state = stateOf(outer);
    var key = keyOf(state.post_id, state.name);
    var request = (requests[key] = (requests[key] || 0) + 1);
    inflight[key] = true;
    var done = function (widget) {
      if (request !== requests[key]) {
        return;
      }
      delete inflight[key];
      if (stale[key]) {
        delete stale[key];
        if (widget) {
          redraw(widget, uiState(stateOf(widget)));
        }
      }
    };
    return fetchWidget(state.post_id, state.name, ui).then(function (html) {
      // A redraw that came in meanwhile replaced the widget: its successor.
      if (!outer.isConnected) {
        outer = findWidget(state) || outer;
      }
      if (request !== requests[key] || !outer.isConnected) {
        done(null);
        return null;
      }
      var widget = parseWidget(html);
      if (!widget) {
        return null;
      }
      var top = options.keepPosition ? buttonsTop(outer) : null;
      outer.replaceWith(widget);
      if (top !== null) {
        var shift = buttonsTop(widget) - top;
        if (shift) {
          window.scrollBy(0, shift);
        }
      }
      if (options.focus) {
        focusCurrentView(widget);
      }
      done(widget);
      return widget;
    }, function (error) {
      done(null);
      throw error;
    });
  }

  // The widget drawn now for the poll a state is of.
  function findWidget(state) {
    var found = null;
    document.querySelectorAll(".poll-outer[data-poll-state]").forEach(function (outer) {
      var other = stateOf(outer);
      if (
        !found &&
        String(other.post_id) === String(state.post_id) &&
        other.name === state.name &&
        !!other.quoted === !!state.quoted
      ) {
        found = outer;
      }
    });
    return found;
  }

  function focusCurrentView(outer) {
    var state = stateOf(outer);
    var target = outer.querySelector(state.show_voted_choices ? ".amend-vote" : "[data-poll-option-id] button");
    if (target) {
      target.focus();
    }
  }

  function showLogin() {
    location.href = basePath() + "/login";
  }

  function isSingle(state) {
    return state.type !== "multiple" && state.type !== "ranked_choice";
  }

  // toggleOption
  function toggleOption(outer, digest) {
    var state = stateOf(outer);
    if (state.closed) {
      return;
    }
    if (!state.member) {
      // Archived topics refuse votes; the rest log in first.
      if (state.archived) {
        return;
      }
      showLogin();
      return;
    }
    if (!state.in_group) {
      return;
    }
    var vote = (state.current_vote || []).slice();
    if (isSingle(state) && vote.length === 1 && vote[0] === digest) {
      return state.amending ? keepVote(outer) : removeVote(outer);
    }
    if (isSingle(state)) {
      chooseOption(outer, digest);
      castVotes(outer, [digest]);
      return;
    }
    // Multiple choice; ranked choice's options are not drawn.
    var index = vote.indexOf(digest);
    if (index === -1) {
      vote.push(digest);
    } else {
      vote.splice(index, 1);
    }
    keptFor(keyOf(state.post_id, state.name)).vote = vote;
    redraw(outer, uiState(state, { vote: vote }));
  }

  // The chosen option's icon at once, as Ember draws it before the vote
  // is cast.
  function chooseOption(outer, digest) {
    outer.querySelectorAll("li[data-poll-option-id] > button > svg").forEach(function (svg) {
      var chosen = svg.parentNode.parentNode.dataset.pollOptionId === digest;
      var name = chosen ? "circle" : "far-circle";
      svg.setAttribute("class", "fa d-icon d-icon-" + name + " svg-icon fa-width-auto svg-string");
      var use = svg.querySelector("use");
      if (use) {
        use.setAttribute("href", "#" + name);
      }
    });
  }

  // castVotes
  function castVotes(outer, vote) {
    var state = stateOf(outer);
    var key = keyOf(state.post_id, state.name);
    if (casting[key]) {
      return;
    }
    vote = vote || state.current_vote || [];
    casting[key] = true;
    var castButton = outer.querySelector(".cast-votes");
    if (castButton) {
      castButton.disabled = true;
    }
    ajax("PUT", "/polls/vote", {
      post_id: state.post_id,
      poll_name: state.name,
      options: vote,
    })
      .then(function () {
        var k = keptFor(key);
        delete k.vote;
        var changes = { vote: undefined, amending: false };
        // Results show once voted, unless they wait for the close; staff
        // only results only to staff.
        if (state.results_type !== "on_close") {
          var results = state.results_type === "staff_only" ? !!state.staff : true;
          changes.results = results;
          changes.toggle = results;
          k.toggle = results;
        }
        k.amending = false;
        return redraw(outer, uiState(state, changes), { focus: true });
      })
      .catch(function (error) {
        if (isSingle(state)) {
          keptFor(key).vote = vote;
          redraw(outer, uiState(state, { vote: vote }));
        }
        popupAjaxError(error, "poll.error_while_casting_votes");
      })
      .finally(function () {
        casting[key] = false;
      });
  }

  // removeVote
  function removeVote(outer) {
    var state = stateOf(outer);
    var key = keyOf(state.post_id, state.name);
    return ajax("DELETE", "/polls/vote", { post_id: state.post_id, poll_name: state.name })
      .then(function () {
        var k = keptFor(key);
        delete k.vote;
        k.toggle = false;
        k.amending = false;
        return redraw(
          outer,
          uiState(state, { vote: undefined, results: false, toggle: false, amending: false }),
          { focus: true }
        );
      })
      .catch(function (error) {
        popupAjaxError(error);
      });
  }

  function amendVote(outer) {
    var state = stateOf(outer);
    keptFor(keyOf(state.post_id, state.name)).amending = true;
    redraw(outer, uiState(state, { amending: true }), { keepPosition: true, focus: true });
  }

  function keepVote(outer) {
    var state = stateOf(outer);
    var k = keptFor(keyOf(state.post_id, state.name));
    delete k.vote;
    k.amending = false;
    return redraw(outer, uiState(state, { vote: undefined, amending: false }), {
      keepPosition: true,
      focus: true,
    });
  }

  function toggleResults(outer) {
    var state = stateOf(outer);
    var results = !state.results;
    keptFor(keyOf(state.post_id, state.name)).toggle = results;
    redraw(outer, uiState(state, { results: results, toggle: results }), { keepPosition: true });
  }

  // PollButtonsDropdown's actions are not ported: each says so.
  var unported = {
    toggleDisplayMode: "the poll's tally toggle",
    showBreakdown: "the poll's breakdown",
    exportResults: "the poll's export",
    toggleStatus: "closing and opening a poll",
  };

  function refuse(action) {
    var what = unported[action] || action;
    if (Discourse.dialog) {
      Discourse.dialog.alert("Not ported yet: " + what + ".");
    }
  }

  // A DMenu by its trigger: the content the server put on it.
  function openMenu(trigger, onClick) {
    if (!Discourse.float) {
      return;
    }
    if (trigger.getAttribute("aria-expanded") === "true") {
      Discourse.float.close(trigger.dataset.pollMenuId);
      return;
    }
    var holder = document.createElement("div");
    holder.innerHTML = trigger.dataset.pollMenu || "";
    var content = holder.firstElementChild;
    if (!content) {
      return;
    }
    var id = "poll-menu-" + Math.random().toString(36).slice(2);
    trigger.dataset.pollMenuId = id;
    trigger.setAttribute("aria-expanded", "true");
    trigger.classList.add("-expanded");
    var menu = Discourse.float.menu(trigger, {
      identifier: id,
      content: content,
      onClose: function () {
        trigger.setAttribute("aria-expanded", "false");
        trigger.classList.remove("-expanded");
      },
    });
    content.addEventListener("click", function (event) {
      var button = event.target.closest("button");
      if (!button) {
        return;
      }
      if (menu && menu.close) {
        menu.close();
      } else {
        Discourse.float.close(id);
      }
      onClick(button);
    });
  }

  document.addEventListener("click", function (event) {
    var target = event.target;
    var outer = target.closest && target.closest(".poll-outer[data-poll-state]");
    if (!outer) {
      return;
    }
    var button;
    if ((button = target.closest("li[data-poll-option-id] > button"))) {
      // PollOptions#sendClick: a local date inside the option opens its
      // own popup instead.
      if (target.closest(".discourse-local-date")) {
        return;
      }
      toggleOption(outer, button.parentNode.dataset.pollOptionId);
    } else if ((button = target.closest(".poll-buttons .cast-votes"))) {
      if (!button.disabled) {
        castVotes(outer);
      }
    } else if (target.closest(".poll-buttons .remove-vote")) {
      removeVote(outer);
    } else if (target.closest(".poll-buttons .amend-vote")) {
      amendVote(outer);
    } else if (target.closest(".poll-buttons .keep-vote")) {
      keepVote(outer);
    } else if (target.closest(".poll-buttons .toggle-results")) {
      toggleResults(outer);
    } else if ((button = target.closest(".poll-buttons-dropdown .widget-dropdown-header"))) {
      openMenu(button, function (item) {
        refuse(item.dataset.pollAction);
      });
    } else if ((button = target.closest(".poll-buttons-dropdown [data-poll-action]"))) {
      refuse(button.dataset.pollAction);
    }
  });

  // attachPolls: the server drew the post's polls; a post drawn anew gets
  // what the poll objects kept, and a cooked poll the server did not draw
  // (one in an expanded quote) is drawn for its post.
  function attach(el) {
    el.querySelectorAll(".poll-outer[data-poll-state]").forEach(function (outer) {
      var state = stateOf(outer);
      var ui = initialState(keyOf(state.post_id, state.name));
      if (ui) {
        redraw(outer, ui);
      }
    });
    var quote = el.closest(".expanded-quote[data-post-id]");
    if (!quote) {
      return;
    }
    el.querySelectorAll(".poll[data-poll-name]").forEach(function (node) {
      if (node.parentNode.tagName === "BLOCKQUOTE" || node.closest(".poll-outer")) {
        return;
      }
      var postId = quote.dataset.postId;
      var name = node.dataset.pollName;
      var ui = initialState(keyOf(postId, name)) || {};
      ui.quoted = true;
      fetchWidget(postId, name, ui)
        .then(function (html) {
          var widget = parseWidget(html);
          if (widget && node.isConnected) {
            node.replaceWith(widget);
          }
        })
        .catch(function () {
          // No such poll for the viewer: the cooked poll stays.
        });
    });
  }

  if (Discourse.decorateCooked) {
    Discourse.decorateCooked("poll", attach);
  }

  // The topic's /polls/<topic_id> messages over the page's stream.
  document.addEventListener("htmx:sseBeforeMessage", function (event) {
    var message = event.detail;
    if (!message || message.type !== "poll") {
      return;
    }
    event.preventDefault();
    var data = {};
    try {
      data = JSON.parse(message.data);
    } catch (e) {
      return;
    }
    document.querySelectorAll(".poll-outer[data-poll-state]").forEach(function (outer) {
      var state = stateOf(outer);
      // A quoted post's poll is not the topic's post: Ember leaves it.
      if (!state.quoted && String(state.post_id) === String(data.post_id)) {
        var key = keyOf(state.post_id, state.name);
        // A redraw on its way was asked in a state this one would undo.
        if (inflight[key]) {
          stale[key] = true;
        } else {
          redraw(outer, uiState(state));
        }
      }
    });
  });
})();
