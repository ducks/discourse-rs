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
// with Chart.js. PollButtonsDropdown's close and open ask first and draw
// the widget again; its breakdown is PollBreakdownModal, the group pies
// from /polls/grouped_poll_results drawn with Chart.js and
// chartjs-plugin-datalabels. Not ported yet, and refused when clicked:
// export (data explorer).
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
  // I18n.t: the string, its %{name}s filled from vars.
  function t(key, vars) {
    if (!strings) {
      var el = document.getElementById("poll-strings");
      try {
        strings = el ? JSON.parse(el.textContent) : {};
      } catch (e) {
        strings = {};
      }
    }
    var s = strings[key] === undefined ? key : strings[key];
    Object.keys(vars || {}).forEach(function (name) {
      s = s.split("%{" + name + "}").join(vars[name]);
    });
    return s;
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
      if (breakdown && breakdown.key === key && breakdown.quoted === !!state.quoted) {
        breakdown.update(stateOf(widget).breakdown);
      }
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

  // A script loaded once, resolving with what it defines (defined()).
  var loading = {};
  function loadScript(src, defined) {
    if (defined()) {
      return Promise.resolve(defined());
    }
    if (!loading[src]) {
      loading[src] = new Promise(function (resolve, reject) {
        var script = document.createElement("script");
        script.src = src;
        script.onload = function () {
          resolve(defined());
        };
        script.onerror = reject;
        document.head.appendChild(script);
      });
    }
    return loading[src];
  }

  var scriptData = document.currentScript ? document.currentScript.dataset : {};

  // loadChartJS: Chart.js, loaded once on the first pie.
  function loadChartJS() {
    return loadScript(scriptData.chartJs, function () {
      return window.Chart;
    });
  }

  // loadChartJSDatalabels: chartjs-plugin-datalabels, whose UMD build
  // reads window.Chart.
  function loadChartJSDatalabels() {
    return loadChartJS().then(function () {
      return loadScript(scriptData.chartDatalabels, function () {
        return window.ChartDataLabels;
      });
    });
  }

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

  // toggleStatus: closes or opens the poll once confirmed; the results
  // show while an on_close or always poll is closed. Ember refreshes the
  // route after closing an on_close poll to get its votes: the widget
  // drawn again from the server has them.
  function toggleStatus(outer) {
    var state = stateOf(outer);
    if (state.automatically_closed || !Discourse.dialog) {
      return;
    }
    Discourse.dialog.yesNoConfirm({
      message: t(state.closed ? "poll.open.confirm" : "poll.close.confirm"),
      didConfirm: function () {
        var current = stateOf(findWidget(state) || outer);
        var key = keyOf(current.post_id, current.name);
        ajax("PUT", "/polls/toggle_status", {
          post_id: current.post_id,
          poll_name: current.name,
          status: current.closed ? "open" : "closed",
        }).then(
          function (result) {
            var poll = (result && result.poll) || {};
            var changes = {};
            if (poll.results === "on_close" || poll.results === "always") {
              var results = poll.status === "closed";
              changes.results = results;
              changes.toggle = results;
              keptFor(key).toggle = results;
            }
            var widget = findWidget(current) || outer;
            redraw(widget, uiState(stateOf(widget), changes));
          },
          function (error) {
            popupAjaxError(error, "poll.error_while_toggling_status");
          }
        );
      },
    });
  }

  // I18n.toNumber(number, { precision }) with the locale's number.format.
  function toNumber(number, precision) {
    // prepareOptions' defaults where the locale has none (the server
    // sends a missing string as its full key).
    var format = function (key, fallback) {
      var value = t(key);
      return value === key || value === "js." + key ? fallback : value;
    };
    var separator = format("number.format.separator", ".");
    var delimiter = format("number.format.delimiter", ",");
    var negative = number < 0;
    var parts = Math.abs(number).toFixed(precision).toString().split(".");
    var whole = parts[0];
    var buffer = [];
    while (whole.length > 0) {
      var pos = Math.max(0, whole.length - 3);
      buffer.unshift(whole.slice(pos, pos + 3));
      whole = whole.slice(0, -3);
    }
    var formatted = buffer.join(delimiter);
    if (precision > 0) {
      formatted += separator + parts[1];
    }
    return negative ? "-" + formatted : formatted;
  }

  // @ember/string's classify.
  function classify(str) {
    return str
      .split("/")
      .map(function (part) {
        return part
          .replace(/^(\-|_)+(.)?/, function (match, separator, chr) {
            return chr ? "_" + chr.toUpperCase() : "";
          })
          .replace(/(.)(\-|\_|\.|\s)+(.)?/g, function (match, initialChar, separator, chr) {
            return initialChar + (chr ? chr.toUpperCase() : "");
          });
      })
      .join("/")
      .replace(/(^|\/|\.)([a-z])/g, function (match) {
        return match.toUpperCase();
      });
  }

  // PollBreakdownModal's groupableUserFields.
  function groupableUserFields(fields) {
    return fields.map(function (field) {
      var transformed = field.split("_").filter(Boolean);
      if (transformed.length > 1) {
        transformed[0] = classify(transformed[0]);
      }
      return { id: field, label: transformed.join(" ") };
    });
  }

  // The ComboBox poll-builder.js draws, loaded with it.
  function loadComboBox() {
    return loadScript(scriptData.pollBuilder, function () {
      return Discourse.pollBuilder && Discourse.pollBuilder.comboBox;
    });
  }

  // The breakdown modal open now: its poll's key, and how it follows the
  // poll as the widget is drawn again (Ember's modal holds the tracked
  // poll object).
  var breakdown = null;

  // PollBreakdownChart: a group's pie over the options with votes, the
  // option under the pointer highlighted in the modal.
  function breakdownChart(m, group, options) {
    var container = document.createElement("div");
    container.className = "poll-breakdown-chart-container";
    var label = document.createElement("label");
    label.className = "poll-breakdown-chart-label";
    label.textContent = group;
    var canvas = document.createElement("canvas");
    canvas.className = "poll-breakdown-chart-chart";
    container.appendChild(label);
    container.appendChild(canvas);
    var c = {
      element: container,
      options: options,
      optionToSlice: {},
      previousDisplayMode: null,
      chart: null,
      destroyed: false,
    };

    c.config = function () {
      var data = (c.options && c.options.map ? c.options.map(function (item) {
        return item.votes;
      }) : null) || [];
      var displayMode = m.displayMode;
      var transformedData = [];
      var counter = 0;
      c.optionToSlice = {};
      data.forEach(function (votes, index) {
        if (votes > 0) {
          transformedData.push(votes);
          c.optionToSlice[index] = counter++;
        }
      });
      var totalVotes = transformedData.reduce(function (sum, votes) {
        return sum + votes;
      }, 0);
      var colors = getColors(data.length).filter(function (color, index) {
        return data[index] > 0;
      });
      return {
        type: "pie",
        data: {
          datasets: [{ data: transformedData, backgroundColor: colors, hoverBorderColor: "#fff" }],
        },
        options: {
          plugins: {
            tooltip: false,
            datalabels: {
              color: "#333",
              backgroundColor: "rgba(255, 255, 255, 0.5)",
              borderRadius: 2,
              font: { family: getComputedStyle(document.body).fontFamily, size: 16 },
              padding: { top: 2, right: 6, bottom: 2, left: 6 },
              formatter: function (votes) {
                if (displayMode !== "percentage") {
                  return votes;
                }
                return toNumber((votes / totalVotes) * 100.0, 1) + "%";
              },
            },
          },
          responsive: true,
          aspectRatio: 1.1,
          animation: { duration: 0 },
          // Ember's next(): hover sends many events.
          onHover: function (event, activeElements) {
            if (!activeElements.length) {
              setTimeout(function () {
                m.setHighlighted(null);
              });
              return;
            }
            var sliceIndex = activeElements[0].index;
            var optionIndex = Object.keys(c.optionToSlice).find(function (option) {
              return c.optionToSlice[option] === sliceIndex;
            });
            setTimeout(function () {
              m.setHighlighted(Number(optionIndex));
            });
          },
        },
      };
    };

    // didReceiveAttrs: _updateDisplayMode, then _updateHighlight, which
    // Ember reaches only while no option is highlighted (setHighlighted),
    // so it never makes a slice active.
    c.update = function () {
      if (!c.chart) {
        return;
      }
      if (m.displayMode !== c.previousDisplayMode) {
        var config = c.config();
        c.chart.data.datasets = config.data.datasets;
        c.chart.options = config.options;
        c.chart.update();
        c.previousDisplayMode = m.displayMode;
      }
      c.chart.setActiveElements([]);
      c.chart.update();
    };

    c.destroy = function () {
      c.destroyed = true;
      if (c.chart) {
        c.chart.destroy();
      }
    };

    Promise.all([loadChartJS(), loadChartJSDatalabels()]).then(function (loaded) {
      if (c.destroyed) {
        return;
      }
      var config = c.config();
      config.plugins = [loaded[1]];
      c.chart = new loaded[0](canvas.getContext("2d"), config);
    });
    return c;
  }

  // PollBreakdownModal over the poll's state: the options with their
  // share or count beside each group's pie from
  // /polls/grouped_poll_results, grouped by the user field chosen.
  function showBreakdown(outer) {
    var state = stateOf(outer);
    if (!state.breakdown || !Discourse.modal) {
      return;
    }
    var fields = groupableUserFields(state.breakdown.fields || []);
    var m = {
      model: state.breakdown,
      displayMode: "percentage",
      highlighted: null,
      groupedBy: fields[0] && fields[0].id,
      charts: [],
      destroyed: false,
    };
    var tabs, sidebar, chartsEl;

    function drawTabs() {
      tabs.querySelectorAll(".modal-tab").forEach(function (tab) {
        tab.classList.toggle("is-active", tab.classList.contains(m.displayMode));
      });
    }

    function totalVotes() {
      return (m.model.options || []).reduce(function (sum, option) {
        return sum + option.votes;
      }, 0);
    }

    // PollBreakdownOption: its highlight, swatch and share or count. A
    // highlighted option's swatch would take Chart.helpers.getHoverColor,
    // but the Chart loadChartJS gives Ember is chart.js/auto's, which has
    // no helpers: drawing it throws, after the row's background and before
    // the swatch, and the render stops there. The swatch keeps its color.
    function drawOption(li, option, index, colors, total) {
      if (m.highlighted === index) {
        li.setAttribute("style", "background: rgba(0, 0, 0, 0.1);");
      } else {
        li.removeAttribute("style");
      }
      li.querySelector(".poll-breakdown-option-color").setAttribute("style", "background: " + colors[index] + ";");
      li.querySelector(".poll-breakdown-option-count").textContent =
        m.displayMode === "percentage"
          ? t("number.percent", { count: toNumber((option.votes / total) * 100.0, 1) })
          : String(option.votes);
    }

    function drawOptions() {
      var options = m.model.options || [];
      var colors = getColors(options.length);
      var total = totalVotes();
      sidebar.querySelectorAll(".poll-breakdown-option").forEach(function (li, index) {
        drawOption(li, options[index], index, colors, total);
      });
    }

    // The sidebar from the model: drawn again as the poll changes.
    function drawSidebar() {
      sidebar.innerHTML =
        '<p class="poll-breakdown-title"></p><div class="poll-breakdown-total-votes"></div><ul class="poll-breakdown-options"></ul>';
      sidebar.querySelector(".poll-breakdown-title").innerHTML = m.model.title || "";
      sidebar.querySelector(".poll-breakdown-total-votes").textContent = m.model.votes || "";
      var list = sidebar.querySelector(".poll-breakdown-options");
      (m.model.options || []).forEach(function (option, index) {
        var li = document.createElement("li");
        li.className = "poll-breakdown-option";
        li.setAttribute("role", "button");
        li.innerHTML =
          '<span class="poll-breakdown-option-color"></span><span class="poll-breakdown-option-count"></span><span class="poll-breakdown-option-text"></span>';
        // decoratePollOption: the HTML, run through the cooked decorators.
        var text = li.querySelector(".poll-breakdown-option-text");
        if (option.html) {
          text.innerHTML = option.html;
          if (Discourse.decorateCooked && Discourse.decorateCooked.apply) {
            Discourse.decorateCooked.apply(text);
          }
        }
        li.addEventListener("mouseover", function () {
          m.setHighlighted(index);
        });
        li.addEventListener("mouseout", function () {
          m.setHighlighted(null);
        });
        list.appendChild(li);
      });
      drawOptions();
    }

    // highlightedOption, set by hovering an option or a slice. With an
    // option highlighted Ember's render throws in its PollBreakdownOption
    // (drawOption) before it reaches the pies, which are left as they
    // are. Ember renders nothing more after that throw; the port keeps
    // drawing, so the row's highlight goes with the pointer.
    m.setHighlighted = function (value) {
      if (m.destroyed || m.highlighted === value) {
        return;
      }
      m.highlighted = value;
      drawOptions();
      var options = m.model.options || [];
      var optionHighlighted = typeof value === "number" && value >= 0 && value < options.length;
      if (!optionHighlighted) {
        m.charts.forEach(function (c) {
          c.update();
        });
      }
    };

    function setDisplayMode(mode) {
      if (m.displayMode === mode) {
        return;
      }
      m.displayMode = mode;
      drawTabs();
      drawOptions();
      m.charts.forEach(function (c) {
        c.update();
      });
    }

    function setCharts(groups) {
      m.charts.forEach(function (c) {
        c.destroy();
      });
      chartsEl.innerHTML = "";
      m.charts = (groups || []).map(function (chart) {
        var c = breakdownChart(m, chart.group, chart.options);
        chartsEl.appendChild(c.element);
        return c;
      });
    }

    function fetchGroupedPollData() {
      ajax("GET", "/polls/grouped_poll_results.json", {
        post_id: state.post_id,
        poll_name: state.name,
        user_field_name: m.groupedBy,
      }).then(
        function (result) {
          if (!m.destroyed) {
            setCharts(result.grouped_results);
          }
        },
        function (error) {
          popupAjaxError(error, "poll.error_while_fetching_voters");
        }
      );
    }

    var modalEl = null;
    Discourse.modal.show({
      className: "poll-breakdown has-tabs",
      title: t("poll.breakdown.title"),
      headerBelowTitle: function () {
        tabs = document.createElement("ul");
        tabs.className = "modal-tabs";
        ["percentage", "count"].forEach(function (mode) {
          var li = document.createElement("li");
          li.className = "modal-tab " + mode;
          li.textContent = t("poll.breakdown." + mode);
          li.addEventListener("click", function () {
            setDisplayMode(mode);
          });
          tabs.appendChild(li);
        });
        drawTabs();
        return tabs;
      },
      body: function (el) {
        modalEl = el.closest(".d-modal");
        el.innerHTML =
          '<div class="poll-breakdown-sidebar"></div><div class="poll-breakdown-body"><div class="poll-breakdown-body-header"><label class="poll-breakdown-body-header-label"></label></div><div class="poll-breakdown-charts"></div></div>';
        sidebar = el.querySelector(".poll-breakdown-sidebar");
        chartsEl = el.querySelector(".poll-breakdown-charts");
        el.querySelector(".poll-breakdown-body-header-label").textContent = t("poll.breakdown.breakdown");
        drawSidebar();
        var header = el.querySelector(".poll-breakdown-body-header");
        loadComboBox().then(function (comboBox) {
          if (m.destroyed) {
            return;
          }
          header.appendChild(
            comboBox(t, {
              className: "poll-breakdown-dropdown",
              content: fields.map(function (f) {
                return { id: f.id, name: f.label };
              }),
              value: m.groupedBy,
              onChange: function (value) {
                m.groupedBy = value;
                fetchGroupedPollData();
              },
            })
          );
        });
      },
      onClose: function () {
        m.destroyed = true;
        m.charts.forEach(function (c) {
          c.destroy();
        });
        if (breakdown && breakdown.m === m) {
          breakdown = null;
        }
      },
    });
    // A click outside an open select-kit closes it (the select-kit
    // service's clickOutside).
    if (modalEl) {
      modalEl.addEventListener("click", function (event) {
        if (!event.target.isConnected) {
          return;
        }
        modalEl.querySelectorAll("details.select-kit[open]").forEach(function (kit) {
          if (!kit.contains(event.target)) {
            kit.open = false;
          }
        });
      });
    }
    breakdown = {
      m: m,
      key: keyOf(state.post_id, state.name),
      quoted: !!state.quoted,
      update: function (model) {
        if (model && !m.destroyed) {
          m.model = model;
          drawSidebar();
        }
      },
    };
    fetchGroupedPollData();
  }

  // PollButtonsDropdown's dropDownClick, from the gear menu or its one
  // button. Export waits for data explorer.
  function dropDownClick(outer, action, menu) {
    if (action === "toggleDisplayMode") {
      toggleDisplayMode(outer, menu);
    } else if (action === "toggleStatus") {
      toggleStatus(outer);
    } else if (action === "showBreakdown") {
      showBreakdown(outer);
    } else if (Discourse.dialog) {
      Discourse.dialog.alert("Not ported yet: the poll's export.");
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
