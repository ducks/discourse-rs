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
// vote_poll is not ported). The results' tally toggle, tabs and voters'
// load more (over /polls/voters.json, merged as Ember merges them) are the
// component's own state, drawn by the server; the pie chart is drawn here
// with Chart.js. Not ported yet, and refused when clicked: the other
// actions behind PollButtonsDropdown (the breakdown, export, close and
// open).
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
    if (state.tally) {
      ui.tally = true;
    }
    if (state.tab) {
      ui.tab = state.tab;
    }
    if (state.expanded) {
      ui.voters = state.voters;
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
      drawPie(widget);
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
    // Multiple choice; ranked choice ranks through its dropdowns (chooseRank).
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

  // toggleDisplayMode: the tally or the percentages. Clicked in the gear
  // menu, the menu stays open as Ember leaves it, its content the other
  // mode's: opened again on the new widget's trigger.
  function toggleDisplayMode(outer, menu) {
    var state = stateOf(outer);
    redraw(outer, uiState(state, { tally: state.tally ? undefined : true })).then(function (widget) {
      if (!menu) {
        return;
      }
      // The old trigger is gone: the menu goes at once, its listeners
      // with it, as the instance's destroy would after its animation.
      (menu.listeners || []).forEach(function (fn) {
        fn();
      });
      menu.listeners = [];
      menu.expanded = false;
      if (menu.teardown) {
        menu.teardown();
      }
      var trigger = widget && widget.querySelector(".poll-buttons-dropdown .widget-dropdown-header");
      if (trigger) {
        openMenu(trigger, gearClick);
      }
    });
  }

  // PollResultsTabs' selectTab
  function selectTab(outer, tab) {
    redraw(outer, uiState(stateOf(outer), { tab: tab }));
  }

  // The ranked choice options' rank, chosen in its dropdown: toggleOption
  // and _toggleOption, every option given a rank in the vote.
  function chooseRank(outer, digest, rank) {
    var state = stateOf(outer);
    if (state.closed || !state.member || !state.in_group) {
      return;
    }
    var vote = (state.current_vote || []).map(function (v) {
      return { digest: v.digest, rank: v.rank };
    });
    (state.options || []).forEach(function (candidate) {
      var chosen = null;
      vote.forEach(function (v) {
        if (!chosen && v.digest === candidate) {
          chosen = v;
        }
      });
      if (!chosen) {
        vote.push({ digest: candidate, rank: candidate === digest ? rank : 0 });
      } else if (candidate === digest) {
        chosen.rank = rank;
      }
    });
    keptFor(keyOf(state.post_id, state.name)).vote = vote;
    redraw(outer, uiState(state, { vote: vote }));
  }

  var FETCH_VOTERS_COUNT = 25;

  // A voter as the widget's state keeps it.
  function compactVoter(voter, ranked) {
    return ranked
      ? [voter.user.username, voter.user.avatar_template, voter.rank]
      : [voter.username, voter.avatar_template];
  }

  // fetchVoters: the option's next page of voters, merged into the lists
  // as Ember merges them (a regular poll's voters dropped from the other
  // options), the spinner in place of the button meanwhile.
  function fetchVoters(outer, button, optionId) {
    var state = stateOf(outer);
    var ranked = state.type === "ranked_choice";
    var all = JSON.parse(JSON.stringify(state.voters || {}));
    var voters = (all[optionId] = all[optionId] || []);
    var expanded = !!state.expanded;
    var container = button.closest(".loading-container");
    if (container) {
      container.classList.add("visible");
      container.setAttribute("data-loading", "");
      container.innerHTML = '<div class="spinner "></div>';
    }
    ajax("GET", "/polls/voters.json", {
      post_id: state.post_id,
      poll_name: state.name,
      option_id: optionId,
      page: Math.floor(voters.length / FETCH_VOTERS_COUNT) + 1,
      limit: FETCH_VOTERS_COUNT,
    })
      .then(function (result) {
        expanded = true;
        var newVoters = result.voters[optionId];
        var seen = {};
        voters.forEach(function (voter) {
          seen[voter[0]] = true;
        });
        newVoters.forEach(function (voter) {
          var compact = compactVoter(voter, ranked);
          if (!seen[compact[0]]) {
            seen[compact[0]] = true;
            voters.push(compact);
          }
        });
        // remove users who changed their vote
        if (state.type === "regular") {
          Object.keys(all).forEach(function (otherOptionId) {
            if (otherOptionId !== optionId) {
              all[otherOptionId] = all[otherOptionId].filter(function (voter) {
                return !seen[voter[0]];
              });
            }
          });
        }
      })
      .catch(function (error) {
        popupAjaxError(error, "poll.error_while_fetching_voters");
      })
      .then(function () {
        var widget = findWidget(state) || outer;
        redraw(widget, uiState(stateOf(widget), { voters: expanded ? all : undefined }));
      });
  }

  // loadChartJS: Chart.js, loaded once on the first pie.
  var chartJs = null;
  function loadChartJS() {
    if (window.Chart) {
      return Promise.resolve(window.Chart);
    }
    if (!chartJs) {
      chartJs = new Promise(function (resolve, reject) {
        var script = document.createElement("script");
        script.src = chartSrc;
        script.onload = function () {
          resolve(window.Chart);
        };
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
    return chartJs;
  }
  var chartSrc = document.currentScript ? document.currentScript.dataset.chartJs : null;

  // lib/chart-colors.js
  function cssColor(name) {
    var value = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
    return value || null;
  }

  function gradientColors(count) {
    var gradient = {
      0: [255, 255, 255],
      25: [220, 237, 200],
      50: [66, 179, 213],
      75: [26, 39, 62],
      100: [0, 0, 0],
    };
    var keys = Object.keys(gradient);
    var colors = [];
    var previous;
    for (var colorIndex = 0; colorIndex < count; colorIndex++) {
      var current = (colorIndex + 1) * (100 / (count + 1));
      previous = previous || 0;
      var base;
      for (var y = previous; y < keys.length; y++) {
        if (!keys[y + 1]) {
          base = y - 1;
          break;
        } else if (current >= keys[y] && current < keys[y + 1]) {
          base = y;
          break;
        }
      }
      var multiplier = (current - keys[base]) / (keys[base + 1] - keys[base]);
      var color = [];
      for (var k = 0; k < 3; k++) {
        color.push(
          Math.round(
            gradient[keys[base]][k] -
              (gradient[keys[base]][k] - gradient[keys[base + 1]][k]) * multiplier
          )
        );
      }
      colors.push("rgb(" + color.toString() + ")");
      previous = base;
    }
    return colors;
  }

  function getColors(count) {
    var colors = [];
    for (var i = 1; i <= count; i++) {
      var color = cssColor("--poll-pie-color-" + i);
      if (color) {
        colors.push(color);
      }
    }
    if (colors.length >= count) {
      return colors.slice(0, count);
    }
    return colors.concat(gradientColors(count - colors.length));
  }

  // chart-legend-icon's buildLegendIcon
  function buildLegendIcon(color, isVisible) {
    var size = parseFloat(getComputedStyle(document.documentElement).fontSize);
    var canvas = document.createElement("canvas");
    canvas.width = size;
    canvas.height = size;
    var ctx = canvas.getContext("2d");
    ctx.globalAlpha = isVisible ? 1 : 0.6;
    ctx.strokeStyle = color;
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.roundRect(1, 1, size - 2, size - 2, 4);
    ctx.stroke();
    if (isVisible) {
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.roundRect(4, 4, size - 8, size - 8, 2);
      ctx.fill();
    }
    return canvas;
  }

  function stripHtml(html) {
    var doc = new DOMParser().parseFromString(html, "text/html");
    return doc.body.textContent || "";
  }

  var htmlLegendPlugin = {
    id: "htmlLegend",
    afterUpdate: function (chart, args, options) {
      var ul = document.getElementById(options.containerID);
      if (!ul) {
        return;
      }
      ul.innerHTML = "";
      var items = chart.options.plugins.legend.labels.generateLabels(chart);
      items.forEach(function (item) {
        var isVisible = chart.getDataVisibility(item.index);
        var li = document.createElement("li");
        li.classList.add("legend");
        li.style.opacity = isVisible ? 1.0 : 0.4;
        li.onclick = function () {
          chart.toggleDataVisibility(item.index);
          chart.update();
        };
        var icon = buildLegendIcon(item.fillStyle, isVisible);
        icon.classList.add("swatch");
        var text = document.createElement("span");
        text.style.color = item.fontColor;
        text.innerHTML = item.text;
        li.appendChild(icon);
        li.appendChild(text);
        ul.appendChild(li);
      });
    },
  };

  // PollResultsPie's drawPie, for a widget showing its pie.
  function drawPie(outer) {
    var pie = stateOf(outer).pie;
    var canvas = outer.querySelector(".poll-results-canvas");
    var legend = outer.querySelector(".pie-chart-legends");
    if (!pie || !canvas || !legend) {
      return;
    }
    loadChartJS().then(function (Chart) {
      if (!canvas.isConnected) {
        return;
      }
      var data = pie.data;
      var labels = pie.labels;
      new Chart(canvas.getContext("2d"), {
        type: "pie",
        data: {
          datasets: [{ data: data, backgroundColor: getColors(data.length) }],
          labels: labels.map(stripHtml),
        },
        plugins: [htmlLegendPlugin],
        options: {
          responsive: true,
          aspectRatio: 2.2,
          animation: { duration: 0 },
          plugins: {
            legend: {
              labels: {
                generateLabels: function () {
                  return labels.map(function (text, index) {
                    return { fillStyle: getColors(data.length)[index], text: text, index: index };
                  });
                },
              },
              display: false,
            },
            htmlLegend: { containerID: legend.id },
          },
        },
      });
      if (Discourse.decorateCooked && Discourse.decorateCooked.apply) {
        Discourse.decorateCooked.apply(legend);
      }
    });
  }

  // PollButtonsDropdown's other actions are not ported: each says so.
  var unported = {
    showBreakdown: "the poll's breakdown",
    exportResults: "the poll's export",
    toggleStatus: "closing and opening a poll",
  };

  // PollButtonsDropdown's dropDownClick, from the gear menu or its one
  // button.
  function dropDownClick(outer, action, menu) {
    if (action === "toggleDisplayMode") {
      toggleDisplayMode(outer, menu);
      return;
    }
    var what = unported[action] || action;
    if (Discourse.dialog) {
      Discourse.dialog.alert("Not ported yet: " + what + ".");
    }
  }

  // The gear menu's items. The tally toggle leaves the menu open.
  function gearClick(item, menu) {
    var outer = menu && menu.trigger && menu.trigger.closest(".poll-outer[data-poll-state]");
    if (outer) {
      dropDownClick(outer, item.dataset.pollAction, menu);
    }
  }
  gearClick.keepOpen = function (item) {
    return item.dataset.pollAction === "toggleDisplayMode";
  };

  // A DMenu by its trigger: the content the server put on it. A click in
  // it closes it, unless onClick.keepOpen(button) says to keep it open.
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
      if (onClick.keepOpen && onClick.keepOpen(button)) {
        onClick(button, menu);
        return;
      }
      if (menu && menu.close) {
        menu.close();
      } else {
        Discourse.float.close(id);
      }
      onClick(button, menu);
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
      openMenu(button, gearClick);
    } else if ((button = target.closest(".poll-buttons-dropdown [data-poll-action]"))) {
      dropDownClick(outer, button.dataset.pollAction);
    } else if ((button = target.closest(".tab-container [data-poll-tab]"))) {
      selectTab(outer, button.dataset.pollTab);
    } else if ((button = target.closest(".poll-voters-toggle-expand[data-poll-voters]"))) {
      fetchVoters(outer, button, button.dataset.pollVoters);
    } else if ((button = target.closest(".ranked-choice-poll-option [data-poll-menu]"))) {
      var digest = button.closest("[data-poll-option-id]").dataset.pollOptionId;
      openMenu(button, function (item) {
        var rank = parseInt(item.dataset.pollRank, 10);
        chooseRank(findWidget(stateOf(outer)) || outer, digest, rank);
      });
    } else if (target.closest(".ranked-choice-poll-option [data-poll-login]")) {
      showLogin();
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
      } else {
        drawPie(outer);
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
            drawPie(widget);
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
